//! The photos VRChat's camera saves on this PC.
//!
//! VRChat writes them to `Pictures\VRChat\<year-month>\` with the moment they were taken in
//! the file name, for example `VRChat_2026-10-16_21-12-13.235_2048x1440.png`. VRChat's own
//! `config.json` can move them elsewhere with `picture_output_folder`. Nothing here
//! changes a photo: the files are only listed, and the app looks up in its own world history
//! where I was at that moment.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, LocalResult, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use serde::Serialize;

/// One photo file and when it was taken.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Photo {
    pub path: String,
    pub file: String,
    /// UTC, RFC 3339 to the second, the same shape the world history uses.
    pub taken_at: String,
    pub width: u32,
    pub height: u32,
}

/// Reads the time and size out of a VRChat photo's file name. The time in the name is local.
pub fn parse_name(name: &str) -> Option<(NaiveDateTime, u32, u32)> {
    let stem = name.strip_suffix(".png").or_else(|| name.strip_suffix(".jpg"))?;
    let rest = stem.strip_prefix("VRChat_")?;
    // "2026-10-16_21-12-13.235_2048x1440"; older names put the size first.
    let mut time = None;
    let mut size = None;
    let parts: Vec<&str> = rest.split('_').collect();
    for (i, part) in parts.iter().enumerate() {
        if let Some((w, h)) = part.split_once('x') {
            if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
                size = Some((w, h));
                continue;
            }
        }
        if time.is_none() && i + 1 < parts.len() {
            let joined = format!("{}_{}", part, parts[i + 1]);
            let clock = joined.split('.').next().unwrap_or(&joined);
            if let Ok(t) = NaiveDateTime::parse_from_str(clock, "%Y-%m-%d_%H-%M-%S") {
                time = Some(t);
            }
        }
    }
    let (w, h) = size.unwrap_or((0, 0));
    time.map(|t| (t, w, h))
}

/// How close the file's own save time must be to the second pass of the repeated hour for the
/// photo to count as taken then. VRChat writes the file as the photo is taken, so it is seconds
/// apart; a file changed much later says nothing about which pass it was.
const SAVED_WITHIN: i64 = 10 * 60;

/// Picks between the two moments a local time in the hour the autumn clock change repeats can
/// mean. The file's save time (`saved`, when Windows gives one) settles it when it lies near the
/// second moment; otherwise the first is kept, as before.
fn pick(first: DateTime<Utc>, second: DateTime<Utc>, saved: Option<DateTime<Utc>>) -> DateTime<Utc> {
    match saved {
        Some(saved) if (saved - second).num_seconds().abs() <= SAVED_WITHIN => second,
        _ => first,
    }
}

/// The photo's local time in UTC. `saved` gives the file's save time, and is only asked for a
/// time in the repeated hour. A time the spring clock change skips cannot be a real photo's, so
/// that file is left out.
fn to_utc(local: NaiveDateTime, saved: impl FnOnce() -> Option<DateTime<Utc>>) -> Option<String> {
    let at = match Local.from_local_datetime(&local) {
        LocalResult::Single(at) => at.with_timezone(&Utc),
        LocalResult::Ambiguous(first, second) => pick(first.with_timezone(&Utc), second.with_timezone(&Utc), saved()),
        LocalResult::None => return None,
    };
    Some(at.to_rfc3339_opts(SecondsFormat::Secs, true))
}

/// The photo folder named by `picture_output_folder` in VRChat's `config.json` inside `game`,
/// when it is set and the folder is there. Nothing otherwise, and nothing for a file that
/// cannot be read, so the usual folder is used then.
pub fn picture_folder(game: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(game.join("config.json")).ok()?;
    let config: serde_json::Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    let folder = config.get("picture_output_folder")?.as_str()?.trim();
    if folder.is_empty() {
        return None;
    }
    let folder = PathBuf::from(folder);
    (folder.is_absolute() && folder.is_dir()).then_some(folder)
}

