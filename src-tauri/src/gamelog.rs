//! Reads VRChat's own output log files to learn which instances this PC
//! visited and who was there. Nothing is written to those files.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDateTime, SecondsFormat, TimeZone, Utc};

use crate::error::{AppError, AppResult};
use crate::store::Store;

#[derive(Debug, PartialEq, Clone)]
pub enum LogEvent {
    /// The world's name, written just before the join line.
    Room(String),
    /// Full location string of the instance being joined.
    Joining(String),
    LeftRoom,
    PlayerJoined { name: String, id: String },
    PlayerLeft { name: String, id: String },
    Quit,
}

/// Most bytes read from one file in one pass.
const CHUNK: u64 = 8 * 1024 * 1024;
const STAMP: &str = "%Y.%m.%d %H:%M:%S";

/// Where VRChat keeps its logs on Windows.
pub fn default_dir() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")?;
    Some(
        PathBuf::from(home)
            .join("AppData")
            .join("LocalLow")
            .join("VRChat")
            .join("VRChat"),
    )
}

fn split_player(text: &str) -> (String, String) {
    if let Some(open) = text.rfind(" (usr_") {
        if text.ends_with(')') {
            return (
                text[..open].to_string(),
                text[open + 2..text.len() - 1].to_string(),
            );
        }
    }
    (text.to_string(), String::new())
}

/// Reads one log line. Lines that are not of interest give `None`.
pub fn parse_line(line: &str) -> Option<(NaiveDateTime, LogEvent)> {
    let stamp = line.get(..19)?;
    let at = NaiveDateTime::parse_from_str(stamp, STAMP).ok()?;
    let body = line.get(19..)?.split_once(" -  ")?.1.trim_end();

    if let Some(text) = body.strip_prefix("[Behaviour] ") {
        if let Some(name) = text.strip_prefix("Entering Room: ") {
            return Some((at, LogEvent::Room(name.to_string())));
        }
        if let Some(location) = text.strip_prefix("Joining ") {
            if location.starts_with("wrld_") || location.starts_with("wld_") {
                return Some((at, LogEvent::Joining(location.to_string())));
            }
            return None;
        }
        if text == "OnLeftRoom" {
            return Some((at, LogEvent::LeftRoom));
        }
        if let Some(who) = text.strip_prefix("OnPlayerJoined ") {
            let (name, id) = split_player(who);
            return Some((at, LogEvent::PlayerJoined { name, id }));
        }
        if let Some(who) = text.strip_prefix("OnPlayerLeft ") {
            let (name, id) = split_player(who);
            return Some((at, LogEvent::PlayerLeft { name, id }));
        }
        return None;
    }
    if body.starts_with("VRCApplication: ")
        && (body.contains("OnApplicationQuit at") || body.contains("HandleApplicationQuit at"))
    {
        return Some((at, LogEvent::Quit));
    }
    None
}

/// Whether the newest log file ends with VRChat's goodbye line. Read straight from the
/// file, so it holds even when the history could not be brought up to date. Nothing when
/// the log cannot be read, so the caller does not mistake that for a crash.
pub fn said_goodbye(dir: &Path) -> Option<bool> {
    let (_, path) = log_files(dir)?.into_iter().max_by(|a, b| a.0.cmp(&b.0))?;
    let size = fs::metadata(&path).ok()?.len();
    let tail = 64 * 1024;
    let bytes = read_range(&path, size.saturating_sub(tail), tail.min(size)).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    Some(text.contains("VRCApplication: HandleApplicationQuit") || text.contains("VRCApplication: OnApplicationQuit"))
}

/// The log uses the PC's local time with no zone. History is stored in UTC.
pub fn to_utc(at: NaiveDateTime) -> String {
    let utc = match Local.from_local_datetime(&at).earliest() {
        Some(local) => local.with_timezone(&Utc),
        None => Utc.from_utc_datetime(&at),
    };
    utc.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// The log files in the folder, or nothing when the folder cannot be read right now.
/// "Cannot be read" must not look like "no files": the caller would forget how far it got.
fn log_files(dir: &Path) -> Option<Vec<(String, PathBuf)>> {
    let entries = fs::read_dir(dir).ok()?;
    let mut files: Vec<(String, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            (name.starts_with("output_log_") && name.ends_with(".txt")).then(|| (name, e.path()))
        })
        .collect();
    // The file name carries the start time, so name order is time order.
    files.sort();
    Some(files)
}

fn read_range(path: &Path, from: u64, len: u64) -> std::io::Result<Vec<u8>> {
    let mut file = fs::File::open(path)?;
    file.seek(SeekFrom::Start(from))?;
    let mut buf = Vec::with_capacity(len as usize);
    file.take(len).read_to_end(&mut buf)?;
    Ok(buf)
}

/// One log file and how far into it the history has already read.
pub struct Pending {
    pub key: String,
    path: PathBuf,
    offset: u64,
}

/// What the next stretch of a log file holds.
pub struct Chunk {
    pub events: Vec<(String, LogEvent)>,
    /// The file was read from its first byte, so the game was started again.
    pub fresh_file: bool,
    /// Where the history stands once these events are recorded.
    pub offset: u64,
}

