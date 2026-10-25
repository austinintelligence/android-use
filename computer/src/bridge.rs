use crate::api::{Code, Error, Node, Plan, Range, Receipt, Result, Scene, MAX_FRAME, MAX_INLINE};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
};

/// Largest artifact slice requested per helper round trip. Base64 inflates it
/// to ~350 KiB, comfortably inside the 1 MiB frame. Helpers older than 1.1
/// reject anything above `MAX_INLINE`; `download` then falls back to that.
pub const MAX_CHUNK: usize = 256 * 1024;

/// One framed request/reply channel to the helper. `TcpStream` is the real
/// one (length-prefixed JSON through an adb forward); `FakeLink` scripts it.
pub trait Wire {
    fn send(&mut self, value: &Value) -> Result<()>;
    fn recv(&mut self) -> Result<Value>;
    fn set_read_timeout(&self, timeout: Duration) -> Result<()>;
}

impl Wire for TcpStream {
    fn send(&mut self, v: &Value) -> Result<()> {
        send(self, v)
    }
    fn recv(&mut self) -> Result<Value> {
        recv(self)
    }
    fn set_read_timeout(&self, timeout: Duration) -> Result<()> {
        Ok(TcpStream::set_read_timeout(self, Some(timeout))?)
    }
}

pub struct Bridge {
    wire: Box<dyn Wire>,
    seq: u64,
    chunk: usize,
}

pub enum Observation {
    Unchanged(u64),
    Scene(Scene),
}

impl Bridge {
    pub fn new(wire: Box<dyn Wire>) -> Self {
        Self { wire, seq: 0, chunk: MAX_CHUNK }
    }

