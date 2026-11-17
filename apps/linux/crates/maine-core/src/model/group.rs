use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A named, user-ordered sidebar section. Groups travel in the vault; whether
/// one is collapsed is device-local state and not part of this type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostGroup {
    #[serde(serialize_with = "super::uuid_upper::serialize")]
    pub id: Uuid,
    pub name: String,
}

impl HostGroup {
    pub const MAX_NAME_LENGTH: usize = 128;

    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
        }
    }
}