/// Which files there are and where each one was left. The only step that needs the history,
/// and it is quick. Nothing when the folder cannot be read right now.
pub fn plan(dir: &Path, store: &Store) -> AppResult<Option<Vec<Pending>>> {
    let Some(files) = log_files(dir) else {
        return Ok(None);
    };
    let mut out = Vec::new();
    for (name, path) in &files {
        let key = format!("log:{name}");
        let offset: u64 = store.setting(&key)?.and_then(|v| v.parse().ok()).unwrap_or(0);
        out.push(Pending { key, path: path.clone(), offset });
    }
    let keep: Vec<String> = out.iter().map(|p| p.key.clone()).collect();
    store.forget_settings_with_prefix("log:", &keep)?;
    Ok(Some(out))
}

/// Reads the next whole lines of one file, straight from disk. Nothing when there is
/// nothing new. The history is not touched, so a long read holds nothing else up.
pub fn read_next(pending: &mut Pending) -> AppResult<Option<Chunk>> {
    loop {
        let Ok(meta) = fs::metadata(&pending.path) else {
            return Ok(None);
        };
        let size = meta.len();
        if size < pending.offset {
            pending.offset = 0;
        }
        if size == pending.offset {
            return Ok(None);
        }
        let bytes = read_range(&pending.path, pending.offset, (size - pending.offset).min(CHUNK))
            .map_err(|e| AppError::Storage(e.to_string()))?;
        // Only whole lines count; a half-written last line waits for the next pass.
        let Some(last_break) = bytes.iter().rposition(|b| *b == b'\n') else {
            // A whole chunk with no line end is one endless line; step over it so the
            // rest of the file is still read.
            if bytes.len() as u64 >= CHUNK {
                pending.offset += bytes.len() as u64;
                continue;
            }
            return Ok(None);
        };
        let text = String::from_utf8_lossy(&bytes[..=last_break]);
        let events: Vec<(String, LogEvent)> = text
            .lines()
            .filter_map(parse_line)
            .map(|(at, event)| (to_utc(at), event))
            .collect();
        let fresh_file = pending.offset == 0;
        pending.offset += last_break as u64 + 1;
        return Ok(Some(Chunk { events, fresh_file, offset: pending.offset }));
    }
}

