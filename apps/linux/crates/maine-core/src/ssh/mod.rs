//! The exact `ssh` invocation, and a read-only view of `~/.ssh/config`.

mod config_file;
mod target;

pub use config_file::{SshConfigEntry, SshConfigFile};
pub use target::{terminal_environment, Resolution, SshTarget, SSH_PATH};
