//! The only way sync writes files: temp file in the same directory, fsync,
//! rename, `0600`. A crash leaves either the old file or the new one.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;
use uuid::Uuid;

pub(crate) fn ensure_parent(path: &Path) -> io::Result<()> {
    if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        if !dir.exists() {
            DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        }
    }
    Ok(())
}

/// Writes `bytes` to `path` atomically with mode `0600`, replacing any file
/// already there.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    ensure_parent(path)?;
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("maine-sync");
    let tmp = dir.join(format!(".{name}.tmp-{}", Uuid::new_v4().simple()));
    let result = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        File::open(dir)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Like `write_atomic`, but never replaces: picks `stem-2.maine`, `stem-3.maine`
/// ... when the name is taken (two copies in the same second must not overwrite
/// each other). Returns the file name actually used.
pub(crate) fn write_new_copy(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<String> {
    let candidate = unique_name(dir, name);
    write_atomic(&dir.join(&candidate), bytes)?;
    Ok(candidate)
}

/// `name` if nothing in `dir` has it, else `stem-2.maine`, `stem-3.maine`...
pub(crate) fn unique_name(dir: &Path, name: &str) -> String {
    let mut candidate = name.to_string();
    let mut counter = 1;
    while dir.join(&candidate).exists() {
        counter += 1;
        candidate = match name.strip_suffix(".maine") {
            Some(stem) => format!("{stem}-{counter}.maine"),
            None => format!("{name}-{counter}"),
        };
    }
    candidate
}
