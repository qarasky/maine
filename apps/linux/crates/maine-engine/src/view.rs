//! JSON snapshots of engine state for the QML layer. Pure functions so the
//! shapes are tested without Qt.

use crate::sessions::{SessionInfo, SessionState};
use crate::{Engine, EngineState};
use maine_core::model::Host;
use serde_json::{json, Value};

fn state_name(state: &SessionState) -> &'static str {
    match state {
        SessionState::Connecting => "connecting",
        SessionState::Connected => "connected",
        SessionState::Closed(_) => "closed",
        SessionState::Disconnected(_) => "disconnected",
        SessionState::Failed(_) => "failed",
    }
}

fn session_json(info: &SessionInfo) -> Value {
    json!({
        "id": info.id.to_string(),
        "hostId": info.host_id.to_string(),
        "label": info.label,
        "state": state_name(&info.state),
        "stateLabel": info.state.label(),
        "live": info.state.is_live(),
        "notice": info.state.notice(),
        "agent": info.agent.as_ref().map(|a| a.client_name.clone()),
        "handedOff": info.agent.as_ref().is_some_and(|a| a.handed_off),
        "formerAgent": info.former_agent,
        "ephemeral": info.ephemeral,
    })
}

fn host_json(host: &Host, engine: &Engine, sessions: &[SessionInfo]) -> Value {
    json!({
        "id": host.id.to_string(),
        "name": host.display_name(),
        "hostname": host.hostname,
        "port": host.port,
        "username": host.username,
        "subtitle": host.subtitle(),
        "groupId": host.group_id.map(|g| g.to_string()),
        "hasPassword": engine.has_saved_password(host.id),
        "sessions": sessions.iter().filter(|s| s.host_id == host.id).map(session_json).collect::<Vec<_>>(),
    })
}

/// Everything the sidebar and detail pane draw.
pub fn sidebar(engine: &Engine) -> Value {
    let sessions = engine.sessions().list();
    let layout = engine.layout();
    let state = match engine.state() {
        EngineState::NoVault => 0,
        EngineState::Locked => 1,
        EngineState::Unlocked => 2,
    };
    json!({
        "state": state,
        "selected": engine.sessions().selected().map(|s| s.to_string()),
        "ungrouped": layout.ungrouped().into_iter().map(|h| host_json(h, engine, &sessions)).collect::<Vec<_>>(),
        "groups": layout.groups().iter().map(|g| json!({
            "id": g.id.to_string(),
            "name": g.name,
            "collapsed": engine.is_collapsed(g.id),
            "hosts": layout.hosts_in(g.id).into_iter().map(|h| host_json(h, engine, &sessions)).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "hostCount": layout.hosts().len(),
        "sessions": sessions.iter().map(session_json).collect::<Vec<_>>(),
        "saveFailure": engine.sessions().save_failure(),
    })
}

/// A host as the editor edits it, including its saved password.
pub fn editor_host(engine: &Engine, id: uuid::Uuid) -> Value {
    match engine.host(id) {
        Some(h) => json!({
            "id": h.id.to_string(), "name": h.name, "hostname": h.hostname, "port": h.port,
            "username": h.username, "groupId": h.group_id.map(|g| g.to_string()),
            "password": engine.saved_password(id).unwrap_or_default(),
        }),
        None => Value::Null,
    }
}

/// Parses the editor's JSON back into a host and password.
pub fn parse_editor_host(text: &str) -> Result<(Host, String), String> {
    let v: Value =
        serde_json::from_str(text).map_err(|_| "The host could not be read.".to_string())?;
    let id = v["id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .unwrap_or_else(uuid::Uuid::new_v4);
    let port = match &v["port"] {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        Value::String(s) if s.trim().is_empty() => Host::DEFAULT_PORT,
        Value::String(s) => s.trim().parse().unwrap_or(0),
        _ => Host::DEFAULT_PORT,
    };
    let host = Host {
        id,
        name: v["name"].as_str().unwrap_or_default().to_string(),
        hostname: v["hostname"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_string(),
        port,
        username: v["username"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_string(),
        group_id: v["groupId"]
            .as_str()
            .and_then(|s| uuid::Uuid::parse_str(s).ok()),
    };
    Ok((host, v["password"].as_str().unwrap_or_default().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_json_parses_numbers_strings_and_blank_ports() {
        let (h, pw) = parse_editor_host(r#"{"name":"Web","hostname":" web.example ","port":"2222","username":" deploy ","password":"pw","groupId":null}"#).unwrap();
        assert_eq!(
            (
                h.name.as_str(),
                h.hostname.as_str(),
                h.port,
                h.username.as_str(),
                pw.as_str()
            ),
            ("Web", "web.example", 2222, "deploy", "pw")
        );
        assert_eq!(h.group_id, None);
        assert_eq!(
            parse_editor_host(r#"{"name":"a","hostname":"b","port":""}"#)
                .unwrap()
                .0
                .port,
            22
        );
        assert_eq!(
            parse_editor_host(r#"{"name":"a","hostname":"b","port":"x"}"#)
                .unwrap()
                .0
                .port,
            0
        );
        assert_eq!(
            parse_editor_host(r#"{"name":"a","hostname":"b","port":443}"#)
                .unwrap()
                .0
                .port,
            443
        );
        assert!(parse_editor_host("not json").is_err());
    }
}
