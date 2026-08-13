use std::io::{Cursor, Read, Write};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AuError, Result};
use crate::{MAX_PROTOCOL_FRAME, PROTOCOL_VERSION};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Request {
    pub version: u16,
    pub id: u64,
    #[serde(flatten)]
    pub body: RequestBody,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RequestBody {
    Hello { client_version: String },
    Execute { argv: Vec<String> },
    Stop,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Response {
    pub version: u16,
    pub id: u64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProtocolError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameMode {
    Json,
    Native,
}

const NATIVE_MAGIC: [u8; 4] = *b"AU2\0";
const NATIVE_REQUEST: u8 = 1;
const NATIVE_RESPONSE: u8 = 2;
const NATIVE_DETAILS_PREFIX: &str = "\u{1e}AUDETAILS:";

impl Response {
    pub fn ok(id: u64, data: Value) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn error(id: u64, error: &AuError) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            ok: false,
            data: None,
            error: Some(ProtocolError {
                code: error.kind().into(),
                message: error.compact_message(),
                details: error.details().cloned(),
            }),
        }
    }
}

pub fn write_frame<W: Write, T: Serialize>(writer: &mut W, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    write_length_prefixed(writer, &bytes)
}

pub fn read_frame<R: Read, T: for<'a> Deserialize<'a>>(reader: &mut R) -> Result<T> {
    let bytes = read_payload(reader)?;
    serde_json::from_slice(&bytes).map_err(|error| AuError::code("E_FRAME", error.to_string()))
}

pub fn write_native_request<W: Write>(writer: &mut W, request: &Request) -> Result<()> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&NATIVE_MAGIC);
    payload.push(NATIVE_REQUEST);
    put_u16(&mut payload, request.version);
    put_u64(&mut payload, request.id);
    match &request.body {
        RequestBody::Hello { client_version } => {
            payload.push(1);
            put_string(&mut payload, client_version)?;
        }
        RequestBody::Execute { argv } => {
            if argv.len() > u16::MAX as usize {
                return Err(AuError::code("E_FRAME", "too many execute arguments"));
            }
            payload.push(2);
            put_u16(&mut payload, argv.len() as u16);
            for argument in argv {
                put_string(&mut payload, argument)?;
            }
        }
        RequestBody::Stop => payload.push(3),
    }
    write_length_prefixed(writer, &payload)
}

pub fn read_native_response<R: Read>(reader: &mut R) -> Result<Response> {
    let payload = read_payload(reader)?;
    decode_native_response(&payload)
}

pub fn read_daemon_request<R: Read>(reader: &mut R) -> Result<(Request, FrameMode)> {
    let payload = read_payload(reader)?;
    if payload.starts_with(&NATIVE_MAGIC) {
        Ok((decode_native_request(&payload)?, FrameMode::Native))
    } else {
        let request = serde_json::from_slice(&payload)
            .map_err(|error| AuError::code("E_FRAME", error.to_string()))?;
        Ok((request, FrameMode::Json))
    }
}

pub fn write_daemon_response<W: Write>(
    writer: &mut W,
    response: &Response,
    mode: FrameMode,
) -> Result<()> {
    match mode {
        FrameMode::Json => write_frame(writer, response),
        FrameMode::Native => {
            let mut payload = Vec::new();
            payload.extend_from_slice(&NATIVE_MAGIC);
            payload.push(NATIVE_RESPONSE);
            put_u16(&mut payload, response.version);
            put_u64(&mut payload, response.id);
            if response.ok {
                payload.push(1);
                let data = response
                    .data
                    .as_ref()
                    .ok_or_else(|| AuError::code("E_FRAME", "success response omitted data"))?;
                let data = serde_json::to_vec(data)?;
                if data.len() > MAX_PROTOCOL_FRAME {
                    return Err(AuError::code("E_FRAME", "response exceeds maximum size"));
                }
                put_u32(&mut payload, data.len() as u32);
                payload.extend_from_slice(&data);
            } else {
                payload.push(0);
                let error = response
                    .error
                    .as_ref()
                    .ok_or_else(|| AuError::code("E_FRAME", "error response omitted error"))?;
                put_string(&mut payload, &error.code)?;
                put_string(
                    &mut payload,
                    &native_error_message(&error.message, error.details.as_ref())?,
                )?;
            }
            write_length_prefixed(writer, &payload)
        }
    }
}