/// Reads whatever is new in the log folder into the history database, in one go and with
/// the history in hand the whole time. Returns true when anything was recorded.
pub fn catch_up(dir: &Path, store: &Store) -> AppResult<bool> {
    let Some(mut files) = plan(dir, store)? else {
        return Ok(false);
    };
    let mut changed = false;
    for pending in &mut files {
        while let Some(chunk) = read_next(pending)? {
            if store.record_log(&chunk.events, chunk.fresh_file, &pending.key, chunk.offset)? {
                changed = true;
            }
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const JOIN: &str = "2026.01.05 20:15:02 Debug      -  [Behaviour] Joining wrld_11111111-2222-3333-4444-555555555555:40211~hidden(usr_aaaa)~region(eu)";

    #[test]
    fn reads_the_lines_that_matter() {
        let (_, e) = parse_line("2026.01.05 20:15:01 Debug      -  [Behaviour] Entering Room: Quiet Pier at Dusk").unwrap();
        assert_eq!(e, LogEvent::Room("Quiet Pier at Dusk".into()));

        let (at, e) = parse_line(JOIN).unwrap();
        assert_eq!(at.format("%H:%M:%S").to_string(), "20:15:02");
        assert!(matches!(e, LogEvent::Joining(l) if l.starts_with("wrld_1111") && l.ends_with("region(eu)")));

        let (_, e) = parse_line("2026.01.05 20:15:09 Debug      -  [Behaviour] OnPlayerJoined Pip Halloway (usr_0000-pip)").unwrap();
        assert_eq!(e, LogEvent::PlayerJoined { name: "Pip Halloway".into(), id: "usr_0000-pip".into() });

        let (_, e) = parse_line("2026.01.05 20:40:00 Debug      -  [Behaviour] OnPlayerLeft Name (with) brackets (usr_0000-odd)\r").unwrap();
        assert_eq!(e, LogEvent::PlayerLeft { name: "Name (with) brackets".into(), id: "usr_0000-odd".into() });

        assert_eq!(parse_line("2026.01.05 20:41:00 Debug      -  [Behaviour] OnLeftRoom").unwrap().1, LogEvent::LeftRoom);
        assert_eq!(
            parse_line("2026.01.05 22:00:00 Debug      -  VRCApplication: HandleApplicationQuit at 7714.082").unwrap().1,
            LogEvent::Quit
        );
    }

    #[test]
    fn ignores_lookalikes_and_noise() {
        for line in [
            "2026.01.05 20:15:02 Debug      -  [Behaviour] Joining or Creating Room: Quiet Pier at Dusk",
            "2026.01.05 20:15:02 Debug      -  [Behaviour] Destination fetching: wrld_1111:1",
            "2026.01.05 20:15:09 Debug      -  [PlayerLog] OnPlayerJoined: Someone (VR=True)",
            "2026.01.05 20:15:09 Debug      -  [Behaviour] OnPlayerJoinComplete Pip",
            "    at SomeStackFrame ()",
            "",
            "not a date at all here, just some long text without meaning",
        ] {
            assert!(parse_line(line).is_none(), "{line}");
        }
    }

    /// Reads this PC's real VRChat logs into a throwaway database and prints
    /// counts only. Run by hand: `cargo test real_logs -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_logs_give_visits() {
        let dir = default_dir().expect("no user profile folder");
        let store = Store::in_memory().unwrap();
        catch_up(&dir, &store).unwrap();
        let visits = store.visits(500, None).unwrap();
        println!("{} log files, {} visits", log_files(&dir).unwrap().len(), visits.len());
        for v in &visits {
            println!(
                "{} to {} people={} named={} access={:?}",
                v.joined_at,
                v.left_at.as_deref().unwrap_or("open"),
                v.people,
                !v.world_name.is_empty(),
                v.place.access
            );
        }
    }

    fn line(time: &str, body: &str) -> String {
        format!("2026.01.05 {time} Debug      -  {body}\r\n")
    }

    #[test]
    fn follows_a_growing_file_and_a_second_session() {
        let dir = std::env::temp_dir().join(format!("anonymous-vrx-logs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let store = Store::in_memory().unwrap();

        let first = dir.join("output_log_2026-01-05_20-00-00.txt");
        let mut text = String::new();
        text += &line("20:15:01", "[Behaviour] Entering Room: Quiet Pier at Dusk");
        text += &format!("{JOIN}\r\n");
        text += &line("20:15:09", "[Behaviour] OnPlayerJoined Tester (usr_me)");
        text += &line("20:15:10", "[Behaviour] OnPlayerJoined Pip Halloway (usr_pip)");
        text += "2026.01.05 20:16:00 Debug      -  [Behaviour] OnPlayerJoined Half Writ";
        fs::write(&first, &text).unwrap();
        fs::write(dir.join("unrelated.txt"), "2026.01.05 20:15:01 Debug      -  [Behaviour] OnLeftRoom\n").unwrap();

        assert!(catch_up(&dir, &store).unwrap());
        let here = store.open_visit().unwrap().unwrap();
        assert_eq!(here.world_name, "Quiet Pier at Dusk");
        assert_eq!(here.place.access.as_deref(), Some("friendsPlus"));
        assert_eq!(store.visit_people(here.id).unwrap().len(), 2);
        // Nothing new: nothing recorded, and the half line is still waiting.
        assert!(!catch_up(&dir, &store).unwrap());

        let mut f = fs::OpenOptions::new().append(true).open(&first).unwrap();
        write!(f, "ten (usr_half)\r\n").unwrap();
        f.write_all(line("20:30:00", "[Behaviour] OnPlayerLeft Pip Halloway (usr_pip)").as_bytes()).unwrap();
        drop(f);
        assert!(catch_up(&dir, &store).unwrap());
        let people = store.visit_people(here.id).unwrap();
        assert_eq!(people.len(), 3);
        assert!(people.iter().any(|p| p.display_name == "Half Written" && p.left_at.is_none()));
        assert!(people.iter().any(|p| p.user_id == "usr_pip" && p.left_at.is_some()));

        // The game crashed: no quit line. A new log file starts a new session.
        let second = dir.join("output_log_2026-01-05_21-00-00.txt");
        let mut text = String::new();
        text += &line("21:00:05", "[Behaviour] Entering Room: Basement Arcade");
        text += &line("21:00:06", "[Behaviour] Joining wrld_99999999-2222-3333-4444-555555555555:7~region(us)");
        text += &line("21:00:09", "[Behaviour] OnPlayerJoined Tester (usr_me)");
        text += &line("21:30:00", "[Behaviour] OnLeftRoom");
        fs::write(&second, &text).unwrap();
        assert!(catch_up(&dir, &store).unwrap());

        assert!(store.open_visit().unwrap().is_none());
        let visits = store.visits(10, None).unwrap();
        assert_eq!(visits.len(), 2);
        assert_eq!(visits[0].world_name, "Basement Arcade");
        assert_eq!(visits[0].people, 1);
        assert!(visits[0].left_at.is_some());
        assert_eq!(visits[1].people, 3);
        // The crashed visit ends at the last thing seen in it.
        assert!(visits[1].left_at.as_deref().unwrap() < visits[0].joined_at.as_str());
        assert!(store.visit_people(visits[1].id).unwrap().iter().all(|p| p.left_at.is_some()));

        // Reading again from saved positions adds nothing.
        assert!(!catch_up(&dir, &store).unwrap());
        assert_eq!(store.visits(10, None).unwrap().len(), 2);

        // A deleted log file has its saved position forgotten.
        fs::remove_file(&first).unwrap();
        catch_up(&dir, &store).unwrap();
        assert!(store.setting("log:output_log_2026-01-05_20-00-00.txt").unwrap().is_none());
        assert!(store.setting("log:output_log_2026-01-05_21-00-00.txt").unwrap().is_some());
        let _ = fs::remove_dir_all(&dir);
    }
}
