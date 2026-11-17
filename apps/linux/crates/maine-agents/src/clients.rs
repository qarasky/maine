//! Paired MCP clients. A client's token is shown once, at pairing, and lives in
//! the client's MCP configuration afterwards; only its SHA-256 is stored, behind
//! a [`SecretStore`] (the app backs it with the Secret Service). Verification
//! compares hashes in constant time. Tokens are never logged.

use crate::apple::AppleDate;
use crate::error::AgentError;
use crate::json::uuid_text;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// A place for small secrets, addressed like a keyring: `(service, key)`.
pub trait SecretStore: Send + Sync {
    fn get(&self, service: &str, key: &str) -> Result<Option<Vec<u8>>, AgentError>;
    fn set(&self, service: &str, key: &str, value: &[u8]) -> Result<(), AgentError>;
    /// Deleting something that is not there succeeds.
    fn delete(&self, service: &str, key: &str) -> Result<(), AgentError>;
    /// Every key stored under `service`.
    fn list(&self, service: &str) -> Result<Vec<String>, AgentError>;
}

/// A [`SecretStore`] in memory, for tests and for sessions without a keyring.
#[derive(Debug, Default)]
pub struct MemorySecretStore {
    items: Mutex<BTreeMap<(String, String), Vec<u8>>>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// The raw stored bytes, for tests that check what was written.
    pub fn raw(&self, service: &str, key: &str) -> Option<Vec<u8>> {
        self.items
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(service.to_string(), key.to_string()))
            .cloned()
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, service: &str, key: &str) -> Result<Option<Vec<u8>>, AgentError> {
        Ok(self.raw(service, key))
    }

    fn set(&self, service: &str, key: &str, value: &[u8]) -> Result<(), AgentError> {
        self.items
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((service.to_string(), key.to_string()), value.to_vec());
        Ok(())
    }

    fn delete(&self, service: &str, key: &str) -> Result<(), AgentError> {
        self.items
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&(service.to_string(), key.to_string()));
        Ok(())
    }

    fn list(&self, service: &str) -> Result<Vec<String>, AgentError> {
        let items = self.items.lock().unwrap_or_else(|e| e.into_inner());
        Ok(items
            .keys()
            .filter(|(s, _)| s == service)
            .map(|(_, k)| k.clone())
            .collect())
    }
}

/// A paired MCP client. Its name is what the user typed in Settings.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentClient {
    pub id: Uuid,
    pub name: String,
    pub created_at: AppleDate,
}

/// What Settings shows once, right after pairing. Its `Debug` output never
/// shows the token.
#[derive(Clone, PartialEq)]
pub struct Pairing {
    pub client: AgentClient,
    pub token: String,
    pub claude_code_command: String,
    pub desktop_json: String,
}

impl std::fmt::Debug for Pairing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pairing")
            .field("client", &self.client)
            .finish_non_exhaustive()
    }
}

#[derive(Serialize, Deserialize)]
struct Record {
    name: String,
    created_at: AppleDate,
    /// Lowercase hex SHA-256 of the token.
    hash: String,
}

/// Paired clients behind a [`SecretStore`] (service `Maine Agents`, key = the
/// client's UUID). Only SHA-256 of a token is stored.
pub struct AgentClientStore {
    secrets: Arc<dyn SecretStore>,
    service: String,
}

impl AgentClientStore {
    pub const TOKEN_PREFIX: &'static str = "maine-agent-";
    pub const DEFAULT_SERVICE: &'static str = "Maine Agents";
    const ALPHABET: &'static [u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

    pub fn new(secrets: Arc<dyn SecretStore>) -> Self {
        Self::with_service(secrets, Self::DEFAULT_SERVICE)
    }

    pub fn with_service(secrets: Arc<dyn SecretStore>, service: impl Into<String>) -> Self {
        Self {
            secrets,
            service: service.into(),
        }
    }

    /// Paired clients, oldest first. Items that are not ours or are damaged are skipped.
    pub fn clients(&self) -> Result<Vec<AgentClient>, AgentError> {
        Ok(self
            .records()?
            .into_iter()
            .map(|(client, _)| client)
            .collect())
    }

