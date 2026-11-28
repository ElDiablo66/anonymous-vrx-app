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
/// Holds the VRChat user id of the account the backup was made for. Every account on this PC
/// backs up into the same folder, so this is how one account's backups are told apart from another's.
const ACCOUNT_FILE: &str = "account.txt";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub name: String,
    pub path: String,
    pub bytes: u64,
    /// The account the backup was made for. Empty for a backup made before backups were tagged.
    pub account: String,
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
        account: account_of(folder).unwrap_or_default(),
    })
}

/// The account a backup was made for, or nothing for an older backup that was never tagged.
fn account_of(folder: &Path) -> Option<String> {
    let text = fs::read_to_string(folder.join(ACCOUNT_FILE)).ok()?;
    let id = text.trim();
    (!id.is_empty()).then(|| id.to_string())
}

/// True when a backup belongs to this account. An untagged older backup could be anyone's,
/// so it counts for whoever is signed in.
fn belongs_to(owner: Option<&str>, account: &str) -> bool {
    owner.is_none_or(|id| id == account)
}

/// Makes a new backup folder under `root` for one account and returns what was written.
pub fn create(root: &Path, store: &Store, settings_json: &str, label: &str, account: &str) -> AppResult<BackupInfo> {
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
    let written = write_into(&folder, store, settings_json, account);
    if written.is_err() {
        let _ = fs::remove_dir_all(&folder);
    }
    written
}

fn write_into(folder: &Path, store: &Store, settings_json: &str, account: &str) -> AppResult<BackupInfo> {
    store.copy_to(&folder.join(HISTORY_FILE))?;
    fs::write(folder.join(SETTINGS_FILE), settings_json).map_err(io)?;
    fs::write(folder.join(ACCOUNT_FILE), account).map_err(io)?;
    let about = "Anonymous VRX backup.\r\n\r\n\
history.sqlite: friends, activity feed, world history, handled notifications, cached world names.\r\n\
settings.json: size, rows, notification switches, what the close button does.\r\n\
account.txt: the VRChat account this backup belongs to. It can only be restored while that account is signed in.\r\n\r\n\
The VRChat sign-in is not in here. After a restore on another PC, sign in again.\r\n\
To restore: Settings, Your data, pick this backup, Restore.\r\n";
    fs::write(folder.join(ABOUT_FILE), about).map_err(io)?;
    info(folder).ok_or_else(|| AppError::Storage("the backup was not written".into()))
}

/// Backups found under `root`, newest first, whoever they belong to.
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

/// The backups of one account under `root`, newest first, with the untagged older ones.
pub fn list_for(root: &Path, account: &str) -> Vec<BackupInfo> {
    list(root)
        .into_iter()
        .filter(|b| belongs_to(Some(b.account.as_str()).filter(|a| !a.is_empty()), account))
        .collect()
}

/// The label automatic backups carry. Only folders with it are ever cleared away.
pub const AUTO_LABEL: &str = "automatic backup";

/// A backup found on disk: when it was made, where it is, and whose it is when it says.
type Found = (std::time::SystemTime, PathBuf, Option<String>);

/// True when this account has no backup with this label yet, or its newest is older than
/// `every`. An untagged older backup counts for whoever is signed in.
pub fn due_for(root: &Path, label: &str, account: &str, every: std::time::Duration) -> bool {
    match labelled(root, label).into_iter().find(|(_, _, owner)| belongs_to(owner.as_deref(), account)) {
        // A backup dated in the future means the PC's clock was set back; that is not "due".
        Some((made, _, _)) => made.elapsed().map(|age| age >= every).unwrap_or(false),
        None => true,
    }
}