    fn call(&mut self, make: impl FnOnce(u64) -> Value) -> Result<Vec<Value>> {
        self.seq = self.seq.checked_add(1).ok_or_else(|| Error::new(Code::Sequence, "sequence exhausted"))?;
        self.wire.send(&make(self.seq))?;
        let v = self.wire.recv()?;
        let a = v.as_array().ok_or_else(|| Error::new(Code::Protocol, "helper reply must be an array"))?;
        if a.first().and_then(Value::as_u64) != Some(self.seq) {
            return Err(Error::new(Code::Sequence, "helper reply sequence mismatch"));
        }
        Ok(a.clone())
    }
    /// Bounds each later wait for a helper reply, e.g. by a plan deadline.
    pub fn set_read_timeout(&self, timeout: Duration) -> Result<()> {
        self.wire.set_read_timeout(timeout)
    }
    pub fn status(&mut self) -> Result<(u64, u32)> {
        let a = self.call(|s| json!([s, "status"]))?;
        ok(&a)?;
        if a.len() != 4 {
            return Err(Error::new(Code::Protocol, "invalid status reply"));
        }
        Ok((u64v(&a[2])?, u32::try_from(u64v(&a[3])?).map_err(|_| Error::new(Code::Protocol, "capability mask overflow"))?))
    }
    /// Every window and its nodes; `capabilities` is the mask from `status`.
    pub fn tree(&mut self, capabilities: u32) -> Result<crate::tree::Tree> {
        let a = self.call(|s| json!([s, "tree"]))?;
        ok(&a)?;
        crate::tree::parse(&a, capabilities)
    }
    pub fn observe(&mut self, base: Option<&str>, detail: u8) -> Result<Observation> {
        let a = self.call(|s| json!([s, "observe", base, detail]))?;
        ok(&a)?;
        if a.len() == 3 {
            return Ok(Observation::Unchanged(u64v(&a[2])?));
        }
        if a.len() != 5 {
            return Err(Error::new(Code::Protocol, "invalid observe reply"));
        }
        let g = u64v(&a[2])?;
        let package_text = text(&a[3])?;
        if package_text.len() > 255 {
            return Err(Error::new(Code::Bounds, "helper package name exceeds limit"));
        }
        let package = package_text.into();
        let rows = a[4].as_array().ok_or_else(|| Error::new(Code::Protocol, "scene nodes must be an array"))?;
        if rows.len() > 256 {
            return Err(Error::new(Code::Bounds, "helper scene exceeds 256 nodes"));
        }
        let mut nodes = Vec::with_capacity(rows.len());
        for row in rows {
            let r = row.as_array().ok_or_else(|| Error::new(Code::Protocol, "node must be an array"))?;
            if r.len() != 4 {
                return Err(Error::new(Code::Protocol, "node must contain four values"));
            }
            let label = text(&r[1])?;
            if label.len() > 1024 {
                return Err(Error::new(Code::Bounds, "helper node label exceeds limit"));
            }
            let role = u8::try_from(u64v(&r[2])?).map_err(|_| Error::new(Code::Bounds, "node role overflow"))?;
            if !b"bitcsmu".contains(&role) {
                return Err(Error::new(Code::Protocol, "unknown node role"));
            }
            let flags = u8::try_from(u64v(&r[3])?).map_err(|_| Error::new(Code::Bounds, "node flags overflow"))?;
            if flags > 15 {
                return Err(Error::new(Code::Protocol, "unknown node flags"));
            }
            nodes.push(Node { id: u16::try_from(u64v(&r[0])?).map_err(|_| Error::new(Code::Bounds, "node ref overflow"))?, label: label.into(), role, flags });
        }
        Ok(Observation::Scene(Scene { observation: g.to_string().into(), generation: g, package, nodes: nodes.into_boxed_slice() }))
    }
    pub fn act(&mut self, p: &Plan) -> Result<Receipt> {
        let a = self.call(|s| p.wire(s))?;
        if a.get(1).and_then(Value::as_u64) == Some(11) {
            return Err(crate::link::stopped());
        }
        if a.get(1).and_then(Value::as_u64) == Some(12) {
            return Err(Error::plain(Code::Denied, "The phone is waiting for a person to answer an approval, so nothing runs until they do. Nothing was run."));
        }
        if a.len() < 4 {
            return Err(Error::new(Code::Protocol, "invalid run reply"));
        }
        let code = u64v(&a[1])?;
        let g = u64v(&a[2])?;
        let m = u8::try_from(u64v(&a[3])?).map_err(|_| Error::new(Code::Protocol, "mutation count overflow"))?;
        let at = a.get(4).and_then(Value::as_u64).map(u8::try_from).transpose().map_err(|_| Error::new(Code::Protocol, "operation index overflow"))?;
        let artifact = a.get(5).and_then(Value::as_str).map(Into::into);
        let error = match code {
            0 => None,
            1 => Some("stale"),
            2 => Some("timeout"),
            3 => Some("partial"),
            4 => Some("ambiguous"),
            5 => Some("unsupported"),
            6 => Some("bounds"),
            7 => Some("unknown"),
            10 => Some("permission"),
            _ => Some("helper"),
        };
        Ok(Receipt {
            id: p.id.clone(),
            ok: u8::from(code == 0),
            g,
            m,
            at,
            e: error.map(Into::into),
            partial: (code == 3).then_some(1),
            next: (code == 7).then(|| "observe".into()),
            artifact,
        })
    }
    pub fn artifact(&mut self, id: &str, range: Option<Range>) -> Result<(u64, u64, Vec<u8>)> {
        let a = self.call(|s| json!([s, "artifact", id, range.map(|r| r.start), range.map(|r| r.end)]))?;
        ok(&a)?;
        if a.len() != 5 {
            return Err(Error::new(Code::Protocol, "invalid artifact reply"));
        }
        let size = u64v(&a[2])?;
        let start = u64v(&a[3])?;
        let bytes = STANDARD.decode(text(&a[4])?).map_err(|_| Error::new(Code::Protocol, "invalid artifact base64"))?;
        if bytes.len() > MAX_CHUNK {
            return Err(Error::new(Code::Bounds, "artifact reply exceeds chunk limit"));
        }
        Ok((size, start, bytes))
    }
    /// Downloads a whole device artifact in as few round trips as the helper
    /// allows, verifying that every slice lands exactly where it should.
    pub fn download(&mut self, id: &str, limit: usize) -> Result<Vec<u8>> {
        let (size, _, _) = self.artifact(id, Some(Range { start: 0, end: 0 }))?;
        let size = usize::try_from(size).ok().filter(|size| (1..=limit).contains(size)).ok_or_else(|| Error::new(Code::Bounds, "the device artifact is empty or too large"))?;
        let mut bytes = Vec::with_capacity(size);
        while bytes.len() < size {
            let (start, end) = (bytes.len() as u64, (bytes.len() + self.chunk).min(size) as u64);
            match self.artifact(id, Some(Range { start, end })) {
                Err(error) if error.code == Code::Bounds && self.chunk > MAX_INLINE => self.chunk = MAX_INLINE,
                Err(error) => return Err(error),
                Ok((total, actual, chunk)) => {
                    if total != size as u64 || actual != start || chunk.is_empty() || chunk.len() as u64 > end - start {
                        return Err(Error::new(Code::Protocol, "the device artifact was malformed"));
                    }
                    bytes.extend_from_slice(&chunk);
                }
            }
        }
        Ok(bytes)
    }
    pub fn query(&mut self, name: &str, args: Value) -> Result<Value> {
        let a = self.call(|s| json!([s, name, args]))?;
        if a.get(1).and_then(Value::as_u64) != Some(0) {
            return Err(helper_error(&a));
        }
        a.get(2).cloned().ok_or_else(|| Error::new(Code::Protocol, "helper query omitted its result"))
    }
}

