//! Which hosts MCP agents may use on this device. A host is allowed when it is
//! listed itself or its group is listed; allowed means fully usable. There are
//! no modes. The app stores this per vault in the sealed device-local state
//! (never in the vault, so it never syncs) as an opaque JSON value: use
//! [`AgentAccess::to_value`] and [`AgentAccess::from_value`], whose shape is
//! the Swift `Codable` one: `{"groups": ["UUID", ...], "hosts": ["UUID", ...]}`.

use crate::json::uuid_text;
use maine_core::model::{Host, HostLayout};
use serde::de::Deserializer;
use serde::ser::{SerializeStruct, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentAccess {
    /// Every host in these groups is allowed.
    pub groups: BTreeSet<Uuid>,
    /// Individually allowed hosts (grouped or not).
    pub hosts: BTreeSet<Uuid>,
}

impl AgentAccess {
    pub fn new(
        groups: impl IntoIterator<Item = Uuid>,
        hosts: impl IntoIterator<Item = Uuid>,
    ) -> Self {
        Self {
            groups: groups.into_iter().collect(),
            hosts: hosts.into_iter().collect(),
        }
    }

    pub fn allows(&self, host: &Host) -> bool {
        self.hosts.contains(&host.id)
            || host
                .group_id
                .is_some_and(|group| self.groups.contains(&group))
    }

    /// Allowed hosts in sidebar order.
    pub fn allowed_hosts<'a>(&self, layout: &'a HostLayout) -> Vec<&'a Host> {
        let mut allowed: Vec<&Host> = layout
            .ungrouped()
            .into_iter()
            .filter(|host| self.allows(host))
            .collect();
        for group in layout.groups() {
            allowed.extend(
                layout
                    .hosts_in(group.id)
                    .into_iter()
                    .filter(|host| self.allows(host)),
            );
        }
        allowed
    }

    /// Drops rules naming groups or hosts the layout no longer has.
    pub fn pruned(&self, layout: &HostLayout) -> Self {
        let groups: BTreeSet<Uuid> = layout.groups().iter().map(|group| group.id).collect();
        let hosts: BTreeSet<Uuid> = layout.hosts().iter().map(|host| host.id).collect();
        Self {
            groups: self.groups.intersection(&groups).copied().collect(),
            hosts: self.hosts.intersection(&hosts).copied().collect(),
        }
    }

    /// The JSON value the app stores in the sealed local state.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn from_value(value: &Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value(value.clone())
    }
}

impl Serialize for AgentAccess {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let spell = |ids: &BTreeSet<Uuid>| -> Vec<String> {
            let mut text: Vec<String> = ids.iter().map(uuid_text).collect();
            text.sort();
            text
        };
        let mut state = serializer.serialize_struct("AgentAccess", 2)?;
        state.serialize_field("groups", &spell(&self.groups))?;
        state.serialize_field("hosts", &spell(&self.hosts))?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for AgentAccess {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            #[serde(default)]
            groups: Vec<Uuid>,
            #[serde(default)]
            hosts: Vec<Uuid>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            groups: wire.groups.into_iter().collect(),
            hosts: wire.hosts.into_iter().collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maine_core::model::HostGroup;
    use serde_json::json;

    struct Fixture {
        prod: HostGroup,
        lab: HostGroup,
        layout: HostLayout,
        web: Host,
        db: Host,
        loose: Host,
    }

    fn fixture() -> Fixture {
        let prod = HostGroup::new("Prod");
        let lab = HostGroup::new("Lab");
        let mut loose = Host::new("Loose", "loose.example");
        let mut web = Host::new("Web", "web.example");
        web.group_id = Some(prod.id);
        let mut db = Host::new("DB", "db.example");
        db.group_id = Some(lab.id);
        loose.group_id = None;
        let layout = HostLayout::new(
            vec![prod.clone(), lab.clone()],
            vec![loose.clone(), web.clone(), db.clone()],
        );
        Fixture {
            prod,
            lab,
            layout,
            web,
            db,
            loose,
        }
    }

    #[test]
    fn a_host_is_allowed_by_its_group_or_by_itself() {
        let f = fixture();
        let by_group = AgentAccess::new([f.prod.id], []);
        assert!(by_group.allows(&f.web));
        assert!(!by_group.allows(&f.db));
        assert!(!by_group.allows(&f.loose));
        assert!(AgentAccess::new([], [f.loose.id]).allows(&f.loose));
        let names: Vec<&str> = AgentAccess::new([f.prod.id], [f.loose.id])
            .allowed_hosts(&f.layout)
            .into_iter()
            .map(|h| h.name.as_str())
            .collect();
        assert_eq!(names, ["Loose", "Web"]);
        assert!(AgentAccess::default().allowed_hosts(&f.layout).is_empty());
        let _ = &f.lab;
    }

    #[test]
    fn pruning_drops_rules_for_gone_groups_and_hosts() {
        let f = fixture();
        let stale = AgentAccess::new([f.prod.id, Uuid::new_v4()], [f.web.id, Uuid::new_v4()]);
        assert_eq!(
            stale.pruned(&f.layout),
            AgentAccess::new([f.prod.id], [f.web.id])
        );
    }

    // The Swift test of the third case stores the rules in the sealed local
    // state file; that file belongs to the app. What this crate owns is the
    // JSON shape, which must stay readable by macOS.

    #[test]
    fn json_matches_the_swift_codable_shape() {
        let group = Uuid::parse_str("0f8fad5b-d9cb-469f-a165-70867728950e").unwrap();
        let host = Uuid::parse_str("7c9e6679-7425-40de-944b-e07fc1f90ae7").unwrap();
        let access = AgentAccess::new([group], [host]);
        assert_eq!(
            access.to_value(),
            json!({"groups": ["0F8FAD5B-D9CB-469F-A165-70867728950E"], "hosts": ["7C9E6679-7425-40DE-944B-E07FC1F90AE7"]})
        );
        assert_eq!(AgentAccess::from_value(&access.to_value()).unwrap(), access);
        // Swift may hand back lowercase or missing members.
        let read =
            AgentAccess::from_value(&json!({"groups": ["0f8fad5b-d9cb-469f-a165-70867728950e"]}))
                .unwrap();
        assert_eq!(read, AgentAccess::new([group], []));
        assert!(AgentAccess::from_value(&json!({"groups": [5]})).is_err());
        assert_eq!(
            AgentAccess::default().to_value(),
            json!({"groups": [], "hosts": []})
        );
    }

    #[test]
    fn rules_never_touch_the_hosts() {
        // Access is data about ids; pruning keeps the original untouched.
        let f = fixture();
        let access = AgentAccess::new([f.prod.id], [f.db.id]);
        let pruned = access.pruned(&HostLayout::new(vec![], vec![]));
        assert_eq!(pruned, AgentAccess::default());
        assert_eq!(access.groups.len(), 1);
    }
}