    /// Pairs a new client: returns it with its token, which is not stored.
    pub fn add(&self, name: &str) -> Result<(AgentClient, String), AgentError> {
        let token = Self::make_token();
        let client = AgentClient {
            id: Uuid::new_v4(),
            name: name.to_string(),
            created_at: AppleDate::now(),
        };
        let record = Record {
            name: client.name.clone(),
            created_at: client.created_at,
            hash: Self::hash(&token),
        };
        let bytes = serde_json::to_vec(&record)
            .map_err(|e| AgentError::new(format!("Could not store the client: {e}")))?;
        self.secrets
            .set(&self.service, &uuid_text(&client.id), &bytes)?;
        Ok((client, token))
    }

    /// [`add`](Self::add), with the setup text for Claude Code and Claude
    /// Desktop. An empty name becomes "MCP client". `executable` is the path of
    /// the running `maine` binary.
    pub fn pair(&self, name: &str, executable: &str) -> Result<Pairing, AgentError> {
        let trimmed = name.trim();
        let (client, token) = self.add(if trimmed.is_empty() {
            "MCP client"
        } else {
            trimmed
        })?;
        let (claude_code_command, desktop_json) = pairing_snippets(&token, executable);
        Ok(Pairing {
            client,
            token,
            claude_code_command,
            desktop_json,
        })
    }

    pub fn revoke(&self, id: Uuid) -> Result<(), AgentError> {
        self.secrets.delete(&self.service, &uuid_text(&id))
    }

    /// The client a presented token belongs to, comparing hashes in constant time.
    pub fn client_for_token(&self, token: &str) -> Result<Option<AgentClient>, AgentError> {
        if !token.starts_with(Self::TOKEN_PREFIX) {
            return Ok(None);
        }
        let presented = Self::hash(token);
        let mut found = None;
        for (client, stored) in self.records()? {
            if constant_time_equal(stored.as_bytes(), presented.as_bytes()) {
                found = Some(client);
            }
        }
        Ok(found)
    }

