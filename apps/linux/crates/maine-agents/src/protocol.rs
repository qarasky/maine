//! Messages between `maine --mcp` (the stdio bridge) and the running app, over
//! the agent socket. Each frame is a 4-byte big-endian length followed by that
//! many bytes of UTF-8 JSON, at most 8 MiB.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

/// The most a frame may carry.
pub const MAX_FRAME_LENGTH: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    FrameTooLarge,
    Encoding(String),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameTooLarge => f.write_str("frame too large"),
            Self::Encoding(message) => write!(f, "could not encode a frame: {message}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

/// The length prefix and the JSON body of `value`.
pub fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>, ProtocolError> {
    let body = serde_json::to_vec(value).map_err(|e| ProtocolError::Encoding(e.to_string()))?;
    if body.len() > MAX_FRAME_LENGTH {
        return Err(ProtocolError::FrameTooLarge);
    }
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Accumulates bytes from the socket and yields whole frames.
#[derive(Debug, Default)]
pub struct FrameReader {
    buffer: Vec<u8>,
}

impl FrameReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, data: &[u8]) -> Result<Vec<Vec<u8>>, ProtocolError> {
        self.buffer.extend_from_slice(data);
        let mut frames = Vec::new();
        let mut consumed = 0;
        loop {
            let rest = &self.buffer[consumed..];
            if rest.len() < 4 {
                break;
            }
            let length = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
            if length > MAX_FRAME_LENGTH {
                return Err(ProtocolError::FrameTooLarge);
            }
            if rest.len() < 4 + length {
                break;
            }
            frames.push(rest[4..4 + length].to_vec());
            consumed += 4 + length;
        }
        self.buffer.drain(..consumed);
        Ok(frames)
    }
}

/// First frame from the bridge. `client_name` is informational only. Its
/// `Debug` output never shows the token.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentHello {
    pub token: String,
    #[serde(
        rename = "clientName",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub client_name: Option<String>,
}

impl fmt::Debug for AgentHello {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentHello")
            .field("token", &"<redacted>")
            .field("client_name", &self.client_name)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentHelloReply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentCall {
    pub id: i64,
    pub tool: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentReply {
    pub id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl AgentReply {
    pub fn success(id: i64, result: Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(id: i64, message: impl Into<String>) -> Self {
        Self {
            id,
            result: None,
            error: Some(message.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::ValueExt;
    use serde_json::json;

    #[test]
    fn frames_round_trip_and_split_anywhere() {
        let call = AgentCall {
            id: 7,
            tool: "run".into(),
            arguments: json!({"command": "ls", "timeout_seconds": 5, "nested": [true, null]}),
        };
        let frame = encode_frame(&call).unwrap();
        let mut reader = FrameReader::new();
        let mut frames = Vec::new();
        for byte in &frame {
            frames.extend(reader.feed(&[*byte]).unwrap());
        }
        assert_eq!(frames.len(), 1);
        let decoded: AgentCall = serde_json::from_slice(&frames[0]).unwrap();
        assert_eq!(decoded, call);
        assert_eq!(decoded.arguments.string("command"), Some("ls"));
        assert_eq!(decoded.arguments.int("timeout_seconds"), Some(5));
        assert_eq!(decoded.arguments.int("missing"), None);

        // Two frames in one read.
        let doubled = [frame.clone(), frame].concat();
        assert_eq!(reader.feed(&doubled).unwrap().len(), 2);
    }

    #[test]
    fn oversized_frames_are_refused() {
        let mut reader = FrameReader::new();
        assert_eq!(
            reader.feed(&[0x00, 0x80, 0x00, 0x01]),
            Err(ProtocolError::FrameTooLarge)
        );
        // Exactly the limit is allowed to start.
        let mut reader = FrameReader::new();
        assert_eq!(reader.feed(&[0x00, 0x80, 0x00, 0x00]), Ok(vec![]));
    }

    #[test]
    fn encoding_refuses_bodies_over_the_limit() {
        let huge = json!({"blob": "x".repeat(MAX_FRAME_LENGTH)});
        assert_eq!(encode_frame(&huge), Err(ProtocolError::FrameTooLarge));
    }

    #[test]
    fn a_hello_never_prints_its_token() {
        let hello = AgentHello {
            token: "maine-agent-SECRET".into(),
            client_name: None,
        };
        assert!(!format!("{hello:?}").contains("SECRET"));
    }

    #[test]
    fn wire_names_match_the_swift_types() {
        let hello = AgentHello {
            token: "t".into(),
            client_name: Some("c".into()),
        };
        assert_eq!(
            serde_json::to_value(&hello).unwrap(),
            json!({"token": "t", "clientName": "c"})
        );
        assert_eq!(
            serde_json::to_value(AgentHello {
                token: "t".into(),
                client_name: None
            })
            .unwrap(),
            json!({"token": "t"})
        );
        assert_eq!(
            serde_json::to_value(AgentHelloReply {
                ok: false,
                error: Some("e".into())
            })
            .unwrap(),
            json!({"ok": false, "error": "e"})
        );
        assert_eq!(
            serde_json::to_value(AgentReply::success(1, json!({}))).unwrap(),
            json!({"id": 1, "result": {}})
        );
        assert_eq!(
            serde_json::to_value(AgentReply::failure(2, "x")).unwrap(),
            json!({"id": 2, "error": "x"})
        );
        let call: AgentCall =
            serde_json::from_value(json!({"id": 3, "tool": "t", "arguments": null})).unwrap();
        assert_eq!(call.arguments, Value::Null);
    }
}
