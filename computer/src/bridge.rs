use crate::{
    api::{Code, Error, Node, Plan, Range, Receipt, Result, Scene, MAX_FRAME},
    device::{Adb, Device},
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    thread,
    time::{Duration, Instant},
};

pub struct Bridge {
    adb: Adb,
    device: Device,
    port: u16,
    stream: TcpStream,
    seq: u64,
}

pub enum Observation {
    Unchanged(u64),
    Scene(Scene),
}

impl Drop for Bridge {
    fn drop(&mut self) {
        if self.port != 0 {
            self.adb.remove_forward(&self.device, self.port);
            self.port = 0;
        }
    }
}

impl Bridge {
    pub fn connect(adb: Adb, device: Device) -> Result<Self> {
        adb.start_helper(&device)?;
        // These abstract-socket names are a deployed wire-protocol identifier, not a public product version.
        // Keep them stable so a v1 host can reconnect to an already-installed helper.
        let bootstrap = adb.forward(&device, "localabstract:aubridge-bootstrap-v3")?;
        let auth = (|| {
            let mut s = connect(bootstrap, Duration::from_secs(8))?;
            send(&mut s, &json!([0, "bootstrap"]))?;
            let r = recv(&mut s)?;
            let a = r.as_array().ok_or_else(|| Error::new(Code::Protocol, "invalid bootstrap reply"))?;
            if a.len() != 4 || a[0].as_u64() != Some(0) || a[1].as_u64() != Some(0) {
                return Err(Error::new(Code::Auth, "helper bootstrap was rejected"));
            }
            Ok((text(&a[2])?.to_string(), text(&a[3])?.to_string()))
        })();
        let (token, nonce) = match auth {
            Ok(v) => v,
            Err(e) => {
                adb.remove_forward(&device, bootstrap);
                return Err(e);
            }
        };
        adb.remove_forward(&device, bootstrap);
        let command = adb.forward(&device, "localabstract:aubridge-v3")?;
        let result = (|| {
            let mut stream = connect(command, Duration::from_secs(5))?;
            send(&mut stream, &json!([0, "hello", token, nonce]))?;
            let h = recv(&mut stream)?;
            let a = h.as_array().ok_or_else(|| Error::new(Code::Protocol, "invalid hello reply"))?;
            if a.len() < 2 || a[0].as_u64() != Some(0) || a[1].as_u64() != Some(0) {
                return Err(Error::new(Code::Auth, "helper authentication failed"));
            }
            Ok(Self { adb: adb.clone(), device: device.clone(), port: command, stream, seq: 0 })
        })();
        if result.is_err() {
            adb.remove_forward(&device, command);
        }
        result
    }
    fn call(&mut self, make: impl FnOnce(u64) -> Value) -> Result<Vec<Value>> {
        self.seq = self.seq.checked_add(1).ok_or_else(|| Error::new(Code::Sequence, "sequence exhausted"))?;
        send(&mut self.stream, &make(self.seq))?;
        let v = recv(&mut self.stream)?;
        let a = v.as_array().ok_or_else(|| Error::new(Code::Protocol, "helper reply must be an array"))?;
        if a.first().and_then(Value::as_u64) != Some(self.seq) {
            return Err(Error::new(Code::Sequence, "helper reply sequence mismatch"));
        }
        Ok(a.clone())
    }
    pub fn status(&mut self) -> Result<(u64, u32)> {
        let a = self.call(|s| json!([s, "status"]))?;
        ok(&a)?;
        if a.len() != 4 {
            return Err(Error::new(Code::Protocol, "invalid status reply"));
        }
        Ok((u64v(&a[2])?, u32::try_from(u64v(&a[3])?).map_err(|_| Error::new(Code::Protocol, "capability mask overflow"))?))
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
        if bytes.len() > crate::api::MAX_INLINE {
            return Err(Error::new(Code::Bounds, "artifact reply exceeds inline limit"));
        }
        Ok((size, start, bytes))
    }
    pub fn query(&mut self, name: &str, args: Value) -> Result<Value> {
        let a = self.call(|s| json!([s, name, args]))?;
        if a.get(1).and_then(Value::as_u64) != Some(0) {
            return Err(helper_error(&a));
        }
        a.get(2).cloned().ok_or_else(|| Error::new(Code::Protocol, "helper query omitted its result"))
    }
}

fn connect(port: u16, timeout: Duration) -> Result<TcpStream> {
    let end = Instant::now() + timeout;
    loop {
        match TcpStream::connect_timeout(&SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port), Duration::from_millis(500)) {
            Ok(s) => {
                s.set_read_timeout(Some(Duration::from_secs(35)))?;
                s.set_write_timeout(Some(Duration::from_secs(5)))?;
                return Ok(s);
            }
            Err(e) if Instant::now() < end => {
                let _ = e;
                thread::sleep(Duration::from_millis(100))
            }
            Err(e) => return Err(Error::new(Code::Helper, format!("helper connection failed: {e}"))),
        }
    }
}
fn send(s: &mut TcpStream, v: &Value) -> Result<()> {
    let b = serde_json::to_vec(v).map_err(|e| Error::new(Code::Protocol, e.to_string()))?;
    if b.len() > MAX_FRAME {
        return Err(Error::new(Code::Bounds, "helper request exceeds frame limit"));
    }
    s.write_all(&(b.len() as u32).to_be_bytes())?;
    s.write_all(&b)?;
    s.flush()?;
    Ok(())
}
fn recv(s: &mut TcpStream) -> Result<Value> {
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
        _ => Code::Helper,
    };
    Error::new(code, a.get(2).and_then(Value::as_str).unwrap_or("helper rejected request"))
}
fn text(v: &Value) -> Result<&str> {
    v.as_str().ok_or_else(|| Error::new(Code::Protocol, "expected string"))
}
fn u64v(v: &Value) -> Result<u64> {
    v.as_u64().ok_or_else(|| Error::new(Code::Protocol, "expected unsigned integer"))
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
