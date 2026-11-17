use maine_core::model::{ConfigError, HostConfiguration};
use std::path::PathBuf;

fn example(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../packages/schema/examples")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn shared_examples_decode() {
    assert_eq!(
        HostConfiguration::from_slice(&example("hosts.empty.json"))
            .unwrap()
            .hosts
            .len(),
        0
    );
    assert!(!HostConfiguration::from_slice(&example("hosts.json"))
        .unwrap()
        .hosts
        .is_empty());
    let g = HostConfiguration::from_slice(&example("hosts.groups.json")).unwrap();
    assert_eq!(g.groups.len(), 2);
    assert_eq!(g.hosts[0].group_id, Some(g.groups[0].id));
    assert_eq!(g.hosts[1].group_id, None);
}

#[test]
fn newer_schema_version_is_refused() {
    assert!(matches!(
        HostConfiguration::from_slice(&example("hosts.invalid-version.json")),
        Err(ConfigError::UnsupportedVersion(_))
    ));
}
