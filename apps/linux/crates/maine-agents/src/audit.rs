//! One thing an agent did, as the user can review it later. The app keeps the
//! list in the encrypted device-local state (never in the vault, never synced,
//! never in diagnostics) as opaque JSON; the shape here is the Swift `Codable`
//! one so a state file can be read by either client. An entry holds commands the
//! agent ran and the fact that a saved password was typed; never the password,
//! never output, and never literal text sent with `send_keys` (only its length).

use crate::apple::AppleDate;
use crate::json::uuid_text;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditKind {
    Open,
    Run,
    Keys,
    Password,
    Close,
    /// The user handed one of their own sessions to this client.
    Handoff,
}

fn serialize_uuid<S: Serializer>(id: &Uuid, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&uuid_text(id))
}

fn deserialize_detail<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.map(|text| AgentAuditEntry::trimmed(&text)))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentAuditEntry {
    #[serde(serialize_with = "serialize_uuid")]
    pub id: Uuid,
    pub at: AppleDate,
    pub client: String,
    #[serde(rename = "hostName")]
    pub host_name: String,
    #[serde(rename = "sessionID", serialize_with = "serialize_uuid")]
    pub session_id: Uuid,
    pub kind: AuditKind,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_detail"
    )]
    pub detail: Option<String>,
    #[serde(rename = "exitCode", default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

impl AgentAuditEntry {
    /// Newest entries kept per vault.
    pub const LIMIT: usize = 1000;
    /// Longest command text stored, in bytes.
    pub const DETAIL_LIMIT: usize = 2048;

    pub fn new(
        client: impl Into<String>,
        host_name: impl Into<String>,
        session_id: Uuid,
        kind: AuditKind,
        detail: Option<&str>,
        exit_code: Option<i32>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            at: AppleDate::now(),
            client: client.into(),
            host_name: host_name.into(),
            session_id,
            kind,
            detail: detail.map(Self::trimmed),
            exit_code,
        }
    }

    /// A command longer than [`DETAIL_LIMIT`](Self::DETAIL_LIMIT) bytes is cut
    /// at a character boundary and ends in an ellipsis.
    pub fn trimmed(text: &str) -> String {
        if text.len() <= Self::DETAIL_LIMIT {
            return text.to_string();
        }
        let mut kept = String::new();
        for c in text.chars() {
            if kept.len() + c.len_utf8() > Self::DETAIL_LIMIT {
                break;
            }
            kept.push(c);
        }
        kept.push('…');
        kept
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn from_value(value: &Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value(value.clone())
    }

    pub fn list_to_value(entries: &[Self]) -> Value {
        serde_json::to_value(entries).unwrap_or(Value::Null)
    }

    pub fn list_from_value(value: &Value) -> Result<Vec<Self>, serde_json::Error> {
        serde_json::from_value(value.clone())
    }

    /// Appends `new` entries, keeping only the newest [`LIMIT`](Self::LIMIT).
    pub fn append(list: &mut Vec<Self>, new: impl IntoIterator<Item = Self>) {
        list.extend(new);
        if list.len() > Self::LIMIT {
            let excess = list.len() - Self::LIMIT;
            list.drain(..excess);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(kind: AuditKind, detail: Option<&str>, exit: Option<i32>) -> AgentAuditEntry {
        AgentAuditEntry::new("Claude Code", "Web", Uuid::new_v4(), kind, detail, exit)
    }

    // The Swift test that seals entries in the local state file belongs to the
    // app (the file is its). Here: the stored value round-trips, and what the
    // value holds never includes more than the entry's fields.
    #[test]
    fn entries_round_trip_through_the_opaque_value() {
        let first = entry(AuditKind::Run, Some("systemctl restart nginx"), Some(0));
        let second = entry(AuditKind::Password, None, None);
        let value = AgentAuditEntry::list_to_value(&[first.clone(), second.clone()]);
        assert_eq!(
            AgentAuditEntry::list_from_value(&value).unwrap(),
            vec![first, second]
        );
        // Absent optionals are omitted, as Swift's synthesized encoder does.
        let object = value[1].as_object().unwrap();
        assert!(!object.contains_key("detail") && !object.contains_key("exitCode"));
    }

    #[test]
    fn json_matches_the_swift_codable_shape() {
        let id = Uuid::parse_str("0f8fad5b-d9cb-469f-a165-70867728950e").unwrap();
        let session = Uuid::parse_str("7c9e6679-7425-40de-944b-e07fc1f90ae7").unwrap();
        let mut entry = AgentAuditEntry::new(
            "Claude Code",
            "Web",
            session,
            AuditKind::Handoff,
            None,
            None,
        );
        entry.id = id;
        entry.at = AppleDate(785_000_000.25);
        assert_eq!(
            entry.to_value(),
            json!({
                "id": "0F8FAD5B-D9CB-469F-A165-70867728950E",
                "at": 785_000_000.25,
                "client": "Claude Code",
                "hostName": "Web",
                "sessionID": "7C9E6679-7425-40DE-944B-E07FC1F90AE7",
                "kind": "handoff"
            })
        );
        let swift = json!({
            "id": "0f8fad5b-d9cb-469f-a165-70867728950e", "at": 1.5, "client": "c", "hostName": "h",
            "sessionID": "7c9e6679-7425-40de-944b-e07fc1f90ae7", "kind": "run", "detail": "ls", "exitCode": 2
        });
        let read = AgentAuditEntry::from_value(&swift).unwrap();
        assert_eq!(
            (read.kind, read.detail.as_deref(), read.exit_code),
            (AuditKind::Run, Some("ls"), Some(2))
        );
        for kind in ["open", "run", "keys", "password", "close", "handoff"] {
            let mut value = swift.clone();
            value["kind"] = json!(kind);
            assert!(AgentAuditEntry::from_value(&value).is_ok(), "{kind}");
        }
        let mut bad = swift.clone();
        bad["kind"] = json!("explode");
        assert!(AgentAuditEntry::from_value(&bad).is_err());
    }

    #[test]
    fn the_log_keeps_the_newest_entries_and_trims_long_commands() {
        let mut list = Vec::new();
        let batch: Vec<_> = (0..AgentAuditEntry::LIMIT + 25)
            .map(|i| entry(AuditKind::Run, Some(&format!("cmd {i}")), None))
            .collect();
        AgentAuditEntry::append(&mut list, batch);
        assert_eq!(list.len(), AgentAuditEntry::LIMIT);
        assert_eq!(list.first().unwrap().detail.as_deref(), Some("cmd 25"));
        assert_eq!(
            list.last().unwrap().detail.as_deref(),
            Some(format!("cmd {}", AgentAuditEntry::LIMIT + 24).as_str())
        );

        let long = "x".repeat(10_000);
        let detail = entry(AuditKind::Run, Some(&long), None).detail.unwrap();
        assert!(detail.len() <= AgentAuditEntry::DETAIL_LIMIT + 16);
        assert!(detail.ends_with('…'));
        // Never splits a character.
        let wide = "é".repeat(5_000);
        assert!(
            entry(AuditKind::Run, Some(&wide), None)
                .detail
                .unwrap()
                .len()
                <= AgentAuditEntry::DETAIL_LIMIT + 3
        );
    }

    #[test]
    fn a_stored_long_detail_is_trimmed_on_read_too() {
        let mut value = entry(AuditKind::Run, Some("ls"), None).to_value();
        value["detail"] = json!("y".repeat(5_000));
        let read = AgentAuditEntry::from_value(&value).unwrap();
        assert!(read.detail.unwrap().len() <= AgentAuditEntry::DETAIL_LIMIT + 3);
    }
}
