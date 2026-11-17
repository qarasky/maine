//! Secret Service (KWallet, GNOME Keyring) access for sync tokens and, later,
//! MCP client token hashes. Only secrets that must survive a restart live here;
//! the vault key never does.

use maine_sync::{SyncError, TokenStore};
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

/// A lazily connected keyring. Connection errors surface on use, so the app
/// still runs (without sync) when no Secret Service is available.
pub struct SecretService {
    keyring: Mutex<Option<oo7::Keyring>>,
}

impl SecretService {
    pub const fn new() -> Self {
        Self {
            keyring: Mutex::new(None),
        }
    }

    fn with<R>(
        &self,
        f: impl AsyncFnOnce(&oo7::Keyring) -> Result<R, oo7::Error>,
    ) -> Result<R, String> {
        let mut guard = self.keyring.lock().unwrap_or_else(|e| e.into_inner());
        runtime().block_on(async {
            if guard.is_none() {
                *guard = Some(oo7::Keyring::new().await.map_err(|e| e.to_string())?);
            }
            let keyring = guard.as_ref().expect("connected above");
            keyring.unlock().await.map_err(|e| e.to_string())?;
            f(keyring).await.map_err(|e| e.to_string())
        })
    }

    pub fn get(&self, service: &str, account: &str) -> Result<Option<String>, String> {
        self.with(async |k| {
            let items = k
                .search_items(&[
                    ("application", "maine"),
                    ("service", service),
                    ("account", account),
                ])
                .await?;
            match items.first() {
                Some(item) => Ok(Some(
                    String::from_utf8_lossy(item.secret().await?.as_bytes()).into_owned(),
                )),
                None => Ok(None),
            }
        })
    }

    pub fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), String> {
        self.with(async |k| {
            k.create_item(
                &format!("Maine {service}"),
                &[
                    ("application", "maine"),
                    ("service", service),
                    ("account", account),
                ],
                secret.as_bytes(),
                true,
            )
            .await?;
            Ok(())
        })
    }

    /// Every account stored under `service`.
    pub fn list(&self, service: &str) -> Result<Vec<String>, String> {
        self.with(async |k| {
            let items = k
                .search_items(&[("application", "maine"), ("service", service)])
                .await?;
            let mut accounts = Vec::new();
            for item in items {
                if let Some(account) = item.attributes().await?.get("account") {
                    accounts.push(account.to_string());
                }
            }
            Ok(accounts)
        })
    }

    pub fn delete(&self, service: &str, account: &str) -> Result<(), String> {
        self.with(async |k| {
            k.delete(&[
                ("application", "maine"),
                ("service", service),
                ("account", account),
            ])
            .await
        })
    }
}

/// The process-wide keyring.
pub fn service() -> &'static SecretService {
    static SERVICE: SecretService = SecretService::new();
    &SERVICE
}

/// Agent client token hashes in the Secret Service.
pub struct AgentSecrets;

impl maine_agents::SecretStore for AgentSecrets {
    fn get(&self, name: &str, key: &str) -> Result<Option<Vec<u8>>, maine_agents::AgentError> {
        service_get(name, key)
    }

    fn set(&self, name: &str, key: &str, value: &[u8]) -> Result<(), maine_agents::AgentError> {
        // Stored as text: the store only holds hashes and JSON, never raw bytes.
        let text = String::from_utf8_lossy(value).into_owned();
        service().set(name, key, &text).map_err(secret_error)
    }

    fn delete(&self, name: &str, key: &str) -> Result<(), maine_agents::AgentError> {
        match service().delete(name, key) {
            Ok(()) => Ok(()),
            Err(m)
                if m.to_lowercase().contains("not found")
                    || m.to_lowercase().contains("no such") =>
            {
                Ok(())
            }
            Err(m) => Err(secret_error(m)),
        }
    }

    fn list(&self, name: &str) -> Result<Vec<String>, maine_agents::AgentError> {
        service_list(name)
    }
}

