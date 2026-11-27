//! Writes the history out as CSV files that a spreadsheet opens. Everything stays on this PC.

use std::collections::HashMap;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;

use chrono::{DateTime, Local};

use crate::error::{AppError, AppResult};
use crate::store::Store;
use crate::vrchat::model::parse_location;

/// The parts of the history that can be exported, in the order the files are written.
pub const PARTS: [&str; 6] = ["feed", "friends", "visits", "people", "avatars", "videos"];

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFile {
    pub name: String,
    pub rows: u64,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportInfo {
    pub name: String,
    pub path: String,
    pub files: Vec<ExportFile>,
}

fn io(e: std::io::Error) -> AppError {
    AppError::Storage(e.to_string())
}

/// One cell. Text that a spreadsheet would run as a formula gets an apostrophe in front;
/// numbers are written as they are.
enum Cell {
    Text(String),
    Num(String),
}

use Cell::{Num, Text};

fn field(cell: &Cell) -> String {
    let (mut s, guard) = match cell {
        Text(t) => (t.replace('\0', ""), true),
        Num(n) => (n.clone(), false),
    };
    if guard && s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        s.insert(0, '\'');
    }
    if s.contains([',', '"', '\r', '\n']) || s.starts_with(' ') || s.ends_with(' ') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

/// A time from the history (UTC) as the PC's own clock shows it, in a form spreadsheets read
/// as a date. Empty when there is no time.
fn local(utc: &str) -> String {
    DateTime::parse_from_rfc3339(utc)
        .map(|t| t.with_timezone(&Local).format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

/// Whole minutes between two history times. Empty while the second one is not there yet.
fn minutes(from: &str, to: Option<&str>) -> String {
    let (Ok(a), Some(Ok(b))) = (DateTime::parse_from_rfc3339(from), to.map(DateTime::parse_from_rfc3339)) else {
        return String::new();
    };
    (((b - a).num_seconds().max(0) as f64) / 60.0).round().to_string()
}

fn kind_word(kind: &str) -> String {
    match kind {
        "online" => "Came online",
        "offline" => "Went offline",
        "location" => "Changed world",
        "status" => "Status",
        "statusText" => "Status text",
        "name" => "Changed name",
        "avatar" => "Changed avatar",
        "friendAdded" => "Became friends",
        "friendRemoved" => "No longer friends",
        other => other,
    }
    .to_string()
}

fn access_word(access: Option<&str>) -> String {
    match access {
        Some("public") => "Public",
        Some("friendsPlus") => "Friends+",
        Some("friends") => "Friends",
        Some("invitePlus") => "Invite+",
        Some("invite") => "Invite",
        Some("group") => "Group",
        Some("groupPlus") => "Group+",
        Some("groupPublic") => "Group public",
        Some(other) => other,
        None => "",
    }
    .to_string()
}

fn region_word(region: Option<&str>) -> String {
    match region {
        Some("us") => "US West",
        Some("use") => "US East",
        Some("eu") => "Europe",
        Some("jp") => "Japan",
        Some(other) => other,
        None => "",
    }
    .to_string()
}

/// One CSV file being written. It starts with the UTF-8 mark so Excel shows every alphabet.
struct Sheet {
    out: BufWriter<fs::File>,
    rows: u64,
}

impl Sheet {
    fn new(path: &Path, head: &[&str]) -> AppResult<Sheet> {
        let mut out = BufWriter::new(fs::File::create(path).map_err(io)?);
        out.write_all(b"\xEF\xBB\xBF").map_err(io)?;
        let mut sheet = Sheet { out, rows: 0 };
        sheet.line(&head.iter().map(|h| Text((*h).to_string())).collect::<Vec<_>>())?;
        sheet.rows = 0;
        Ok(sheet)
    }

    fn line(&mut self, cells: &[Cell]) -> AppResult<()> {
        let text: Vec<String> = cells.iter().map(field).collect();
        self.out.write_all(text.join(",").as_bytes()).map_err(io)?;
        self.out.write_all(b"\r\n").map_err(io)?;
        self.rows += 1;
        Ok(())
    }

    fn done(mut self) -> AppResult<u64> {
        self.out.flush().map_err(io)?;
        Ok(self.rows)
    }
}

/// Makes a new folder under `root` with one CSV file for each part asked for.
pub fn create(root: &Path, store: &Store, parts: &[String]) -> AppResult<ExportInfo> {
    let wanted: Vec<&str> = PARTS.iter().copied().filter(|p| parts.iter().any(|w| w == p)).collect();
    if wanted.is_empty() {
        return Err(AppError::CannotDo("Pick at least one part of the history to export.".into()));
    }
    fs::create_dir_all(root).map_err(io)?;
    let stamp = Local::now().format("%Y-%m-%d %H.%M.%S");
    let mut folder = root.join(format!("history export {stamp}"));
    let mut n = 2;
    while folder.exists() {
        folder = root.join(format!("history export {stamp} ({n})"));
        n += 1;
    }
    fs::create_dir_all(&folder).map_err(io)?;
    // An export that could not be finished is taken away again, so no half-written files stay.
    match write_into(&folder, store, &wanted) {
        Ok(files) => Ok(ExportInfo {
            name: folder.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            path: folder.display().to_string(),
            files,
        }),
        Err(e) => {
            let _ = fs::remove_dir_all(&folder);
            Err(e)
        }
    }
}

fn write_into(folder: &Path, store: &Store, wanted: &[&str]) -> AppResult<Vec<ExportFile>> {
    let worlds: HashMap<String, String> = store.worlds()?.into_iter().map(|w| (w.id, w.name)).collect();
    let world_name = |id: &str| worlds.get(id).cloned().unwrap_or_default();
    let mut files = Vec::new();
    for part in wanted {
        let (name, rows) = match *part {
            "feed" => {
                let name = "activity feed.csv";
                let mut s = Sheet::new(&folder.join(name), &["Time (UTC)", "Time (local)", "What", "User ID", "Name", "Detail", "Before", "World", "World ID", "Location"])?;
                store.each_row("SELECT at, kind, user_id, display_name, detail, previous, location FROM feed ORDER BY id", |r| {
                    let t = |i: usize| r[i].clone().unwrap_or_default();
                    let world_id = parse_location(&t(6)).world_id.unwrap_or_default();
                    s.line(&[Text(t(0)), Text(local(&t(0))), Text(kind_word(&t(1))), Text(t(2)), Text(t(3)), Text(t(4)), Text(t(5)), Text(world_name(&world_id)), Text(world_id.clone()), Text(t(6))])
                })?;
                (name, s.done()?)
            }
            "friends" => {
                let name = "friend log.csv";
                let mut s = Sheet::new(&folder.join(name), &["Time (UTC)", "Time (local)", "What", "User ID", "Name", "Name before"])?;
                store.each_row(
                    "SELECT at, kind, user_id, display_name, previous FROM feed WHERE kind IN ('friendAdded', 'friendRemoved', 'name') ORDER BY id",
                    |r| {
                        let t = |i: usize| r[i].clone().unwrap_or_default();
                        s.line(&[Text(t(0)), Text(local(&t(0))), Text(kind_word(&t(1))), Text(t(2)), Text(t(3)), Text(t(4))])
                    },
                )?;
                (name, s.done()?)
            }
            "visits" => {
                let name = "world visits.csv";
                let mut s = Sheet::new(
                    &folder.join(name),
                    &["Visit", "Joined (UTC)", "Joined (local)", "Left (UTC)", "Left (local)", "Minutes", "World", "World ID", "Instance type", "Region", "Location", "People"],
                )?;
                store.each_row(
                    "SELECT v.id, v.joined_at, v.left_at, v.world_id, COALESCE(NULLIF(v.world_name, ''), w.name, ''), v.location,
                            (SELECT COUNT(DISTINCT CASE WHEN s.user_id = '' THEN s.display_name ELSE s.user_id END) FROM sightings s WHERE s.visit_id = v.id)
                     FROM visits v LEFT JOIN worlds w ON w.world_id = v.world_id ORDER BY v.id",
                    |r| {
                        let t = |i: usize| r[i].clone().unwrap_or_default();
                        let left = r[2].clone();
                        let place = parse_location(&t(5));
                        s.line(&[
                            Num(t(0)),
                            Text(t(1)),
                            Text(local(&t(1))),
                            Text(left.clone().unwrap_or_default()),
                            Text(left.as_deref().map(local).unwrap_or_default()),
                            Num(minutes(&t(1), left.as_deref())),
                            Text(t(4)),
                            Text(t(3)),
                            Text(access_word(place.access.as_deref())),
                            Text(region_word(place.region.as_deref())),
                            Text(t(5)),
                            Num(t(6)),
                        ])
                    },
                )?;
                (name, s.done()?)
            }
            "people" => {
                let name = "people in worlds.csv";
                let mut s = Sheet::new(
                    &folder.join(name),
                    &["Visit", "World", "User ID", "Name", "Arrived (UTC)", "Arrived (local)", "Left (UTC)", "Left (local)", "Minutes"],
                )?;
                store.each_row(
                    "SELECT s.visit_id, COALESCE(NULLIF(v.world_name, ''), w.name, ''), s.user_id, s.display_name, s.joined_at, s.left_at
                     FROM sightings s JOIN visits v ON v.id = s.visit_id LEFT JOIN worlds w ON w.world_id = v.world_id
                     ORDER BY s.visit_id, s.id",
                    |r| {
                        let t = |i: usize| r[i].clone().unwrap_or_default();
                        let left = r[5].clone();
                        s.line(&[
                            Num(t(0)),
                            Text(t(1)),
                            Text(t(2)),
                            Text(t(3)),
                            Text(t(4)),
                            Text(local(&t(4))),
                            Text(left.clone().unwrap_or_default()),
                            Text(left.as_deref().map(local).unwrap_or_default()),
                            Num(minutes(&t(4), left.as_deref())),
                        ])
                    },
                )?;
                (name, s.done()?)
            }
            "avatars" => {
                let name = "avatars worn.csv";
                let mut s = Sheet::new(
                    &folder.join(name),
                    &["Worn (UTC)", "Worn (local)", "Until (UTC)", "Until (local)", "Minutes", "Avatar ID", "Avatar", "Author ID", "Author", "Public or private"],
                )?;
                store.each_row(
                    "SELECT worn_at, ended_at, avatar_id, name, author_id, author_name, release_status FROM avatar_history ORDER BY id",
                    |r| {
                        let t = |i: usize| r[i].clone().unwrap_or_default();
                        let until = r[1].clone();
                        s.line(&[
                            Text(t(0)),
                            Text(local(&t(0))),
                            Text(until.clone().unwrap_or_default()),
                            Text(until.as_deref().map(local).unwrap_or_default()),
                            Num(minutes(&t(0), until.as_deref())),
                            Text(t(2)),
                            Text(t(3)),
                            Text(t(4)),
                            Text(t(5)),
                            Text(t(6)),
                        ])
                    },
                )?;
                (name, s.done()?)
            }
            _ => {
                let name = "videos played.csv";
                let mut s = Sheet::new(&folder.join(name), &["Time (UTC)", "Time (local)", "World", "World ID", "Link", "Title", "Channel", "Put on by"])?;
                store.each_row(
                    "SELECT p.at, COALESCE(NULLIF(v.world_name, ''), w.name, ''), COALESCE(v.world_id, ''), p.url, COALESCE(t.title, ''), COALESCE(t.channel, ''), p.requested_by
                     FROM videos p LEFT JOIN visits v ON v.id = p.visit_id LEFT JOIN worlds w ON w.world_id = v.world_id
                     LEFT JOIN video_titles t ON t.video_id = p.youtube_id AND p.youtube_id <> ''
                     ORDER BY p.at, p.id",
                    |r| {
                        let t = |i: usize| r[i].clone().unwrap_or_default();
                        s.line(&[Text(t(0)), Text(local(&t(0))), Text(t(1)), Text(t(2)), Text(t(3)), Text(t(4)), Text(t(5)), Text(t(6))])
                    },
                )?;
                (name, s.done()?)
            }
        };
        files.push(ExportFile { name: name.to_string(), rows });
    }
    let about = "Anonymous VRX history export.\r\n\r\n\
Each file opens in Excel, LibreOffice or Google Sheets. Times are given twice: in UTC, the way the\r\n\
app keeps them, and in this PC's own time.\r\n\r\n\
These files hold real names, user ids and instance addresses of the people you met. Keep them\r\n\
private, the same as a backup. Hide names in the app only covers the screen, not these files.\r\n\r\n\
Text that starts with = + - or @ has an apostrophe in front, so a spreadsheet shows it as text\r\n\
and never runs it as a formula.\r\n\r\n\
In countries that write decimals with a comma, Excel may put every line in one column. Then use\r\n\
Data, From Text/CSV, and pick comma as the separator.\r\n";
    fs::write(folder.join("about this export.txt"), about).map_err(io)?;
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gamelog::LogEvent;
    use crate::vrchat::model::FeedDraft;

    #[test]
    fn cells_are_safe_for_spreadsheets() {
        assert_eq!(field(&Text("plain".into())), "plain");
        assert_eq!(field(&Text("=HYPERLINK(\"x\")".into())), "\"'=HYPERLINK(\"\"x\"\")\"");
        assert_eq!(field(&Text("-Kitty-".into())), "'-Kitty-");
        assert_eq!(field(&Text("@here".into())), "'@here");
        assert_eq!(field(&Text("a, b".into())), "\"a, b\"");
        assert_eq!(field(&Text("two\nlines".into())), "\"two\nlines\"");
        assert_eq!(field(&Text(" space".into())), "\" space\"");
        assert_eq!(field(&Text("nul\0gone".into())), "nulgone");
        assert_eq!(field(&Num("-5".into())), "-5");
        assert_eq!(minutes("2026-01-05T20:00:00Z", Some("2026-01-05T20:30:40Z")), "31");
        assert_eq!(minutes("2026-01-05T20:00:00Z", None), "");
    }

    #[test]
    fn writes_every_part_and_cleans_up() {
        let store = Store::in_memory().unwrap();
        store
            .add_feed(&FeedDraft {
                user_id: "usr_1".into(),
                display_name: "=SUM(A1)".into(),
                kind: "statusText",
                detail: "out, \"back\" soon\nreally".into(),
                previous: String::new(),
                location: String::new(),
            })
            .unwrap();
        store
            .add_feed(&FeedDraft { user_id: "usr_2".into(), display_name: "Mossy 🌿".into(), kind: "friendAdded", detail: String::new(), previous: String::new(), location: String::new() })
            .unwrap();
        let at = |t: &str| format!("2026-01-05T{t}Z");
        let events = vec![
            (at("20:00:00"), LogEvent::Room("Quiet Pier".into())),
            (at("20:00:00"), LogEvent::Joining("wrld_a:1~region(eu)".into())),
            (at("20:00:05"), LogEvent::PlayerJoined { name: "Pip".into(), id: "usr_pip".into() }),
            (at("20:00:12"), LogEvent::Video { url: "https://www.youtube.com/watch?v=AbCdEfGhIjK".into() }),
        ];
        store.record_log(&events, true, "log:test", 1).unwrap();

        let root = std::env::temp_dir().join(format!("anonymous-vrx-export-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let all: Vec<String> = PARTS.iter().map(|p| p.to_string()).collect();
        let info = create(&root, &store, &all).unwrap();
        assert_eq!(info.files.len(), 6);
        let feed = fs::read(root.join(&info.name).join("activity feed.csv")).unwrap();
        assert!(feed.starts_with(b"\xEF\xBB\xBF"));
        let feed = String::from_utf8(feed[3..].to_vec()).unwrap();
        assert!(feed.contains("'=SUM(A1)"));
        assert!(feed.contains("\"out, \"\"back\"\" soon\nreally\""));
        assert!(feed.contains("Mossy 🌿"));
        assert!(feed.ends_with("\r\n"));
        let rows = |n: &str| info.files.iter().find(|f| f.name == n).unwrap().rows;
        assert_eq!(rows("activity feed.csv"), 2);
        assert_eq!(rows("friend log.csv"), 1);
        assert_eq!(rows("world visits.csv"), 1);
        assert_eq!(rows("people in worlds.csv"), 1);
        assert_eq!(rows("videos played.csv"), 1);
        let visits = fs::read_to_string(root.join(&info.name).join("world visits.csv")).unwrap();
        // Still in that world: no time left and no minutes yet.
        assert!(visits.contains(",,,Quiet Pier,wrld_a,"), "{visits}");
        assert!(visits.contains(",Europe,wrld_a:1~region(eu),1"), "{visits}");
        assert!(root.join(&info.name).join("about this export.txt").is_file());

        // Nothing asked for: no folder.
        assert!(create(&root, &store, &[]).is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        let _ = fs::remove_dir_all(&root);
    }
}
