//! VRChat's own download cache on this PC: how big it is, and clearing it.
//!
//! The game keeps every world and avatar it downloaded in `Cache-WindowsPlayer`, one folder
//! per item with one folder per version inside. Only that folder is ever touched, and only
//! while the game is closed.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::error::{AppError, AppResult};

const CACHE_FOLDER: &str = "Cache-WindowsPlayer";
/// VRChat's own floor and default for `cache_size`, in gigabytes.
pub const SMALLEST_LIMIT: u64 = 30;

/// Where VRChat keeps its files for this Windows user.
pub fn game_folder() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join("AppData").join("LocalLow").join("VRChat").join("VRChat"))
}

fn config_file(game: &Path) -> PathBuf {
    game.join("config.json")
}

/// VRChat's `config.json` as an object. Missing is an empty one; unreadable is an error, so a
/// file with a typo in it is never overwritten.
fn read_config(game: &Path) -> AppResult<serde_json::Map<String, Value>> {
    let file = config_file(game);
    if !file.is_file() {
        return Ok(serde_json::Map::new());
    }
    let text = fs::read_to_string(&file).map_err(|e| AppError::Storage(e.to_string()))?;
    match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(AppError::CannotDo("VRChat's config.json has a mistake in it, so it was left alone. Fix or remove the file first.".into())),
    }
}

/// The cache folder: the one named in `config.json`, or VRChat's usual place.
pub fn cache_folder(game: &Path) -> PathBuf {
    let own = read_config(game)
        .ok()
        .and_then(|c| c.get("cache_directory").and_then(Value::as_str).map(str::to_string))
        .filter(|d| !d.trim().is_empty());
    match own {
        Some(dir) => PathBuf::from(dir).join(CACHE_FOLDER),
        None => game.join(CACHE_FOLDER),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheInfo {
    pub path: String,
    /// False when the folder is not there (VRChat never ran, or the cache was moved).
    pub found: bool,
    pub bytes: u64,
    /// Worlds and avatars kept.
    pub items: u64,
    /// Versions of those that a newer download has replaced.
    pub old_versions: u64,
    pub old_bytes: u64,
    /// The size VRChat keeps the cache under, in gigabytes.
    pub limit_gb: u64,
}

fn folder_bytes(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => folder_bytes(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

/// The version folders of one cached item, newest first.
fn versions(item: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(item) else {
        return Vec::new();
    };
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| Some((e.metadata().and_then(|m| m.modified()).ok()?, e.path())))
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, p)| p).collect()
}

fn items(cache: &Path) -> Vec<PathBuf> {
    fs::read_dir(cache)
        .map(|entries| entries.filter_map(Result::ok).filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).collect())
        .unwrap_or_default()
}