fn service_get(name: &str, key: &str) -> Result<Option<Vec<u8>>, maine_agents::AgentError> {
    service()
        .get(name, key)
        .map(|o| o.map(String::into_bytes))
        .map_err(secret_error)
}

fn service_list(name: &str) -> Result<Vec<String>, maine_agents::AgentError> {
    service().list(name).map_err(secret_error)
}

fn secret_error(message: String) -> maine_agents::AgentError {
    maine_agents::AgentError::new(message)
}

/// Sync tokens in the Secret Service, one item per profile.
pub struct SyncTokens;

const SYNC_SERVICE: &str = "Maine Sync";

impl TokenStore for SyncTokens {
    fn get(&self, profile: Uuid) -> Result<Option<String>, SyncError> {
        service()
            .get(SYNC_SERVICE, &profile.to_string())
            .map_err(SyncError::TokenStore)
    }

    fn set(&self, profile: Uuid, token: &str) -> Result<(), SyncError> {
        service()
            .set(SYNC_SERVICE, &profile.to_string(), token)
            .map_err(SyncError::TokenStore)
    }

    fn delete(&self, profile: Uuid) -> Result<(), SyncError> {
        match service().delete(SYNC_SERVICE, &profile.to_string()) {
            Ok(()) => Ok(()),
            // Deleting a token that is not there is not an error.
            Err(message)
                if message.to_lowercase().contains("not found")
                    || message.to_lowercase().contains("no such") =>
            {
                Ok(())
            }
            Err(message) => Err(SyncError::TokenStore(message)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs a running Secret Service (KWallet or GNOME Keyring):
    /// `cargo test -p maine-app secret_service -- --ignored`. Uses a throwaway item and removes it.
    #[test]
    #[ignore]
    fn secret_service_set_get_delete_roundtrip() {
        let svc = service();
        let account = format!("test-{}", Uuid::new_v4());
        assert_eq!(svc.get("Maine Test", &account).unwrap(), None);
        svc.set("Maine Test", &account, "pässword-секрет").unwrap();
        assert_eq!(
            svc.get("Maine Test", &account).unwrap().as_deref(),
            Some("pässword-секрет")
        );
        svc.set("Maine Test", &account, "replaced").unwrap();
        assert_eq!(
            svc.get("Maine Test", &account).unwrap().as_deref(),
            Some("replaced")
        );
        svc.delete("Maine Test", &account).unwrap();
        assert_eq!(svc.get("Maine Test", &account).unwrap(), None);
    }

    #[test]
    #[ignore]
    fn sync_token_store_delete_of_a_missing_token_is_not_an_error() {
        SyncTokens.delete(Uuid::new_v4()).unwrap();
    }
}

#[cfg(test)]
mod cleanup {
    use super::*;

    /// Developer tool: removes every Maine item (sync tokens, agent client records) left by manual end-to-end runs.
    /// `cargo test -p maine-app remove_all_maine_items -- --ignored`
    #[test]
    #[ignore]
    fn remove_all_maine_items() {
        let removed = service()
            .with(async |k| {
                let items = k.search_items(&[("application", "maine")]).await?;
                let n = items.len();
                for item in items {
                    item.delete().await?;
                }
                Ok(n)
            })
            .unwrap();
        println!("removed {removed} items");
    }
}

#[cfg(test)]
mod list_items {
    use super::*;

    /// Developer tool: prints the Maine items in the keyring (service/account only, never secrets).
    #[test]
    #[ignore]
    fn list_maine_items() {
        let n = service()
            .with(async |k| {
                let items = k.search_items(&[("application", "maine")]).await?;
                for item in &items {
                    let a = item.attributes().await?;
                    println!(
                        "item service={:?} account={:?}",
                        a.get("service"),
                        a.get("account")
                    );
                }
                Ok(items.len())
            })
            .unwrap();
        println!("{n} items");
    }
}
