use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

/// A single saved SSH host; maps 1:1 onto the `host` object of
/// `packages/schema/hosts.schema.json`.
///
/// `port` is stored as `i64` so an out-of-range value in a file decodes and is
/// reported by [`Host::issues`] instead of making the whole list unreadable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Host {
    #[serde(serialize_with = "super::uuid_upper::serialize")]
    pub id: Uuid,
    pub name: String,
    /// An IP, a DNS name, or a `Host` alias from `~/.ssh/config`.
    pub hostname: String,
    pub port: i64,
    /// Empty means "not specified": OpenSSH decides.
    pub username: String,
    #[serde(
        rename = "group",
        skip_serializing_if = "Option::is_none",
        serialize_with = "super::uuid_upper::serialize_opt"
    )]
    pub group_id: Option<Uuid>,
}

#[derive(Deserialize)]
struct Raw {
    id: Option<Uuid>,
    name: Option<String>,
    hostname: Option<String>,
    port: Option<i64>,
    username: Option<String>,
    group: Option<Uuid>,
}

// Lenient reading, like macOS: an absent or null field falls back to its
// default; a field of the wrong JSON type still fails.
impl<'de> Deserialize<'de> for Host {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = Raw::deserialize(d)?;
        Ok(Host {
            id: raw.id.unwrap_or_else(Uuid::new_v4),
            name: raw.name.unwrap_or_default(),
            hostname: raw.hostname.unwrap_or_default(),
            port: raw.port.unwrap_or(Host::DEFAULT_PORT),
            username: raw.username.unwrap_or_default(),
            group_id: raw.group,
        })
    }
}

impl Host {
    /// Storing 22 means "default": the `-p` flag is then left off so a matching
    /// `~/.ssh/config` stanza can pick the real port.
    pub const DEFAULT_PORT: i64 = 22;

    pub fn new(name: impl Into<String>, hostname: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            hostname: hostname.into(),
            port: Self::DEFAULT_PORT,
            username: String::new(),
            group_id: None,
        }
    }

    pub fn display_name(&self) -> String {
        let name = self.name.trim();
        if name.is_empty() {
            self.hostname.clone()
        } else {
            name.to_string()
        }
    }

    /// `user@hostname`, or just `hostname`.
    pub fn target(&self) -> String {
        if self.username.is_empty() {
            self.hostname.clone()
        } else {
            format!("{}@{}", self.username, self.hostname)
        }
    }

    /// One-line summary: target, plus `:port` when not the default.
    pub fn subtitle(&self) -> String {
        let mut line = self.target();
        if self.port != Self::DEFAULT_PORT {
            line.push_str(&format!(":{}", self.port));
        }
        line
    }

    /// Problems that stop this host from being used (same rules and wording as macOS).
    pub fn issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if self.name.trim().is_empty() {
            issues.push("A name is required.".to_string());
        }
        let address = self.hostname.trim();
        if address.is_empty() {
            issues.push("A hostname or IP address is required.".to_string());
        } else if address.chars().any(char::is_whitespace) {
            issues.push("The hostname cannot contain spaces.".to_string());
        } else if address.starts_with('-') {
            issues.push(
                "The hostname cannot start with \"-\", it would be read as an option by ssh."
                    .to_string(),
            );
        } else if address.chars().count() > 253 {
            issues.push("The hostname cannot be longer than 253 characters.".to_string());
        }
        if !(1..=65535).contains(&self.port) {
            issues.push("The port must be between 1 and 65535.".to_string());
        }
        if self.username.chars().any(char::is_whitespace) {
            issues.push("The username cannot contain spaces.".to_string());
        } else if self.username.chars().count() > 128 {
            issues.push("The username cannot be longer than 128 characters.".to_string());
        }
        issues
    }

    pub fn is_valid(&self) -> bool {
        self.issues().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> Host {
        Host::new("Web", "example.com")
    }

    #[test]
    fn valid_host_has_no_issues() {
        assert!(host().is_valid());
    }

    #[test]
    fn blank_name_is_reported() {
        let mut h = host();
        h.name = "  ".into();
        assert_eq!(h.issues(), vec!["A name is required."]);
    }

    #[test]
    fn hostname_rules() {
        let mut h = host();
        h.hostname = "a b".into();
        assert_eq!(h.issues(), vec!["The hostname cannot contain spaces."]);
        h.hostname = "-oProxyCommand=x".into();
        assert_eq!(
            h.issues(),
            vec!["The hostname cannot start with \"-\", it would be read as an option by ssh."]
        );
        h.hostname = "a".repeat(254);
        assert_eq!(
            h.issues(),
            vec!["The hostname cannot be longer than 253 characters."]
        );
        h.hostname = "".into();
        assert_eq!(h.issues(), vec!["A hostname or IP address is required."]);
    }

    #[test]
    fn port_bounds() {
        let mut h = host();
        for bad in [0, 65536, -1] {
            h.port = bad;
            assert_eq!(h.issues(), vec!["The port must be between 1 and 65535."]);
        }
        h.port = 65535;
        assert!(h.is_valid());
    }

    #[test]
    fn username_rules() {
        let mut h = host();
        h.username = "a b".into();
        assert_eq!(h.issues(), vec!["The username cannot contain spaces."]);
        h.username = "u".repeat(129);
        assert_eq!(
            h.issues(),
            vec!["The username cannot be longer than 128 characters."]
        );
    }

    #[test]
    fn serializes_schema_keys_and_omits_group_when_none() {
        let v = serde_json::to_value(host()).unwrap();
        let o = v.as_object().unwrap();
        for k in ["id", "name", "hostname", "port", "username"] {
            assert!(o.contains_key(k), "{k}");
        }
        assert!(!o.contains_key("group"));
        let id = o["id"].as_str().unwrap();
        assert_eq!(id, id.to_uppercase());
    }

    #[test]
    fn missing_and_null_fields_use_defaults() {
        let h: Host = serde_json::from_str(r#"{"id":"0F8FAD5B-D9CB-469F-A165-70867728950E","name":"Web","hostname":"x","port":null}"#).unwrap();
        assert_eq!(h.port, 22);
        assert_eq!(h.username, "");
        assert_eq!(h.group_id, None);
    }

    #[test]
    fn wrong_type_fails() {
        assert!(serde_json::from_str::<Host>(r#"{"port":"22"}"#).is_err());
    }

    #[test]
    fn subtitle_and_display_name() {
        let mut h = host();
        h.username = "deploy".into();
        h.port = 2222;
        assert_eq!(h.subtitle(), "deploy@example.com:2222");
        h.name = " ".into();
        assert_eq!(h.display_name(), "example.com");
    }
}
