use super::{Host, HostGroup};
use serde::{Deserialize, Serialize};
use std::fmt;

/// The versioned JSON host list every Maine client shares
/// (`packages/schema/hosts.schema.json`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostConfiguration {
    pub schema_version: u32,
    pub groups: Vec<HostGroup>,
    pub hosts: Vec<Host>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// Valid JSON from a newer client; must not be loaded or overwritten.
    UnsupportedVersion(u32),
    Malformed(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(v) => write!(
                f,
                "This host list was written with a newer version of the Maine configuration format (version {v}). Update Maine to read it."
            ),
            Self::Malformed(detail) => write!(f, "Maine could not read this host list: {detail}"),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Raw {
    schema_version: u32,
    #[serde(default)]
    groups: Option<Vec<HostGroup>>,
    hosts: Vec<Host>,
}

impl HostConfiguration {
    pub const CURRENT_SCHEMA_VERSION: u32 = 2;

    pub fn new(groups: Vec<HostGroup>, hosts: Vec<Host>) -> Self {
        Self {
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            groups,
            hosts,
        }
    }

    pub fn from_slice(data: &[u8]) -> Result<Self, ConfigError> {
        let mut de = serde_json::Deserializer::from_slice(data);
        let raw: Raw = serde_path_to_error::deserialize(&mut de).map_err(|e| {
            if e.inner().is_syntax() || e.inner().is_eof() {
                ConfigError::Malformed("the file is not valid JSON.".into())
            } else {
                let path = e.path().to_string();
                let path = if path == "." {
                    "the host list".to_string()
                } else {
                    path
                };
                ConfigError::Malformed(format!("\"{path}\" has the wrong type or is missing."))
            }
        })?;
        if raw.schema_version > Self::CURRENT_SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedVersion(raw.schema_version));
        }
        Ok(Self {
            schema_version: raw.schema_version,
            groups: raw.groups.unwrap_or_default(),
            hosts: raw.hosts,
        })
    }

    /// Pretty JSON, stable key order, for the file on disk.
    pub fn to_vec(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(self).expect("host configuration serializes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn host() -> Host {
        let mut h = Host::new("Web server", "203.0.113.10");
        h.id = Uuid::parse_str("0F8FAD5B-D9CB-469F-A165-70867728950E").unwrap();
        h
    }

    #[test]
    fn round_trip_keeps_every_field() {
        let c = HostConfiguration::new(vec![], vec![host()]);
        let back = HostConfiguration::from_slice(&c.to_vec()).unwrap();
        assert_eq!(back.schema_version, 2);
        assert_eq!(back.hosts, vec![host()]);
    }

    #[test]
    fn encoded_file_has_schema_fields() {
        let v: serde_json::Value =
            serde_json::from_slice(&HostConfiguration::new(vec![], vec![host()]).to_vec()).unwrap();
        assert_eq!(v["schemaVersion"], 2);
        let entry = &v["hosts"][0];
        for k in ["id", "name", "hostname", "port", "username"] {
            assert!(!entry[k].is_null(), "{k}");
        }
        assert_eq!(entry["port"], 22);
        assert!(entry.get("group").is_none());
    }

    #[test]
    fn groups_round_trip_and_hosts_reference_them_by_id() {
        let group = HostGroup::new("Production");
        let mut grouped = host();
        grouped.group_id = Some(group.id);
        let c = HostConfiguration::new(vec![group.clone()], vec![grouped.clone()]);
        let v: serde_json::Value = serde_json::from_slice(&c.to_vec()).unwrap();
        assert_eq!(v["hosts"][0]["group"], group.id.to_string().to_uppercase());
        assert_eq!(v["groups"][0]["name"], "Production");
        let back = HostConfiguration::from_slice(&c.to_vec()).unwrap();
        assert_eq!(back.groups, vec![group]);
        assert_eq!(back.hosts, vec![grouped]);
    }

    #[test]
    fn version_one_files_load_without_groups() {
        let data = br#"{"schemaVersion":1,"hosts":[{"id":"0F8FAD5B-D9CB-469F-A165-70867728950E","name":"Web","hostname":"example.com","port":22,"username":""}]}"#;
        let c = HostConfiguration::from_slice(data).unwrap();
        assert!(c.groups.is_empty());
        assert_eq!(c.hosts[0].group_id, None);
    }

    #[test]
    fn missing_optionals_fall_back_to_defaults() {
        let data = br#"{"schemaVersion":1,"hosts":[{"id":"0F8FAD5B-D9CB-469F-A165-70867728950E","name":"Web","hostname":"example.com"}]}"#;
        let c = HostConfiguration::from_slice(data).unwrap();
        assert_eq!(c.hosts[0].port, 22);
        assert_eq!(c.hosts[0].username, "");
    }

    #[test]
    fn empty_list_is_valid() {
        let c = HostConfiguration::from_slice(&HostConfiguration::new(vec![], vec![]).to_vec())
            .unwrap();
        assert!(c.hosts.is_empty());
    }

    #[test]
    fn future_schema_version_is_refused() {
        assert_eq!(
            HostConfiguration::from_slice(br#"{"schemaVersion":3,"hosts":[]}"#),
            Err(ConfigError::UnsupportedVersion(3))
        );
    }

    #[test]
    fn wrong_field_type_is_refused_with_a_readable_message() {
        let data = br#"{"schemaVersion":1,"hosts":[{"id":"0F8FAD5B-D9CB-469F-A165-70867728950E","name":"Web","hostname":"example.com","port":"22"}]}"#;
        match HostConfiguration::from_slice(data) {
            Err(ConfigError::Malformed(detail)) => assert!(detail.contains("port"), "{detail}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn invalid_json_is_refused() {
        assert!(matches!(
            HostConfiguration::from_slice(b"not json"),
            Err(ConfigError::Malformed(_))
        ));
    }
}
