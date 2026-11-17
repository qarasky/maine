//! Swift writes UUIDs uppercase and the vault AAD and password keys depend on
//! that spelling, so every client writes uppercase and reads either case.
use serde::Serializer;
use uuid::Uuid;

pub fn serialize<S: Serializer>(id: &Uuid, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&upper(id))
}

pub fn serialize_opt<S: Serializer>(id: &Option<Uuid>, s: S) -> Result<S::Ok, S::Error> {
    match id {
        Some(id) => s.serialize_str(&upper(id)),
        None => s.serialize_none(),
    }
}

pub fn upper(id: &Uuid) -> String {
    id.hyphenated().to_string().to_uppercase()
}
