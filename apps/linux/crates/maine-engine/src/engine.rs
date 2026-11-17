use crate::askpass::{self, Broker};
use crate::paths::Paths;
use crate::sessions::{Launcher, SessionManager, SshLauncher};
use maine_core::model::{credential_text, Host, HostGroup, HostLayout};
use maine_core::ssh::{SshConfigEntry, SshConfigFile};
use maine_core::vault::{Unlocked, VaultFile};
use maine_core::{StoreError, VaultError};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineState {
    /// No vault file yet: the next step is creating one.
    NoVault,
    Locked,
    Unlocked,
}

/// The application core. Not thread-safe by itself; the app wraps it in a mutex.
pub struct Engine {
    paths: Paths,
    vault: VaultFile,
    unlocked: Option<Unlocked>,
    layout: HostLayout,
    collapsed: BTreeSet<Uuid>,
    /// Hosts with a saved password, read at `reload`; the sidebar must not decrypt the vault every refresh.
    with_password: BTreeSet<Uuid>,
    broker: Arc<Broker>,
    sessions: SessionManager,
    write_hook: Option<Arc<dyn Fn() + Send + Sync>>,
    _askpass_server: Option<askpass::Server>,
}

impl Engine {
    /// The real engine: sessions run the system `ssh`, askpass goes through the
    /// socket under the runtime directory, and `askpass_exe` is the `maine`
    /// binary ssh runs for credentials.
    pub fn new(paths: Paths, askpass_exe: PathBuf) -> Self {
        let broker = Broker::new();
        let launcher = Arc::new(SshLauncher {
            broker: broker.clone(),
            askpass_exe,
            askpass_socket: paths.askpass_socket(),
        });
        let mut engine = Self::with_launcher(paths, broker, launcher);
        engine.start_askpass_server();
        engine
    }

    /// Engine with a custom launcher and no askpass listener (tests, tools).
    pub fn with_launcher(paths: Paths, broker: Arc<Broker>, launcher: Arc<dyn Launcher>) -> Self {
        let vault = VaultFile::new(paths.vault.clone());
        Self {
            paths,
            vault,
            unlocked: None,
            layout: HostLayout::new(vec![], vec![]),
            collapsed: BTreeSet::new(),
            with_password: BTreeSet::new(),
            broker,
            sessions: SessionManager::new(launcher),
            write_hook: None,
            _askpass_server: None,
        }
    }

    /// Starts the askpass / single-instance socket (done by `new`; engines built
    /// with a custom launcher call this when they want the same behaviour).
    pub fn start_askpass_server(&mut self) {
        self._askpass_server =
            askpass::Server::start(&self.paths.askpass_socket(), self.broker.clone()).ok();
    }

    /// Called after every successful write to the synced vault file (sync schedules a push from it).
    pub fn set_write_hook(&mut self, hook: impl Fn() + Send + Sync + 'static) {
        self.write_hook = Some(Arc::new(hook));
    }