/// Every VRChat photo under `root`, newest first.
pub fn scan(root: &Path) -> Vec<Photo> {
    let mut found = Vec::new();
    let mut folders: Vec<PathBuf> = vec![root.to_path_buf()];
    // The month folders, and one level below in case someone sorted them further.
    for depth in 0..3 {
        let mut next = Vec::new();
        for folder in &folders {
            let Ok(entries) = std::fs::read_dir(folder) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if depth < 2 {
                        next.push(path);
                    }
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let Some((local, width, height)) = parse_name(&name) else { continue };
                let saved = || entry.metadata().ok()?.modified().ok().map(DateTime::<Utc>::from);
                let Some(taken_at) = to_utc(local, saved) else { continue };
                found.push(Photo { path: path.to_string_lossy().into_owned(), file: name, taken_at, width, height });
            }
        }
        folders = next;
    }
    found.sort_by(|a, b| b.taken_at.cmp(&a.taken_at).then_with(|| b.file.cmp(&a.file)));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_time_and_size_from_the_name() {
        let (t, w, h) = parse_name("VRChat_2026-10-16_21-12-13.235_2048x1440.png").unwrap();
        assert_eq!(t.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-10-16 21:12:13");
        assert_eq!((w, h), (2048, 1440));
        // The older order, size first.
        let (t, w, h) = parse_name("VRChat_1920x1080_2022-03-05_08-01-59.120.png").unwrap();
        assert_eq!(t.format("%Y-%m-%d %H:%M:%S").to_string(), "2022-03-05 08:01:59");
        assert_eq!((w, h), (1920, 1080));
        assert!(parse_name("holiday.png").is_none());
        assert!(parse_name("VRChat_notes.txt").is_none());
    }

    #[test]
    fn the_repeated_hour_follows_the_files_save_time() {
        let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        // 02:30 local on the autumn night, in a zone going from UTC+2 to UTC+1.
        let first = at("2026-10-25T00:30:00Z");
        let second = at("2026-10-25T01:30:00Z");
        // Saved seconds after the second pass: the second.
        assert_eq!(pick(first, second, Some(at("2026-10-25T01:30:04Z"))), second);
        // Saved right after the first pass: the first.
        assert_eq!(pick(first, second, Some(at("2026-10-25T00:30:04Z"))), first);
        // No save time, or one from much later (an edited file): the first, as before.
        assert_eq!(pick(first, second, None), first);
        assert_eq!(pick(first, second, Some(at("2026-11-02T12:00:00Z"))), first);
    }

    #[test]
    fn lists_newest_first_and_skips_other_files() {
        let root = std::env::temp_dir().join(format!("avrx-photos-{}", std::process::id()));
        let month = root.join("2026-10");
        std::fs::create_dir_all(&month).unwrap();
        for name in ["VRChat_2026-10-16_21-12-13.235_2048x1440.png", "VRChat_2026-10-17_09-00-00.000_1920x1080.png", "desktop.ini"] {
            std::fs::write(month.join(name), b"x").unwrap();
        }
        let photos = scan(&root);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(photos.len(), 2);
        assert!(photos[0].file.contains("2026-10-17"));
        assert!(photos[0].taken_at.ends_with('Z'));
    }

    #[test]
    fn uses_the_photo_folder_vrchat_was_told_to_use() {
        let game = std::env::temp_dir().join(format!("avrx-photo-config-{}", std::process::id()));
        let own = game.join("Camera roll");
        std::fs::create_dir_all(&own).unwrap();
        // No config.json, or one without the setting: the usual folder.
        assert_eq!(picture_folder(&game), None);
        std::fs::write(game.join("config.json"), "{ \"cache_size\": 40 }").unwrap();
        assert_eq!(picture_folder(&game), None);
        // A folder that is there is used, written the way VRChat's file has it.
        let named = serde_json::json!({ "picture_output_folder": own.to_string_lossy() }).to_string();
        std::fs::write(game.join("config.json"), format!("\u{feff}{named}")).unwrap();
        assert_eq!(picture_folder(&game), Some(own.clone()));
        // A folder that is not there, an empty one or a broken file: the usual folder.
        std::fs::write(game.join("config.json"), r#"{ "picture_output_folder": "Z:\\gone\\nowhere" }"#).unwrap();
        assert_eq!(picture_folder(&game), None);
        std::fs::write(game.join("config.json"), r#"{ "picture_output_folder": "  " }"#).unwrap();
        assert_eq!(picture_folder(&game), None);
        std::fs::write(game.join("config.json"), "{ oops").unwrap();
        assert_eq!(picture_folder(&game), None);
        let _ = std::fs::remove_dir_all(&game);
    }
}