    fn records(&self) -> Result<Vec<(AgentClient, String)>, AgentError> {
        let mut records = Vec::new();
        for key in self.secrets.list(&self.service)? {
            let Ok(id) = Uuid::parse_str(&key) else {
                continue;
            };
            let Some(bytes) = self.secrets.get(&self.service, &key)? else {
                continue;
            };
            let Ok(record) = serde_json::from_slice::<Record>(&bytes) else {
                continue;
            };
            records.push((
                AgentClient {
                    id,
                    name: record.name,
                    created_at: record.created_at,
                },
                record.hash,
            ));
        }
        records.sort_by(|a, b| {
            a.0.created_at
                .partial_cmp(&b.0.created_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(records)
    }

    /// `maine-agent-` and the 32 random bytes in Crockford base32 (`0-9A-Z`
    /// without `I`, `L`, `O`, `U`): 64 characters in all.
    pub fn make_token() -> String {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let mut output = String::from(Self::TOKEN_PREFIX);
        let (mut bits, mut value) = (0u32, 0u32);
        for byte in bytes {
            value = (value << 8 | u32::from(byte)) & 0xFFFF;
            bits += 8;
            while bits >= 5 {
                output.push(Self::ALPHABET[((value >> (bits - 5)) & 31) as usize] as char);
                bits -= 5;
            }
        }
        if bits > 0 {
            output.push(Self::ALPHABET[((value << (5 - bits)) & 31) as usize] as char);
        }
        output
    }

    /// Lowercase hex SHA-256.
    pub fn hash(token: &str) -> String {
        Sha256::digest(token.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

/// The Claude Code command and the Claude Desktop JSON for a freshly paired
/// client. The executable is double-quoted for the shell.
pub fn pairing_snippets(token: &str, executable: &str) -> (String, String) {
    let quoted: String = executable
        .chars()
        .flat_map(|c| {
            if matches!(c, '"' | '\\' | '$' | '`') {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect();
    let command =
        format!("claude mcp add maine --env MAINE_AGENT_TOKEN={token} -- \"{quoted}\" --mcp");
    let desktop: Value = json!({"mcpServers": {"maine": {
        "args": ["--mcp"], "command": executable, "env": {"MAINE_AGENT_TOKEN": token},
    }}});
    (
        command,
        serde_json::to_string_pretty(&desktop).unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::ValueExt;

    fn store() -> (AgentClientStore, Arc<MemorySecretStore>) {
        let secrets = Arc::new(MemorySecretStore::new());
        (AgentClientStore::new(secrets.clone()), secrets)
    }

    #[test]
    fn tokens_are_shown_once_and_stored_hashed() {
        let (store, secrets) = store();
        let (client, token) = store.add("Claude Code").unwrap();
        assert!(token.starts_with("maine-agent-"));
        assert_eq!(token.len(), 64);
        assert_eq!(
            store.client_for_token(&token).unwrap().map(|c| c.id),
            Some(client.id)
        );
        assert_eq!(store.client_for_token(&format!("{token}x")).unwrap(), None);
        assert_eq!(store.client_for_token("").unwrap(), None);

        let stored = secrets.raw("Maine Agents", &uuid_text(&client.id)).unwrap();
        assert_ne!(stored, token.as_bytes());
        assert!(!String::from_utf8_lossy(&stored).contains(&token));
        assert!(String::from_utf8_lossy(&stored).contains(&AgentClientStore::hash(&token)));

        let names: Vec<String> = store
            .clients()
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["Claude Code"]);
        let (second, second_token) = store.add("Desktop").unwrap();
        assert_ne!(token, second_token);
        store.revoke(client.id).unwrap();
        assert_eq!(store.client_for_token(&token).unwrap(), None);
        assert_eq!(
            store.client_for_token(&second_token).unwrap().map(|c| c.id),
            Some(second.id)
        );
    }

    #[test]
    fn token_alphabet_is_crockford_without_ambiguous_letters() {
        for _ in 0..50 {
            let token = AgentClientStore::make_token();
            let body = token.strip_prefix("maine-agent-").unwrap();
            assert_eq!(body.len(), 52);
            assert!(
                body.chars()
                    .all(|c| "0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(c)),
                "{token}"
            );
        }
        assert_ne!(
            AgentClientStore::make_token(),
            AgentClientStore::make_token()
        );
    }

    #[test]
    fn hashing_is_sha256_hex() {
        // The standard SHA-256 test vector for "abc".
        assert_eq!(
            AgentClientStore::hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn revoking_an_unknown_client_is_fine_and_damaged_items_are_skipped() {
        let (store, secrets) = store();
        store.revoke(Uuid::new_v4()).unwrap();
        secrets.set("Maine Agents", "not-a-uuid", b"{}").unwrap();
        secrets
            .set("Maine Agents", &uuid_text(&Uuid::new_v4()), b"garbage")
            .unwrap();
        secrets
            .set("Other Service", &uuid_text(&Uuid::new_v4()), b"{}")
            .unwrap();
        assert!(store.clients().unwrap().is_empty());
        let (_, token) = store.add("Real").unwrap();
        assert!(store.client_for_token(&token).unwrap().is_some());
    }

    #[test]
    fn a_pairing_never_prints_its_token() {
        let (store, _) = store();
        let pairing = store.pair("x", "/usr/bin/maine").unwrap();
        assert!(!format!("{pairing:?}").contains(&pairing.token));
    }

    #[test]
    fn pairing_snippets_use_the_given_executable() {
        let (store, _) = store();
        let pairing = store.pair("  ", "/usr/bin/maine").unwrap();
        assert_eq!(pairing.client.name, "MCP client");
        assert!(pairing
            .claude_code_command
            .starts_with("claude mcp add maine --env MAINE_AGENT_TOKEN=maine-agent-"));
        assert!(pairing.claude_code_command.ends_with(" --mcp"));
        assert!(pairing.claude_code_command.contains(&pairing.token));
        assert!(pairing.claude_code_command.contains("\"/usr/bin/maine\""));
        let desktop: Value = serde_json::from_str(&pairing.desktop_json).unwrap();
        assert_eq!(
            desktop["mcpServers"]["maine"].array("args"),
            Some(&vec![json!("--mcp")])
        );
        assert_eq!(
            desktop["mcpServers"]["maine"]["env"].string("MAINE_AGENT_TOKEN"),
            Some(pairing.token.as_str())
        );
        assert_eq!(
            desktop["mcpServers"]["maine"].string("command"),
            Some("/usr/bin/maine")
        );

        let (command, _) = pairing_snippets("t", "/opt/my \"app\"/maine");
        assert!(command.contains(r#""/opt/my \"app\"/maine""#));
    }
}