    fn wrote_vault(&self) {
        if let Some(hook) = &self.write_hook {
            hook();
        }
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    pub fn broker(&self) -> Arc<Broker> {
        self.broker.clone()
    }

    pub fn sessions(&self) -> &SessionManager {
        &self.sessions
    }

    pub fn sessions_mut(&mut self) -> &mut SessionManager {
        &mut self.sessions
    }

    pub fn vault_file(&self) -> &VaultFile {
        &self.vault
    }

    /// The key while unlocked; sync and agents need it.
    pub fn unlocked(&self) -> Option<&Unlocked> {
        self.unlocked.as_ref()
    }

    pub fn state(&self) -> EngineState {
        if self.unlocked.is_some() {
            EngineState::Unlocked
        } else if self.vault.exists() {
            EngineState::Locked
        } else {
            EngineState::NoVault
        }
    }

    // MARK: lifecycle

    /// Creates the vault and unlocks it.
    pub fn create_vault(&mut self, master: &str) -> Result<(), VaultError> {
        let unlocked = self.vault.create(master)?;
        self.adopt_unlocked(unlocked)
    }

    /// Master-password gate. The password is never stored; only the derived key
    /// lives in memory until [`Engine::lock`].
    pub fn unlock(&mut self, master: &str) -> Result<(), VaultError> {
        let unlocked = self.vault.unlock(master)?;
        self.adopt_unlocked(unlocked)
    }

    fn adopt_unlocked(&mut self, unlocked: Unlocked) -> Result<(), VaultError> {
        self.unlocked = Some(unlocked.clone());
        self.broker
            .set_vault(Some((self.vault.clone(), unlocked.clone())));
        self.sessions
            .set_vault(Some((self.vault.clone(), unlocked)));
        self.reload()?;
        // Restored history stays scrollback; sessions reconnect on their own.
        let hosts = self.layout.hosts().to_vec();
        let _ = self.sessions.restore(&hosts);
        Ok(())
    }

    /// Locks the vault. Live sessions stay connected; saved passwords stop being
    /// available (askpass fails closed) and nothing writes until unlock.
    pub fn lock(&mut self) {
        let _ = self.sessions.checkpoint();
        self.unlocked = None;
        self.broker.set_vault(None);
        self.sessions.set_vault(None);
        self.layout = HostLayout::new(vec![], vec![]);
        self.collapsed.clear();
        self.with_password.clear();
    }

    /// Quit stops processes but keeps their records.
    pub fn quit(&mut self) {
        self.sessions.shutdown();
        self.broker.set_vault(None);
    }

    /// Periodic work: the 2 s session checkpoint.
    pub fn tick(&mut self) {
        let _ = self.sessions.checkpoint();
    }

    /// Re-reads hosts and groups from the vault file (after sync adopted a
    /// remote copy) and closes sessions whose host is gone.
    pub fn reload(&mut self) -> Result<(), VaultError> {
        let unlocked = self.unlocked.as_ref().ok_or(VaultError::Locked)?;
        let contents = self.vault.contents(unlocked)?;
        self.with_password = contents
            .passwords
            .iter()
            .filter(|(_, p)| !p.is_empty())
            .map(|(id, _)| *id)
            .collect();
        self.layout = HostLayout::new(contents.groups.unwrap_or_default(), contents.hosts);
        let state = self.vault.local_state(unlocked)?;
        let known: BTreeSet<Uuid> = self.layout.groups().iter().map(|g| g.id).collect();
        self.collapsed = state
            .collapsed_groups
            .into_iter()
            .filter(|g| known.contains(g))
            .collect();
        Ok(())
    }

    pub fn adopt_remote_change(&mut self) -> Result<(), VaultError> {
        self.reload()?;
        let hosts = self.layout.hosts().to_vec();
        self.sessions.close_missing_hosts(&hosts);
        Ok(())
    }

    // MARK: hosts

    pub fn layout(&self) -> &HostLayout {
        &self.layout
    }

    pub fn hosts(&self) -> &[Host] {
        self.layout.hosts()
    }

    pub fn groups(&self) -> &[HostGroup] {
        self.layout.groups()
    }

    pub fn host(&self, id: Uuid) -> Option<&Host> {
        self.layout.hosts().iter().find(|h| h.id == id)
    }

    pub fn contains_hostname(&self, hostname: &str) -> bool {
        self.layout
            .hosts()
            .iter()
            .any(|h| h.hostname.eq_ignore_ascii_case(hostname))
    }

    fn key(&self) -> Result<&Unlocked, StoreError> {
        self.unlocked
            .as_ref()
            .ok_or_else(|| StoreError(VaultError::Locked.to_string()))
    }

    /// Adds or updates a host and its saved password in one atomic vault write.
    /// An empty password means key login: nothing is stored.
    pub fn save_host(&mut self, host: Host, password: &str) -> Result<(), StoreError> {
        if !host.is_valid() {
            return Err(StoreError(host.issues().join(" ")));
        }
        let key = self.key()?.clone();
        let (id, entry) = (host.id, host.clone());
        let password = password.to_string();
        self.vault
            .update(&key, |c| {
                match c.hosts.iter().position(|h| h.id == id) {
                    Some(i) => c.hosts[i] = entry,
                    None => c.hosts.push(entry),
                }
                if password.is_empty() {
                    c.passwords.remove(&id);
                } else {
                    c.passwords.insert(id, password);
                }
            })
            .map_err(|e| StoreError(e.to_string()))?;
        self.wrote_vault();
        self.reload().map_err(|e| StoreError(e.to_string()))
    }

    /// Removes the host, its password, its saved session records and its open sessions.
    pub fn delete_host(&mut self, id: Uuid) -> Result<(), StoreError> {
        let key = self.key()?.clone();
        self.vault
            .update(&key, |c| {
                c.hosts.retain(|h| h.id != id);
                c.passwords.remove(&id);
            })
            .map_err(|e| StoreError(e.to_string()))?;
        self.vault
            .update_local_state(&key, |l| l.sessions.retain(|s| s.host_id != id))
            .map_err(|e| StoreError(e.to_string()))?;
        self.sessions.close_sessions_for_host(id);
        self.wrote_vault();
        self.reload().map_err(|e| StoreError(e.to_string()))
    }

    /// Applies a sidebar rearrangement and writes groups and host order in one
    /// atomic vault replacement; nothing changes in memory if the write fails.
    pub fn update_layout(
        &mut self,
        change: impl FnOnce(&mut HostLayout) -> Result<(), StoreError>,
    ) -> Result<(), StoreError> {
        let key = self.key()?.clone();
        let mut next = self.layout.clone();
        change(&mut next)?;
        if next == self.layout {
            return Ok(());
        }
        let (groups, hosts) = (next.groups().to_vec(), next.hosts().to_vec());
        self.vault
            .update(&key, |c| {
                c.groups = (!groups.is_empty()).then_some(groups);
                c.hosts = hosts;
            })
            .map_err(|e| StoreError(e.to_string()))?;
        self.layout = next;
        self.wrote_vault();
        Ok(())
    }

    pub fn is_collapsed(&self, group: Uuid) -> bool {
        self.collapsed.contains(&group)
    }

    pub fn set_collapsed(&mut self, group: Uuid, collapsed: bool) -> Result<(), StoreError> {
        let key = self.key()?.clone();
        if collapsed {
            self.collapsed.insert(group);
        } else {
            self.collapsed.remove(&group);
        }
        let known: BTreeSet<Uuid> = self.layout.groups().iter().map(|g| g.id).collect();
        self.collapsed.retain(|g| known.contains(g));
        let mut ids: Vec<Uuid> = self.collapsed.iter().copied().collect();
        ids.sort_by_key(|id| id.hyphenated().to_string().to_uppercase());
        self.vault
            .update_local_state(&key, |l| l.collapsed_groups = ids)
            .map_err(|e| StoreError(e.to_string()))
    }

    /// Cheap: no decryption. True when the host has a saved password.
    pub fn has_saved_password(&self, host: Uuid) -> bool {
        self.with_password.contains(&host)
    }

    pub fn saved_password(&self, host: Uuid) -> Option<String> {
        self.broker.password(host)
    }

    /// The plain-text credential block for the clipboard.
    pub fn credential_text(&self, host: Uuid) -> Option<String> {
        let h = self.host(host)?;
        Some(credential_text(h, self.saved_password(host).as_deref()))
    }

    /// Single-host entries of `~/.ssh/config` that are not saved yet.
    pub fn ssh_config_candidates(&self, config: &SshConfigFile) -> Vec<Host> {
        config
            .entries
            .iter()
            .filter(|e| SshConfigEntry::is_connectable(&e.pattern))
            .filter(|e| !self.contains_hostname(&e.pattern))
            .filter_map(SshConfigEntry::to_host)
            .collect()
    }
}