pub fn measure(game: &Path) -> CacheInfo {
    let cache = cache_folder(game);
    let limit_gb = read_config(game)
        .ok()
        .and_then(|c| c.get("cache_size").and_then(Value::as_u64))
        .map_or(SMALLEST_LIMIT, |gb| gb.max(SMALLEST_LIMIT));
    let mut info = CacheInfo { path: cache.display().to_string(), found: cache.is_dir(), bytes: 0, items: 0, old_versions: 0, old_bytes: 0, limit_gb };
    for item in items(&cache) {
        info.items += 1;
        for (n, version) in versions(&item).iter().enumerate() {
            let bytes = folder_bytes(version);
            info.bytes += bytes;
            if n > 0 {
                info.old_versions += 1;
                info.old_bytes += bytes;
            }
        }
    }
    info
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cleared {
    pub bytes: u64,
    pub folders: u64,
    /// Folders Windows would not let go of.
    pub kept: u64,
}

/// Removes everything in the cache, or with `old_only` just the versions a newer one replaced.
pub fn clear(game: &Path, old_only: bool) -> AppResult<Cleared> {
    let cache = cache_folder(game);
    // Never anything but VRChat's cache folder, whatever config.json points at.
    if cache.file_name().and_then(|n| n.to_str()) != Some(CACHE_FOLDER) || !cache.is_dir() {
        return Err(AppError::CannotDo("VRChat's cache folder was not found on this PC.".into()));
    }
    let mut done = Cleared { bytes: 0, folders: 0, kept: 0 };
    let mut remove = |dir: &Path| {
        let bytes = folder_bytes(dir);
        if fs::remove_dir_all(dir).is_ok() {
            done.bytes += bytes;
            done.folders += 1;
        } else {
            done.kept += 1;
        }
    };
    for item in items(&cache) {
        if old_only {
            for version in versions(&item).iter().skip(1) {
                remove(version);
            }
        } else {
            remove(&item);
        }
    }
    Ok(done)
}

/// Writes the cache size limit into VRChat's `config.json`, leaving every other line as it is.
pub fn set_limit(game: &Path, gb: u64) -> AppResult<u64> {
    let gb = gb.clamp(SMALLEST_LIMIT, 2000);
    let mut config = read_config(game)?;
    config.insert("cache_size".into(), Value::from(gb));
    let text = serde_json::to_string_pretty(&Value::Object(config)).map_err(|e| AppError::Storage(e.to_string()))?;
    fs::create_dir_all(game).map_err(|e| AppError::Storage(e.to_string()))?;
    fs::write(config_file(game), text).map_err(|e| AppError::Storage(e.to_string()))?;
    Ok(gb)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("anon-vrx-cache-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn version(game: &Path, item: &str, version: &str, bytes: usize) {
        let dir = game.join(CACHE_FOLDER).join(item).join(version);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("__data"), vec![0u8; bytes]).unwrap();
        fs::write(dir.join("__info"), b"-1\n10\n").unwrap();
        // Later calls get a later time, so "newest" is not a coin toss.
        std::thread::sleep(std::time::Duration::from_millis(30));
    }

    #[test]
    fn old_versions_go_and_the_newest_of_each_item_stays() {
        let game = scratch("sweep");
        version(&game, "111", "0001", 1000);
        version(&game, "111", "0002", 2000);
        version(&game, "222", "0001", 500);
        fs::write(game.join("keep me.txt"), b"not cache").unwrap();

        let before = measure(&game);
        assert_eq!((before.items, before.old_versions, before.old_bytes), (2, 1, 1006));

        let swept = clear(&game, true).unwrap();
        assert_eq!((swept.folders, swept.bytes, swept.kept), (1, 1006, 0));
        assert!(!game.join(CACHE_FOLDER).join("111").join("0001").exists());
        assert!(game.join(CACHE_FOLDER).join("111").join("0002").join("__data").is_file());
        assert!(game.join(CACHE_FOLDER).join("222").join("0001").is_dir());

        let all = clear(&game, false).unwrap();
        assert_eq!(all.folders, 2);
        assert_eq!(measure(&game).bytes, 0);
        // The cache folder itself and everything beside it are still there.
        assert!(game.join(CACHE_FOLDER).is_dir());
        assert!(game.join("keep me.txt").is_file());
        let _ = fs::remove_dir_all(&game);
    }

    #[test]
    fn the_size_limit_is_written_without_losing_other_lines() {
        let game = scratch("limit");
        fs::write(config_file(&game), "{ \"camera_res_height\": 2160, \"cache_size\": 40 }").unwrap();
        assert_eq!(measure(&game).limit_gb, 40);
        assert_eq!(set_limit(&game, 5).unwrap(), 30, "never under VRChat's own floor");
        assert_eq!(set_limit(&game, 60).unwrap(), 60);
        let kept: Value = serde_json::from_str(&fs::read_to_string(config_file(&game)).unwrap()).unwrap();
        assert_eq!(kept["camera_res_height"], 2160);
        assert_eq!(kept["cache_size"], 60);

        // A file with a mistake in it is not overwritten.
        fs::write(config_file(&game), "{ not json").unwrap();
        assert!(set_limit(&game, 60).is_err());
        assert_eq!(fs::read_to_string(config_file(&game)).unwrap(), "{ not json");
        let _ = fs::remove_dir_all(&game);
    }

    #[test]
    fn a_cache_moved_in_the_settings_file_is_followed() {
        let game = scratch("moved");
        let other = scratch("moved-to");
        fs::write(config_file(&game), serde_json::json!({ "cache_directory": other.display().to_string() }).to_string()).unwrap();
        assert_eq!(cache_folder(&game), other.join(CACHE_FOLDER));
        assert!(clear(&game, false).is_err(), "nothing to clear where there is no cache folder");
        let _ = fs::remove_dir_all(&game);
        let _ = fs::remove_dir_all(&other);
    }
}
