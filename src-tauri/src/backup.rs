//! Backups of everything the app keeps on this PC: the history file and the
//! window's settings. The saved sign-in is left out on purpose.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::store::Store;

pub const HISTORY_FILE: &str = "history.sqlite";
pub const SETTINGS_FILE: &str = "settings.json";
const ABOUT_FILE: &str = "about this backup.txt";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub name: String,
    pub path: String,
    pub bytes: u64,
}

fn io(e: std::io::Error) -> AppError {
    AppError::Storage(e.to_string())
}

fn info(folder: &Path) -> Option<BackupInfo> {
    let bytes = fs::metadata(folder.join(HISTORY_FILE)).ok()?.len();
    Some(BackupInfo {
        name: folder.file_name()?.to_string_lossy().to_string(),
        path: folder.display().to_string(),
        bytes,
    })
}

/// Makes a new backup folder under `root` and returns what was written.
pub fn create(root: &Path, store: &Store, settings_json: &str, label: &str) -> AppResult<BackupInfo> {
    let stamp = chrono::Local::now().format("%Y-%m-%d %H.%M.%S");
    let mut folder = root.join(format!("{label} {stamp}"));
    let mut n = 2;
    while folder.exists() {
        folder = root.join(format!("{label} {stamp} ({n})"));
        n += 1;
    }
    fs::create_dir_all(&folder).map_err(io)?;
    // A backup that could not be finished is taken away again, so a full disk does not
    // collect half-written copies.
    let written = write_into(&folder, store, settings_json);
    if written.is_err() {
        let _ = fs::remove_dir_all(&folder);
    }
    written
}

fn write_into(folder: &Path, store: &Store, settings_json: &str) -> AppResult<BackupInfo> {
    store.copy_to(&folder.join(HISTORY_FILE))?;
    fs::write(folder.join(SETTINGS_FILE), settings_json).map_err(io)?;
    let about = "Anonymous VRX backup.\r\n\r\n\
history.sqlite: friends, activity feed, world history, handled notifications, cached world names.\r\n\
settings.json: size, rows, notification switches, what the close button does.\r\n\r\n\
The VRChat sign-in is not in here. After a restore on another PC, sign in again.\r\n\
To restore: Settings, Your data, pick this backup, Restore.\r\n";
    fs::write(folder.join(ABOUT_FILE), about).map_err(io)?;
    info(folder).ok_or_else(|| AppError::Storage("the backup was not written".into()))
}

/// Backups found under `root`, newest first.
pub fn list(root: &Path) -> Vec<BackupInfo> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<(std::time::SystemTime, BackupInfo)> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let made = e.metadata().and_then(|m| m.modified()).ok()?;
            Some((made, info(&e.path())?))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, i)| i).collect()
}

/// The label automatic backups carry. Only folders with it are ever cleared away.
pub const AUTO_LABEL: &str = "automatic backup";

/// The automatic backups under `root` with the time each was made, newest first.
fn automatic(root: &Path) -> Vec<(std::time::SystemTime, PathBuf)> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        // A real one of ours: the right name, and the note this app writes into every backup.
        .filter(|e| e.file_name().to_string_lossy().starts_with(&format!("{AUTO_LABEL} ")) && e.path().join(ABOUT_FILE).is_file())
        .filter_map(|e| Some((e.metadata().and_then(|m| m.modified()).ok()?, e.path())))
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found
}

/// True when there is no automatic backup yet, or the newest is older than `every`.
pub fn auto_due(root: &Path, every: std::time::Duration) -> bool {
    match automatic(root).first() {
        // A backup dated in the future means the PC's clock was set back; that is not "due".
        Some((made, _)) => made.elapsed().map(|age| age >= every).unwrap_or(false),
        None => true,
    }
}