fn write_length_prefixed<W: Write>(writer: &mut W, payload: &[u8]) -> Result<()> {
    if payload.is_empty() || payload.len() > MAX_PROTOCOL_FRAME {
        return Err(AuError::code("E_FRAME", "invalid frame size"));
    }
    writer.write_all(&(payload.len() as u32).to_le_bytes())?;
    writer.write_all(payload)?;
    writer.flush()?;
    Ok(())
}

fn read_payload<R: Read>(reader: &mut R) -> Result<Vec<u8>> {
    let mut header = [0u8; 4];
    reader.read_exact(&mut header)?;
    let length = u32::from_le_bytes(header) as usize;
    if length == 0 || length > MAX_PROTOCOL_FRAME {
        return Err(AuError::code("E_FRAME", "invalid frame size"));
    }
    let mut bytes = vec![0u8; length];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn put_u16(payload: &mut Vec<u8>, value: u16) {
    payload.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(payload: &mut Vec<u8>, value: u32) {
    payload.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(payload: &mut Vec<u8>, value: u64) {
    payload.extend_from_slice(&value.to_le_bytes());
}

fn put_string(payload: &mut Vec<u8>, value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    if bytes.len() > u16::MAX as usize {
        return Err(AuError::code(
            "E_FRAME",
            "native string exceeds maximum size",
        ));
    }
    put_u16(payload, bytes.len() as u16);
    payload.extend_from_slice(bytes);
    Ok(())
}

fn native_error_message(message: &str, details: Option<&Value>) -> Result<String> {
    let Some(details) = details else {
        return Ok(message.into());
    };
    let encoded =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(details)?);
    let combined = format!("{message}{NATIVE_DETAILS_PREFIX}{encoded}");
    if combined.len() > u16::MAX as usize {
        return Err(AuError::code(
            "E_FRAME",
            "native error details exceed maximum size",
        ));
    }
    Ok(combined)
}

fn decode_native_error_message(message: &str) -> Result<(String, Option<Value>)> {
    let Some((plain, encoded)) = message.split_once(NATIVE_DETAILS_PREFIX) else {
        return Ok((message.into(), None));
    };
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|error| {
            AuError::code("E_FRAME", format!("invalid native error details: {error}"))
        })?;
    let details = serde_json::from_slice(&bytes).map_err(|error| {
        AuError::code("E_FRAME", format!("invalid native error details: {error}"))
    })?;
    Ok((plain.into(), Some(details)))
}

fn take_exact<'a>(cursor: &mut Cursor<&'a [u8]>, length: usize) -> Result<&'a [u8]> {
    let position = cursor.position() as usize;
    let end = position
        .checked_add(length)
        .ok_or_else(|| AuError::code("E_FRAME", "native frame position overflow"))?;
    if end > cursor.get_ref().len() {
        return Err(AuError::code("E_FRAME", "truncated native frame"));
    }
    cursor.set_position(end as u64);
    Ok(&cursor.get_ref()[position..end])
}

fn take_u8(cursor: &mut Cursor<&[u8]>) -> Result<u8> {
    Ok(take_exact(cursor, 1)?[0])
}