/// Keeps the newest `keep` backups of this account with this label and removes its older
/// ones. Another account's backups are never touched, and neither is an untagged older one,
/// since nobody can tell whose it is. Returns how many were removed.
pub fn prune_for(root: &Path, label: &str, account: &str, keep: usize) -> usize {
    let mut removed = 0;
    let own = labelled(root, label).into_iter().filter(|(_, _, owner)| owner.as_deref() == Some(account));
    for (_, folder, _) in own.skip(keep.max(1)) {
        if fs::remove_dir_all(&folder).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// The backups under `root` that carry this label, with the time each was made and the
/// account each belongs to, newest first.
fn labelled(root: &Path, label: &str) -> Vec<Found> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<Found> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        // A real one of ours: the right name, and the note this app writes into every backup.
        .filter(|e| e.file_name().to_string_lossy().starts_with(&format!("{label} ")) && e.path().join(ABOUT_FILE).is_file())
        .filter_map(|e| Some((e.metadata().and_then(|m| m.modified()).ok()?, e.path(), account_of(&e.path()))))
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found
}

/// True when this account has no automatic backup yet, or its newest is older than `every`.
pub fn auto_due(root: &Path, account: &str, every: std::time::Duration) -> bool {
    due_for(root, AUTO_LABEL, account, every)
}

/// Keeps the newest `keep` automatic backups of this account and removes its older ones.
/// Backups made by hand, other accounts' backups, untagged older ones and anything else in
/// the folder are never touched. Returns how many were removed.
pub fn prune_auto(root: &Path, account: &str, keep: usize) -> usize {
    prune_for(root, AUTO_LABEL, account, keep)
}

/// Where a restored history waits until the next start.
pub fn pending_path(live: &Path) -> PathBuf {
    let mut name = live.file_name().unwrap_or_default().to_os_string();
    name.push(".restore");
    live.with_file_name(name)
}

/// Checks that a backup can be restored for this account, before anything is written, and
/// gives back its folder. A backup made for another account is refused, so one account's
/// history never takes the place of another's.
pub fn check_restore(root: &Path, name: &str, account: &str) -> AppResult<PathBuf> {
    if name.is_empty() || name.contains(['/', '\\', ':']) || name.contains("..") {
        return Err(AppError::CannotDo("That is not a backup name.".into()));
    }
    let folder = root.join(name);
    if !folder.join(HISTORY_FILE).is_file() {
        return Err(AppError::CannotDo("That backup has no history file in it.".into()));
    }
    if !belongs_to(account_of(&folder).as_deref(), account) {
        return Err(AppError::CannotDo(
            "That backup belongs to another VRChat account. Sign in to that account to restore it.".into(),
        ));
    }
    Ok(folder)
}

/// Checks a backup and puts its history next to the live file, to be swapped in
/// at the next start. Returns the backup's settings text.
pub fn stage_restore(root: &Path, name: &str, live: &Path, account: &str) -> AppResult<String> {
    let folder = check_restore(root, name, account)?;
    let source = folder.join(HISTORY_FILE);
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
        let made = create(&root, &store, "{\"rows\":\"tight\"}", "backup", "usr_1").unwrap();
        assert!(made.bytes > 0);
        assert!(made.name.starts_with("backup "));
        assert_eq!(made.account, "usr_1");
        store.save_world(&world("After")).unwrap();
        assert_eq!(list(&root).len(), 1);

        // Bad names never reach the disk.
        assert!(stage_restore(&root, "..", &live, "usr_1").is_err());
        assert!(stage_restore(&root, "nope", &live, "usr_1").is_err());

        let settings = stage_restore(&root, &made.name, &live, "usr_1").unwrap();
        assert_eq!(settings, "{\"rows\":\"tight\"}");
        // Nothing changes until the next start.
        assert_eq!(store.world("wrld_a").unwrap().unwrap().name, "After");
        drop(store);

        // Automatic backups are due when there is none, and only they are cleared away.
        assert!(auto_due(&root, "usr_1", std::time::Duration::from_secs(3600)));
        for _ in 0..3 {
            create(&root, &store_for_auto(&dir), "{}", AUTO_LABEL, "usr_1").unwrap();
        }
        assert!(!auto_due(&root, "usr_1", std::time::Duration::from_secs(3600)));
        assert!(auto_due(&root, "usr_1", std::time::Duration::ZERO));
        assert_eq!(prune_auto(&root, "usr_1", 2), 1);
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

    #[test]
    fn each_account_keeps_and_restores_only_its_own_backups() {
        let dir = std::env::temp_dir().join(format!("anonymous-vrx-backup-accounts-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let root = dir.join("backups");
        fs::create_dir_all(&dir).unwrap();
        let day = std::time::Duration::from_secs(24 * 3600);

        // The main account made its daily backups; the second account signing in the same
        // day still gets its own.
        for _ in 0..3 {
            create(&root, &store_for_auto(&dir), "{}", AUTO_LABEL, "usr_main").unwrap();
        }
        assert!(!auto_due(&root, "usr_main", day));
        assert!(auto_due(&root, "usr_alt", day));
        let alt = create(&root, &store_for_auto(&dir), "{}", AUTO_LABEL, "usr_alt").unwrap();
        assert!(!auto_due(&root, "usr_alt", day));

        // Pruning for one account never removes the other's.
        assert_eq!(prune_auto(&root, "usr_alt", 1), 0);
        assert_eq!(prune_auto(&root, "usr_main", 1), 2);
        assert_eq!(list(&root).iter().filter(|b| b.account == "usr_main").count(), 1);
        assert_eq!(list(&root).iter().filter(|b| b.account == "usr_alt").count(), 1);

        // An older backup with no account in it is shown to everyone, counts as anyone's
        // last backup, and is never cleared away by itself.
        let old = create(&root, &store_for_auto(&dir), "{}", AUTO_LABEL, "usr_old").unwrap();
        fs::remove_file(Path::new(&old.path).join(ACCOUNT_FILE)).unwrap();
        assert_eq!(list_for(&root, "usr_main").len(), 2);
        assert_eq!(list_for(&root, "usr_alt").len(), 2);
        assert!(!auto_due(&root, "usr_new", day));
        assert_eq!(prune_auto(&root, "usr_main", 1), 0);
        assert!(list(&root).iter().any(|b| b.name == old.name));

        // Another account's backup is refused before anything is written; an untagged one is not.
        let live = dir.join("history usr_main.sqlite");
        assert!(matches!(stage_restore(&root, &alt.name, &live, "usr_main"), Err(AppError::CannotDo(_))));
        assert!(!pending_path(&live).exists());
        assert!(check_restore(&root, &alt.name, "usr_alt").is_ok());
        assert!(check_restore(&root, &old.name, "usr_main").is_ok());
        let _ = fs::remove_dir_all(&dir);
    }
}
