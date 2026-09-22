use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const WIRE_VERSION: u8 = 1;
/// Largest DataChannel message sent or accepted. webrtc-rs reads each incoming
/// message into a `u16::MAX`-byte buffer and closes the channel on anything
/// larger, so 64 KiB (65,536) would be one byte too many.
pub const MAX_MESSAGE_SIZE: usize = u16::MAX as usize;
pub const BINARY_HEADER_SIZE: usize = 14;
pub const MAX_IDENTIFIER_BYTES: usize = 128;

const BINARY_MAGIC: [u8; 4] = [0x52, 0x53, 0x42, WIRE_VERSION];

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PayloadEncoding {
    None,
    Json,
    Text,
    Binary,
    FormData,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PayloadDescriptor {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub encoding: PayloadEncoding,
    pub byte_length: u64,
    pub chunks: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

impl PayloadDescriptor {
    pub fn none() -> Self {
        Self {
            id: None,
            encoding: PayloadEncoding::None,
            byte_length: 0,
            chunks: 0,
            content_type: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum IncomingControl {
    Request(RequestFrame),
    Subscribe(SubscribeFrame),
    Cancel(CancelFrame),
    ResponseCredit(ResponseCreditFrame),
    Close(CloseFrame),
}

impl IncomingControl {
    pub fn version(&self) -> u8 {
        match self {
            Self::Request(frame) => frame.v,
            Self::Subscribe(frame) => frame.v,
            Self::Cancel(frame) => frame.v,
            Self::ResponseCredit(frame) => frame.v,
            Self::Close(frame) => frame.v,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ResponseCreditFrame {
    pub v: u8,
    pub id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestFrame {
    pub v: u8,
    pub id: String,
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub params: HashMap<String, Value>,
    #[serde(default)]
    pub response_type: Option<String>,
    /// Opts in to incrementally framed response bodies. Version 1 uses
    /// response-start/response-segment/response-end control frames.
    #[serde(default)]
    pub response_stream: Option<u8>,
    pub payload: PayloadDescriptor,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscribeFrame {
    pub v: u8,
    pub id: String,
    pub path: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub params: HashMap<String, Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CancelTarget {
    Request,
    Subscription,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CancelReason {
    Cancelled,
    Timeout,
    Closed,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CancelFrame {
    pub v: u8,
    pub id: String,
    pub target: CancelTarget,
    #[allow(dead_code)]
    pub reason: Option<CancelReason>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CloseFrame {
    pub v: u8,
    #[allow(dead_code)]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WireError {
    pub kind: &'static str,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum OutgoingControl<'a> {
    Response {
        v: u8,
        id: &'a str,
        status: u16,
        headers: HashMap<String, String>,
        payload: PayloadDescriptor,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<WireError>,
    },
    ResponseStart {
        v: u8,
        id: &'a str,
        status: u16,
        headers: HashMap<String, String>,
        encoding: PayloadEncoding,
        #[serde(rename = "contentType")]
        #[serde(skip_serializing_if = "Option::is_none")]
        content_type: Option<String>,
        #[serde(rename = "byteLength")]
        #[serde(skip_serializing_if = "Option::is_none")]
        byte_length: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<WireError>,
    },
    ResponseSegment {
        v: u8,
        id: &'a str,
        sequence: u64,
        payload: PayloadDescriptor,
    },
    ResponseEnd {
        v: u8,
        id: &'a str,
        segments: u64,
        #[serde(rename = "byteLength")]
        byte_length: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<WireError>,
    },
    Event {
        v: u8,
        #[serde(rename = "subscriptionId")]
        subscription_id: &'a str,
        sequence: u64,
        event: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        retry: Option<u64>,
        payload: PayloadDescriptor,
    },
    SubscriptionEnd {
        v: u8,
        #[serde(rename = "subscriptionId")]
        subscription_id: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<WireError>,
    },
    Close {
        v: u8,
        reason: &'a str,
    },
}

#[derive(Debug)]
pub struct BinaryChunk {
    pub payload_id: String,
    pub index: u32,
    pub count: u32,
    pub bytes: Vec<u8>,
}

pub fn decode_control(bytes: &[u8]) -> Result<IncomingControl, String> {
    if bytes.len() > MAX_MESSAGE_SIZE {
        return Err("control frame exceeds the message-size limit".to_owned());
    }
    let frame = serde_json::from_slice::<IncomingControl>(bytes)
        .map_err(|_| "invalid DataChannel control frame".to_owned())?;
    if frame.version() != WIRE_VERSION {
        return Err("unsupported DataChannel protocol version".to_owned());
    }
    Ok(frame)
}

pub fn encode_control(frame: &OutgoingControl<'_>) -> Result<String, String> {
    let value = serde_json::to_string(frame)
        .map_err(|_| "unable to encode DataChannel control frame".to_owned())?;
    if value.len() > MAX_MESSAGE_SIZE {
        return Err("control frame exceeds the message-size limit".to_owned());
    }
    Ok(value)
}

pub fn decode_binary(bytes: &[u8]) -> Result<BinaryChunk, String> {
    if bytes.len() > MAX_MESSAGE_SIZE {
        return Err("binary frame exceeds the message-size limit".to_owned());
    }
    if bytes.len() < BINARY_HEADER_SIZE || bytes[..4] != BINARY_MAGIC {
        return Err("invalid DataChannel binary frame".to_owned());
    }
    let id_length = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    let index = u32::from_be_bytes(bytes[6..10].try_into().unwrap());
    let count = u32::from_be_bytes(bytes[10..14].try_into().unwrap());
    if id_length == 0
        || id_length > MAX_IDENTIFIER_BYTES
        || bytes.len() < BINARY_HEADER_SIZE + id_length
        || count == 0
        || index >= count
    {
        return Err("invalid DataChannel binary frame fields".to_owned());
    }
    let payload_id =
        std::str::from_utf8(&bytes[BINARY_HEADER_SIZE..BINARY_HEADER_SIZE + id_length])
            .map_err(|_| "binary payload ID is not UTF-8".to_owned())?
            .to_owned();
    validate_identifier(&payload_id, "payload")?;
    Ok(BinaryChunk {
        payload_id,
        index,
        count,
        bytes: bytes[BINARY_HEADER_SIZE + id_length..].to_vec(),
    })
}

pub fn encode_binary(
    payload_id: &str,
    index: u32,
    count: u32,
    bytes: &[u8],
) -> Result<Vec<u8>, String> {
    validate_identifier(payload_id, "payload")?;
    if count == 0 || index >= count {
        return Err("invalid binary chunk index".to_owned());
    }
    let id = payload_id.as_bytes();
    let mut frame = Vec::with_capacity(BINARY_HEADER_SIZE + id.len() + bytes.len());
    frame.extend_from_slice(&BINARY_MAGIC);
    frame.extend_from_slice(&(id.len() as u16).to_be_bytes());
    frame.extend_from_slice(&index.to_be_bytes());
    frame.extend_from_slice(&count.to_be_bytes());
    frame.extend_from_slice(id);
    frame.extend_from_slice(bytes);
    if frame.len() > MAX_MESSAGE_SIZE {
        return Err("binary frame exceeds the message-size limit".to_owned());
    }
    Ok(frame)
}

pub fn max_chunk_size(payload_id: &str) -> Result<usize, String> {
    validate_identifier(payload_id, "payload")?;
    Ok(MAX_MESSAGE_SIZE - BINARY_HEADER_SIZE - payload_id.len())
}

pub fn validate_identifier(value: &str, kind: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > MAX_IDENTIFIER_BYTES {
        return Err(format!(
            "{kind} ID must be 1-{MAX_IDENTIFIER_BYTES} UTF-8 bytes"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_chunks_round_trip_and_reject_invalid_fields() {
        let encoded = encode_binary("payload-1", 1, 3, &[1, 2, 3]).unwrap();
        let decoded = decode_binary(&encoded).unwrap();
        assert_eq!(decoded.payload_id, "payload-1");
        assert_eq!(decoded.index, 1);
        assert_eq!(decoded.count, 3);
        assert_eq!(decoded.bytes, [1, 2, 3]);

        let mut bad_version = encoded.clone();
        bad_version[3] = 2;
        assert!(decode_binary(&bad_version).is_err());
        assert!(encode_binary("payload-1", 3, 3, &[]).is_err());
    }

    #[test]
    fn control_frames_require_version_one_and_known_types() {
        assert!(decode_control(br#"{"v":2,"type":"close"}"#).is_err());
        assert!(decode_control(br#"{"v":1,"type":"future"}"#).is_err());
        assert!(matches!(
            decode_control(br#"{"v":1,"type":"close"}"#).unwrap(),
            IncomingControl::Close(_)
        ));
        assert!(matches!(
            decode_control(br#"{"v":1,"type":"response-credit","id":"request-1"}"#).unwrap(),
            IncomingControl::ResponseCredit(_)
        ));
    }

    #[test]
    fn streaming_response_fields_use_the_browser_wire_names() {
        let encoded = encode_control(&OutgoingControl::ResponseStart {
            v: 1,
            id: "request-1",
            status: 200,
            headers: HashMap::new(),
            encoding: PayloadEncoding::Binary,
            content_type: Some("video/mp4".to_owned()),
            byte_length: Some(42),
            error: None,
        })
        .unwrap();
        let value: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(value["type"], "response-start");
        assert_eq!(value["contentType"], "video/mp4");
        assert_eq!(value["byteLength"], 42);
        assert!(value.get("content_type").is_none());
    }
}
