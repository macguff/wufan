//! Versioned, bounded wire envelope. Framing uses a little-endian u32 length.
use crate::{BrokerGeneration, ClientInstanceId, CommitId, MessageIdentity};
use serde::{Deserialize, Serialize};

pub const VERSION: u16 = 1;
pub const MAX_FRAME: usize = 64 * 1024;
pub const MAX_TEXT: usize = 16 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub version: u16,
    pub client_instance_id: ClientInstanceId,
    pub broker_generation: BrokerGeneration,
    pub learning_enabled: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u16,
    pub identity: MessageIdentity,
    pub action: Action,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum Action {
    Open,
    Key {
        code: i32,
        modifiers: i32,
    },
    Clear,
    Close,
    CommitAck {
        commit_id: CommitId,
        outcome: CommitOutcome,
    },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CommitOutcome {
    Applied,
    Rejected,
    Indeterminate,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub version: u16,
    pub identity: MessageIdentity,
    pub result: ReplyResult,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ReplyResult {
    Opened,
    View {
        consumed: bool,
        preedit: String,
        cursor_utf16: u32,
        candidates: Vec<WireCandidate>,
        highlighted: u32,
        page: u32,
        last_page: bool,
        commit: Option<CommitIntent>,
    },
    Cleared,
    Closed,
    Acknowledged,
    Error {
        code: ErrorCode,
    },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ErrorCode {
    StaleIdentity,
    InvalidSequence,
    InvalidState,
    Capacity,
    EngineFailure,
    InvalidKey,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireCandidate {
    pub text: String,
    pub comment: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitIntent {
    pub id: CommitId,
    pub text: String,
}

pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let body = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if body.is_empty() || body.len() > MAX_FRAME {
        return Err("frame size exceeds limit".into());
    }
    let mut bytes = Vec::with_capacity(body.len() + 4);
    bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&body);
    Ok(bytes)
}
pub fn decode<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, String> {
    if body.is_empty() || body.len() > MAX_FRAME {
        return Err("invalid frame size".into());
    }
    serde_json::from_slice(body).map_err(|e| e.to_string())
}
pub fn frame_len(header: [u8; 4]) -> Result<usize, String> {
    let len = u32::from_le_bytes(header) as usize;
    if len == 0 || len > MAX_FRAME {
        Err("invalid frame size".into())
    } else {
        Ok(len)
    }
}