/// Keeps the newest `keep` automatic backups and removes the older ones. Backups made by
/// hand, and anything else in the folder, are never touched. Returns how many were removed.
pub fn prune_auto(root: &Path, keep: usize) -> usize {
    let mut removed = 0;
    for (_, folder) in automatic(root).into_iter().skip(keep.max(1)) {
        if fs::remove_dir_all(&folder).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Where a restored history waits until the next start.
pub fn pending_path(live: &Path) -> PathBuf {
    let mut name = live.file_name().unwrap_or_default().to_os_string();
    name.push(".restore");
    live.with_file_name(name)
}

/// Checks a backup and puts its history next to the live file, to be swapped in
/// at the next start. Returns the backup's settings text.
pub fn stage_restore(root: &Path, name: &str, live: &Path) -> AppResult<String> {
    if name.is_empty() || name.contains(['/', '\\', ':']) || name.contains("..") {
        return Err(AppError::CannotDo("That is not a backup name.".into()));
    }
    let folder = root.join(name);
    let source = folder.join(HISTORY_FILE);
    if !source.is_file() {
        return Err(AppError::CannotDo("That backup has no history file in it.".into()));
    }
    // Opening it proves it is a readable history file before anything is replaced.
    Store::open_existing(&source)
        .map_err(|_| AppError::CannotDo("That backup's history file cannot be read.".into()))?;
    fs::copy(&source, pending_path(live)).map_err(io)?;
    Ok(fs::read_to_string(folder.join(SETTINGS_FILE)).unwrap_or_default())
}

/// Swaps in a staged restore, if there is one. Called before the history file is opened.
pub fn finish_restore(live: &Path) -> AppResult<bool> {
    let pending = pending_path(live);
    if !pending.is_file() {
        return Ok(false);
    }
    for suffix in ["", "-wal", "-shm"] {
        let mut name = live.file_name().unwrap_or_default().to_os_string();
        name.push(suffix);
        let old = live.with_file_name(name);
        if old.exists() {
            fs::remove_file(&old).map_err(io)?;
        }
    }
    fs::rename(&pending, live).map_err(io)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vrchat::model::World;

    /// A second small history file to back up, so the live one stays closed for the restore.
    fn store_for_auto(dir: &Path) -> Store {
        Store::open(&dir.join("history auto.sqlite")).unwrap()
    }

    fn world(name: &str) -> World {
        World { id: "wrld_a".into(), name: name.into(), image: String::new() }
    }

    #[test]
    fn backup_then_restore_brings_old_data_back() {
        let dir = std::env::temp_dir().join(format!("anonymous-vrx-backup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let root = dir.join("backups");
        let live = dir.join("history usr_1.sqlite");
        fs::create_dir_all(&dir).unwrap();

        let store = Store::open(&live).unwrap();
        store.save_world(&world("Before")).unwrap();
        let made = create(&root, &store, "{\"rows\":\"tight\"}", "backup").unwrap();
        assert!(made.bytes > 0);
        assert!(made.name.starts_with("backup "));
        store.save_world(&world("After")).unwrap();
        assert_eq!(list(&root).len(), 1);

        // Bad names never reach the disk.
        assert!(stage_restore(&root, "..", &live).is_err());
        assert!(stage_restore(&root, "nope", &live).is_err());

        let settings = stage_restore(&root, &made.name, &live).unwrap();
        assert_eq!(settings, "{\"rows\":\"tight\"}");
        // Nothing changes until the next start.
        assert_eq!(store.world("wrld_a").unwrap().unwrap().name, "After");
        drop(store);

        // Automatic backups are due when there is none, and only they are cleared away.
        assert!(auto_due(&root, std::time::Duration::from_secs(3600)));
        for _ in 0..3 {
            create(&root, &store_for_auto(&dir), "{}", AUTO_LABEL).unwrap();
        }
        assert!(!auto_due(&root, std::time::Duration::from_secs(3600)));
        assert!(auto_due(&root, std::time::Duration::ZERO));
        assert_eq!(prune_auto(&root, 2), 1);
        let names: Vec<String> = list(&root).into_iter().map(|b| b.name).collect();
        assert_eq!(names.iter().filter(|n| n.starts_with(AUTO_LABEL)).count(), 2);
        assert!(names.contains(&made.name), "a backup made by hand stays");

        assert!(finish_restore(&live).unwrap());
        assert!(!finish_restore(&live).unwrap());
        let again = Store::open(&live).unwrap();
        assert_eq!(again.world("wrld_a").unwrap().unwrap().name, "Before");
        drop(again);
        let _ = fs::remove_dir_all(&dir);
    }
}
