//! The photos VRChat's camera saves on this PC.
//!
//! VRChat writes them to `Pictures\VRChat\<year-month>\` with the moment they were taken in
//! the file name, for example `VRChat_2026-10-16_21-12-13.235_2048x1440.png`. Nothing here
//! changes a photo: the files are only listed, and the app looks up in its own world history
//! where I was at that moment.

use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDateTime, SecondsFormat, TimeZone, Utc};
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

fn to_utc(local: NaiveDateTime) -> Option<String> {
    let at = Local.from_local_datetime(&local).earliest()?;
    Some(at.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Secs, true))
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
                let Some(taken_at) = to_utc(local) else { continue };
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
}