fn take_u16(cursor: &mut Cursor<&[u8]>) -> Result<u16> {
    let bytes = take_exact(cursor, 2)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn take_u32(cursor: &mut Cursor<&[u8]>) -> Result<u32> {
    let bytes = take_exact(cursor, 4)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn take_u64(cursor: &mut Cursor<&[u8]>) -> Result<u64> {
    let bytes = take_exact(cursor, 8)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn take_string(cursor: &mut Cursor<&[u8]>) -> Result<String> {
    let length = take_u16(cursor)? as usize;
    let bytes = take_exact(cursor, length)?;
    String::from_utf8(bytes.to_vec())
        .map_err(|error| AuError::code("E_FRAME", format!("invalid native UTF-8: {error}")))
}

fn finish_native(cursor: &Cursor<&[u8]>) -> Result<()> {
    if cursor.position() as usize != cursor.get_ref().len() {
        return Err(AuError::code("E_FRAME", "trailing native frame bytes"));
    }
    Ok(())
}

fn native_header(cursor: &mut Cursor<&[u8]>, kind: u8) -> Result<()> {
    if take_exact(cursor, NATIVE_MAGIC.len())? != NATIVE_MAGIC {
        return Err(AuError::code("E_FRAME", "invalid native frame magic"));
    }
    if take_u8(cursor)? != kind {
        return Err(AuError::code("E_FRAME", "invalid native frame kind"));
    }
    Ok(())
}

fn decode_native_request(payload: &[u8]) -> Result<Request> {
    let mut cursor = Cursor::new(payload);
    native_header(&mut cursor, NATIVE_REQUEST)?;
    let version = take_u16(&mut cursor)?;
    let id = take_u64(&mut cursor)?;
    let body = match take_u8(&mut cursor)? {
        1 => RequestBody::Hello {
            client_version: take_string(&mut cursor)?,
        },
        2 => {
            let count = take_u16(&mut cursor)? as usize;
            if count > 512 {
                return Err(AuError::code(
                    "E_PROTOCOL",
                    "invalid execute argument vector",
                ));
            }
            let mut argv = Vec::with_capacity(count);
            for _ in 0..count {
                let argument = take_string(&mut cursor)?;
                if argument.len() > 8_192 {
                    return Err(AuError::code(
                        "E_PROTOCOL",
                        "invalid execute argument vector",
                    ));
                }
                argv.push(argument);
            }
            RequestBody::Execute { argv }
        }
        3 => RequestBody::Stop,
        _ => return Err(AuError::code("E_FRAME", "invalid native request body")),
    };
    finish_native(&cursor)?;
    Ok(Request { version, id, body })
}

fn decode_native_response(payload: &[u8]) -> Result<Response> {
    let mut cursor = Cursor::new(payload);
    native_header(&mut cursor, NATIVE_RESPONSE)?;
    let version = take_u16(&mut cursor)?;
    let id = take_u64(&mut cursor)?;
    let ok = match take_u8(&mut cursor)? {
        0 => false,
        1 => true,
        _ => return Err(AuError::code("E_FRAME", "invalid native response status")),
    };
    let (data, error) = if ok {
        let length = take_u32(&mut cursor)? as usize;
        if length > MAX_PROTOCOL_FRAME {
            return Err(AuError::code(
                "E_FRAME",
                "native response data exceeds limit",
            ));
        }
        let bytes = take_exact(&mut cursor, length)?;
        let data = serde_json::from_slice(bytes)
            .map_err(|error| AuError::code("E_FRAME", error.to_string()))?;
        (Some(data), None)
    } else {
        let code = take_string(&mut cursor)?;
        let (message, details) = decode_native_error_message(&take_string(&mut cursor)?)?;
        (
            None,
            Some(ProtocolError {
                code,
                message,
                details,
            }),
        )
    };
    finish_native(&cursor)?;
    Ok(Response {
        version,
        id,
        ok,
        data,
        error,
    })
}

pub fn validate_request(request: &Request) -> Result<()> {
    if request.version != PROTOCOL_VERSION {
        return Err(AuError::code(
            "E_PROTOCOL",
            format!(
                "protocol {} is incompatible with {}",
                request.version, PROTOCOL_VERSION
            ),
        ));
    }
    if let RequestBody::Execute { argv } = &request.body {
        if argv.is_empty() || argv.len() > 512 || argv.iter().any(|argument| argument.len() > 8192)
        {
            return Err(AuError::code(
                "E_PROTOCOL",
                "invalid execute argument vector",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        read_frame, read_native_response, validate_request, write_daemon_response, write_frame,
        write_native_request, FrameMode, ProtocolError, Request, RequestBody, Response,
    };
    use crate::{MAX_PROTOCOL_FRAME, PROTOCOL_VERSION};

    #[test]
    fn frames_round_trip() {
        let request = Request {
            version: PROTOCOL_VERSION,
            id: 9,
            body: RequestBody::Hello {
                client_version: "test".into(),
            },
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &request).expect("write");
        let result: Request = read_frame(&mut bytes.as_slice()).expect("read");
        assert_eq!(result.id, 9);
    }

    #[test]
    fn malformed_or_oversized_frames_are_rejected_before_allocation() {
        let mut bytes = ((MAX_PROTOCOL_FRAME + 1) as u32).to_le_bytes().to_vec();
        bytes.extend_from_slice(b"never-read");
        let error = read_frame::<_, Request>(&mut bytes.as_slice()).expect_err("oversized frame");
        assert_eq!(error.kind(), "E_FRAME");
    }

    #[test]
    fn protocol_rejects_incompatible_and_unbounded_execute_requests() {
        let incompatible = Request {
            version: PROTOCOL_VERSION + 1,
            id: 1,
            body: RequestBody::Stop,
        };
        assert_eq!(
            validate_request(&incompatible).expect_err("version").kind(),
            "E_PROTOCOL"
        );
        let oversized = Request {
            version: PROTOCOL_VERSION,
            id: 2,
            body: RequestBody::Execute {
                argv: vec!["x".repeat(8_193)],
            },
        };
        assert_eq!(
            validate_request(&oversized).expect_err("argv bound").kind(),
            "E_PROTOCOL"
        );
    }

    #[test]
    fn native_daemon_frames_round_trip_without_json_request_overhead() {
        let request = Request {
            version: PROTOCOL_VERSION,
            id: 17,
            body: RequestBody::Execute {
                argv: vec![
                    "ui".into(),
                    "find".into(),
                    "text~Allow,clickable=true#0".into(),
                ],
            },
        };
        let mut bytes = Vec::new();
        write_native_request(&mut bytes, &request).expect("native request");
        assert!(bytes.len() < serde_json::to_vec(&request).expect("json request").len());

        let response = Response::ok(17, serde_json::json!({"ok":true,"n":1}));
        let mut response_bytes = Vec::new();
        write_daemon_response(&mut response_bytes, &response, FrameMode::Native)
            .expect("native response");
        let decoded = read_native_response(&mut response_bytes.as_slice()).expect("response");
        assert_eq!(decoded.id, 17);
        assert!(decoded.ok);
    }

    #[test]
    fn native_error_frames_preserve_bounded_recovery_details() {
        let response = Response {
            version: PROTOCOL_VERSION,
            id: 18,
            ok: false,
            data: None,
            error: Some(ProtocolError {
                code: "E_PARTIAL".into(),
                message: "step failed".into(),
                details: Some(serde_json::json!({"failed_index":2,"next":"observe"})),
            }),
        };
        let mut bytes = Vec::new();
        write_daemon_response(&mut bytes, &response, FrameMode::Native).expect("write error");
        let decoded = read_native_response(&mut bytes.as_slice()).expect("read error");
        let error = decoded.error.expect("protocol error");
        assert_eq!(error.code, "E_PARTIAL");
        assert_eq!(error.details.expect("details")["failed_index"], 2);
    }

    #[test]
    fn native_frames_reject_bad_magic_and_trailing_bytes() {
        let mut bytes = Vec::new();
        write_native_request(
            &mut bytes,
            &Request {
                version: PROTOCOL_VERSION,
                id: 1,
                body: RequestBody::Stop,
            },
        )
        .expect("request");
        bytes.push(0);
        let length = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) + 1;
        bytes[0..4].copy_from_slice(&length.to_le_bytes());
        assert_eq!(
            super::read_daemon_request(&mut bytes.as_slice())
                .expect_err("trailing bytes")
                .kind(),
            "E_FRAME"
        );
    }
}