pub(crate) fn send(s: &mut TcpStream, v: &Value) -> Result<()> {
    let b = serde_json::to_vec(v).map_err(|e| Error::new(Code::Protocol, e.to_string()))?;
    if b.len() > MAX_FRAME {
        return Err(Error::new(Code::Bounds, "helper request exceeds frame limit"));
    }
    s.write_all(&(b.len() as u32).to_be_bytes())?;
    s.write_all(&b)?;
    s.flush()?;
    Ok(())
}
pub(crate) fn recv(s: &mut TcpStream) -> Result<Value> {
    let mut h = [0u8; 4];
    s.read_exact(&mut h)?;
    let n = u32::from_be_bytes(h) as usize;
    if n == 0 || n > MAX_FRAME {
        return Err(Error::new(Code::Bounds, "helper frame length is invalid"));
    }
    let mut b = vec![0; n];
    s.read_exact(&mut b)?;
    serde_json::from_slice(&b).map_err(|e| Error::new(Code::Protocol, e.to_string()))
}
fn ok(a: &[Value]) -> Result<()> {
    if a.get(1).and_then(Value::as_u64) == Some(0) {
        Ok(())
    } else {
        Err(helper_error(a))
    }
}
fn helper_error(a: &[Value]) -> Error {
    let code = match a.get(1).and_then(Value::as_u64) {
        Some(5) => Code::Unsupported,
        Some(6) => Code::Bounds,
        Some(10) => Code::Permission,
        Some(11) => return crate::link::stopped(),
        _ => Code::Helper,
    };
    Error::new(code, a.get(2).and_then(Value::as_str).unwrap_or("helper rejected request"))
}
pub(crate) fn text(v: &Value) -> Result<&str> {
    v.as_str().ok_or_else(|| Error::new(Code::Protocol, "expected string"))
}
fn u64v(v: &Value) -> Result<u64> {
    v.as_u64().ok_or_else(|| Error::new(Code::Protocol, "expected unsigned integer"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stopped_helper_is_its_own_result() {
        let error = helper_error(&[json!(3), json!(11)]);
        assert_eq!((error.code, error.message.contains("au setup")), (Code::Stopped, true));
    }
    #[test]
    fn rejects_oversize_header() {
        let l = (MAX_FRAME as u32 + 1).to_be_bytes();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            s.write_all(&l).unwrap();
        });
        let mut s = TcpStream::connect(addr).unwrap();
        assert_eq!(recv(&mut s).unwrap_err().code, Code::Bounds);
        t.join().unwrap();
    }
    #[test]
    fn maps_accessibility_loss_to_permission() {
        let e = helper_error(&[json!(1), json!(10)]);
        assert_eq!(e.code, Code::Permission);
        assert_eq!(e.code.wire(), "permission");
    }

    /// A minimal helper that serves one artifact, optionally with the
    /// pre-1.1 limit of `MAX_INLINE` bytes per request.
    fn fake_helper(data: Vec<u8>, legacy: bool) -> (Bridge, std::thread::JoinHandle<usize>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut requests = 0;
            while let Ok(q) = recv(&mut s) {
                requests += 1;
                let seq = q[0].as_u64().unwrap();
                let (start, end) = (q[3].as_u64().unwrap() as usize, q[4].as_u64().unwrap() as usize);
                let reply =
                    if end > data.len() || (legacy && end - start > MAX_INLINE) { json!([seq, 6]) } else { json!([seq, 0, data.len(), start, STANDARD.encode(&data[start..end])]) };
                send(&mut s, &reply).unwrap();
            }
            requests
        });
        let stream = TcpStream::connect(addr).unwrap();
        (Bridge::new(Box::new(stream)), server)
    }

    #[test]
    fn downloads_large_artifacts_in_few_round_trips() {
        let data: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
        let (mut bridge, server) = fake_helper(data.clone(), false);
        assert_eq!(bridge.download("a1", 1 << 24).unwrap(), data);
        drop(bridge);
        assert_eq!(server.join().unwrap(), 1 + data.len().div_ceil(MAX_CHUNK));
    }

    #[test]
    fn falls_back_to_inline_slices_for_older_helpers() {
        let data: Vec<u8> = (0..9_000u32).map(|i| (i % 7) as u8).collect();
        let (mut bridge, _server) = fake_helper(data.clone(), true);
        assert_eq!(bridge.download("a1", 1 << 24).unwrap(), data);
        assert_eq!(bridge.chunk, MAX_INLINE);
    }

    #[test]
    fn refuses_artifacts_over_the_caller_limit() {
        let (mut bridge, _server) = fake_helper(vec![1; 5_000], false);
        assert_eq!(bridge.download("a1", 4_000).unwrap_err().code, Code::Bounds);
    }
}
