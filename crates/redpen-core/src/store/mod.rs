//! Redpen's files (docs/ARCHITECTURE.md §4): `settings.json` and `secrets.json` in the config
//! directory, `redpen.db` in the data directory.
//!
//! The stores never choose a path: the caller passes the directories (the app from Tauri's path
//! resolver, tests a temporary directory). This module is the only core code that touches the
//! filesystem, and only under the directories it was given (invariant 10). Every JSON file is
//! written atomically: a temporary file beside it, flushed, then renamed over it, so a crash
//! never leaves half a file.

pub mod db;
pub mod secrets;
pub mod settings;

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub use db::{CacheRow, Db, DbError, SentEntry, Swept, Usage, local_day};
pub use secrets::{Secrets, SecretsError};
pub use settings::{Settings, SettingsError};

/// Writes `bytes` to `path` through `<path>.tmp` and a rename. With `private`, the file is
/// readable and writable by its owner only (mode 0600) on Unix; on Windows the per-user config
/// folder's access list already keeps other users out.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8], private: bool) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = temp_path(path);
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        #[cfg(unix)]
        if private {
            // The mode above applies only to a new file; a leftover temporary keeps its own.
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
        }
        #[cfg(not(unix))]
        let _ = private;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_replace_the_file_and_leave_no_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("settings.json");
        write_atomic(&path, b"one", false).unwrap();
        write_atomic(&path, b"two", false).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        assert!(!dir.path().join("nested").join("settings.json.tmp").exists());
    }

    #[cfg(unix)]
    #[test]
    fn private_files_are_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        write_atomic(&path, b"{}", true).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
