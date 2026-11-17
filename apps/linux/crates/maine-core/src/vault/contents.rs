use crate::model::uuid_upper::upper;
use crate::model::{Host, HostGroup};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use uuid::Uuid;

/// A portable session snapshot. Read only for migration from older vaults; the
/// Linux client keeps sessions in the device-local file instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedSession {
    #[serde(serialize_with = "crate::model::uuid_upper::serialize")]
    pub id: Uuid,
    #[serde(
        rename = "hostID",
        serialize_with = "crate::model::uuid_upper::serialize"
    )]
    pub host_id: Uuid,
    pub name: String,
    pub history: String,
    /// Seconds since 2001-01-01 UTC (Swift's default `Date` encoding).
    #[serde(rename = "savedAt")]
    pub saved_at: f64,
    #[serde(
        rename = "usesDefaultName",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub uses_default_name: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<i64>,
}

/// The decrypted vault payload.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Contents {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<HostGroup>>,
    #[serde(default)]
    pub hosts: Vec<Host>,
    #[serde(default, with = "passwords")]
    pub passwords: BTreeMap<Uuid, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sessions: Option<Vec<SavedSession>>,
}

/// Keys are host UUIDs written uppercase; any case is accepted on read.
mod passwords {
    use super::*;

    pub fn serialize<S: Serializer>(map: &BTreeMap<Uuid, String>, s: S) -> Result<S::Ok, S::Error> {
        let upper: BTreeMap<String, &String> = map.iter().map(|(k, v)| (upper(k), v)).collect();
        upper.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<BTreeMap<Uuid, String>, D::Error> {
        let raw = BTreeMap::<String, String>::deserialize(d)?;
        raw.into_iter()
            .map(|(k, v)| {
                Uuid::parse_str(&k)
                    .map(|id| (id, v))
                    .map_err(|_| serde::de::Error::custom("password key is not a UUID"))
            })
            .collect()
    }
}
