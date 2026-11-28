//! Reads VRChat's own output log files to learn which instances this PC
//! visited and who was there. Nothing is written to those files.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Local, LocalResult, NaiveDateTime, SecondsFormat, TimeZone, Utc};

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
    /// A video player in the world started a video. VRChat writes this line itself, whichever
    /// player the world uses.
    Video { url: String },
    /// The USharpVideo player names who asked for a video, just before VRChat starts it.
    VideoBy { url: String, name: String },
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
    if let Some(rest) = body
        .strip_prefix("[Video Playback] Attempting to resolve URL '")
        .or_else(|| body.strip_prefix("[Video Playback] Resolving URL '"))
    {
        let url = rest.strip_suffix('\'')?;
        return web_link(url).then(|| (at, LogEvent::Video { url: url.to_string() }));
    }
    if body.contains("USharpVideo") {
        // "Started video load for URL: <link>, requested by <name>"
        let (url, name) = body.split_once("] Started video load for URL: ")?.1.rsplit_once(" requested by ")?;
        let url = url.trim_end_matches(',');
        let name = name.trim();
        return (web_link(url) && !name.is_empty()).then(|| (at, LogEvent::VideoBy { url: url.to_string(), name: name.to_string() }));
    }
    if body.starts_with("VRCApplication: ")
        && (body.contains("OnApplicationQuit at") || body.contains("HandleApplicationQuit at"))
    {
        return Some((at, LogEvent::Quit));
    }
    None
}

/// A plain web address, the only kind of video link kept.
fn web_link(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://"))
        && url.len() <= 2000
        && !url.chars().any(|c| c.is_control() || c.is_whitespace() || c == '\'' || c == '"')
}

/// The eleven-character id of a YouTube video link, for its title and picture. Nothing for
/// any other link.
pub fn youtube_id(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.split(':').next()?.to_ascii_lowercase();
    let host = host.trim_start_matches("www.").trim_start_matches("m.").trim_start_matches("music.");
    let id = if host == "youtu.be" {
        path.split(['?', '#', '/']).next()?
    } else if host == "youtube.com" || host == "youtube-nocookie.com" {
        let (route, query) = path.split_once('?').unwrap_or((path, ""));
        let route = route.split('#').next()?.trim_end_matches('/');
        if route == "watch" {
            query.split('#').next()?.split('&').find_map(|p| p.strip_prefix("v="))?
        } else {
            let mut parts = route.split('/');
            match (parts.next()?, parts.next()?) {
                // A playlist or a channel's live stream, not one video.
                ("embed", "videoseries" | "live_stream") => return None,
                ("shorts" | "live" | "embed" | "v", id) => id,
                _ => return None,
            }
        }
    } else {
        return None;
    };
    let ok = id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    ok.then(|| id.to_string())
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

/// How far the line before may run ahead of a line in the repeated hour before that line is
/// taken to be from the second pass. Lines written at nearly the same moment can come out a
/// second or so out of order, and that must not move them an hour.
const OUT_OF_ORDER: i64 = 60;

/// Picks between the two moments a local time in the repeated hour of the autumn clock change
/// can mean. A log only moves forward, so once the line before is past the first moment, this
/// line is from the second pass.
fn pick(first: DateTime<Utc>, second: DateTime<Utc>, before: Option<DateTime<Utc>>) -> DateTime<Utc> {
    match before {
        Some(before) if before > first + chrono::Duration::seconds(OUT_OF_ORDER) => second,
        _ => first,
    }
}

/// The log uses the PC's local time with no zone. History is stored in UTC. `before` is the
/// moment of the line before in the same file; it settles times in the hour the autumn clock
/// change repeats, and is moved on to this line.
fn to_utc(at: NaiveDateTime, before: &mut Option<DateTime<Utc>>) -> String {
    let utc = match Local.from_local_datetime(&at) {
        LocalResult::Single(local) => local.with_timezone(&Utc),
        LocalResult::Ambiguous(first, second) => pick(first.with_timezone(&Utc), second.with_timezone(&Utc), *before),
        LocalResult::None => Utc.from_utc_datetime(&at),
    };
    *before = Some(utc);
    utc.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// For each log file, where reading stopped and the moment of the last line read there. A new
/// plan is made every pass, so this keeps the repeated hour sorted out from one pass to the next.
static LAST_LINE: Mutex<Option<LastLines>> = Mutex::new(None);

/// Each file's position and the moment of the last line read before it.
type LastLines = HashMap<PathBuf, (u64, DateTime<Utc>)>;

fn last_line(path: &Path, offset: u64) -> Option<DateTime<Utc>> {
    let seen = LAST_LINE.lock().unwrap_or_else(|e| e.into_inner());
    seen.as_ref()?.get(path).filter(|(at, _)| *at == offset).map(|(_, when)| *when)
}

fn remember_last_line(path: &Path, offset: u64, when: Option<DateTime<Utc>>) {
    let Some(when) = when else { return };
    let mut seen = LAST_LINE.lock().unwrap_or_else(|e| e.into_inner());
    seen.get_or_insert_with(HashMap::new).insert(path.to_path_buf(), (offset, when));
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
    /// The moment of the last line read from this file, when known.
    last: Option<DateTime<Utc>>,
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
        let last = last_line(path, offset);
        out.push(Pending { key, path: path.clone(), offset, last });
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
            pending.last = None;
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
        let mut last = pending.last;
        let events: Vec<(String, LogEvent)> = text
            .lines()
            .filter_map(parse_line)
            .map(|(at, event)| (to_utc(at, &mut last), event))
            .collect();
        let fresh_file = pending.offset == 0;
        pending.offset += last_break as u64 + 1;
        pending.last = last;
        remember_last_line(&pending.path, pending.offset, last);
        return Ok(Some(Chunk { events, fresh_file, offset: pending.offset }));
    }
}

/// Each log file the history has already read a part of, and how far. Those parts were read
/// before videos were kept, so they are read once more for their videos alone.
pub fn read_so_far(dir: &Path, store: &Store) -> AppResult<Vec<(PathBuf, u64)>> {
    let Some(files) = log_files(dir) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (name, path) in files {
        let offset: u64 = store.setting(&format!("log:{name}"))?.and_then(|v| v.parse().ok()).unwrap_or(0);
        if offset > 0 {
            out.push((path, offset));
        }
    }
    Ok(out)
}

/// The video lines in the first `upto` bytes of one log file.
pub fn videos_in(path: &Path, upto: u64) -> AppResult<Vec<(String, LogEvent)>> {
    let mut out = Vec::new();
    let mut from = 0u64;
    let mut last = None;
    while from < upto {
        let bytes = read_range(path, from, (upto - from).min(CHUNK)).map_err(|e| AppError::Storage(e.to_string()))?;
        if bytes.is_empty() {
            break;
        }
        let end = match bytes.iter().rposition(|b| *b == b'\n') {
            Some(i) => i + 1,
            None if bytes.len() as u64 >= CHUNK => {
                from += bytes.len() as u64;
                continue;
            }
            None => bytes.len(),
        };
        let text = String::from_utf8_lossy(&bytes[..end]);
        // Every line moves the clock on, not only the video lines, so the repeated hour is
        // read the same way as in `read_next`.
        for (at, e) in text.lines().filter_map(parse_line) {
            let stamp = to_utc(at, &mut last);
            if matches!(e, LogEvent::Video { .. } | LogEvent::VideoBy { .. }) {
                out.push((stamp, e));
            }
        }
        from += end as u64;
    }
    Ok(out)
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
    fn reads_video_lines() {
        let (_, e) = parse_line("2026.01.05 20:15:14 Debug      -  [Video Playback] Attempting to resolve URL 'https://www.youtube.com/watch?v=AbCdEfGhIjK'").unwrap();
        assert_eq!(e, LogEvent::Video { url: "https://www.youtube.com/watch?v=AbCdEfGhIjK".into() });
        let (_, e) = parse_line("2026.01.05 20:15:14 Debug      -  [Video Playback] Resolving URL 'http://example.test/clip.mp4'\r").unwrap();
        assert_eq!(e, LogEvent::Video { url: "http://example.test/clip.mp4".into() });
        let (_, e) = parse_line(
            "2026.01.05 20:15:14 Debug      -  [<color=#9C6994>USharpVideo</color>] Started video load for URL: https://youtu.be/AbCdEfGhIjK, requested by Pip Halloway",
        )
        .unwrap();
        assert_eq!(e, LogEvent::VideoBy { url: "https://youtu.be/AbCdEfGhIjK".into(), name: "Pip Halloway".into() });
    }

    #[test]
    fn the_repeated_hour_follows_the_line_before() {
        let at = |t: &str| DateTime::parse_from_rfc3339(t).unwrap().with_timezone(&Utc);
        // 02:20 local on the autumn change in Europe: 00:20 UTC on the first pass, 01:20 on the second.
        let (first, second) = (at("2026-10-25T00:20:00Z"), at("2026-10-25T01:20:00Z"));
        // Nothing read before, or the line before earlier in the first pass: the first pass.
        assert_eq!(pick(first, second, None), first);
        assert_eq!(pick(first, second, Some(at("2026-10-25T00:10:00Z"))), first);
        // A line a second later written just before this one does not move it an hour.
        assert_eq!(pick(first, second, Some(at("2026-10-25T00:20:01Z"))), first);
        // The line before was 02:50 on the first pass: this one is from the second pass, so a
        // stay joined at 02:50 and left at 02:20 lasts half an hour instead of minus half an hour.
        assert_eq!(pick(first, second, Some(at("2026-10-25T00:50:00Z"))), second);
        // Further into the second pass it stays there.
        assert_eq!(pick(first, second, Some(at("2026-10-25T01:10:00Z"))), second);
    }

    #[test]
    fn finds_youtube_ids() {
        for (url, id) in [
            ("https://www.youtube.com/watch?v=AbCdEfGhIjK", Some("AbCdEfGhIjK")),
            ("https://youtube.com/watch?feature=share&v=AbCdEfGhIjK&t=30", Some("AbCdEfGhIjK")),
            ("https://m.youtube.com/watch/?v=AbCdEfGhIjK#top", Some("AbCdEfGhIjK")),
            ("https://music.youtube.com/watch?v=Ab-dEf_hIjK", Some("Ab-dEf_hIjK")),
            ("https://youtu.be/AbCdEfGhIjK?si=x", Some("AbCdEfGhIjK")),
            ("https://www.youtube.com/shorts/AbCdEfGhIjK", Some("AbCdEfGhIjK")),
            ("https://www.youtube.com/live/AbCdEfGhIjK?feature=x", Some("AbCdEfGhIjK")),
            ("https://www.youtube.com/watch?v=short", None),
            ("https://www.youtube.com/playlist?list=PL1", None),
            ("https://www.youtube.com/embed/AbCdEfGhIjK?autoplay=1", Some("AbCdEfGhIjK")),
            ("https://www.youtube.com/embed/videoseries?list=PL1", None),
            ("https://www.youtube.com/embed/live_stream?channel=UC1", None),
            ("https://notyoutube.com/watch?v=AbCdEfGhIjK", None),
            ("https://example.test/AbCdEfGhIjK", None),
        ] {
            assert_eq!(youtube_id(url).as_deref(), id, "{url}");
        }
    }

    #[test]
    fn ignores_lookalikes_and_noise() {
        for line in [
            "2026.01.05 20:15:02 Debug      -  [Behaviour] Joining or Creating Room: Quiet Pier at Dusk",
            "2026.01.05 20:15:02 Debug      -  [Behaviour] Destination fetching: wrld_1111:1",
            "2026.01.05 20:15:09 Debug      -  [PlayerLog] OnPlayerJoined: Someone (VR=True)",
            "2026.01.05 20:15:09 Debug      -  [Behaviour] OnPlayerJoinComplete Pip",
            "2026.01.05 20:15:14 Debug      -  [Image Download] Attempting to load image from URL 'https://example.test/a.png'",
            "2026.01.05 20:15:14 Debug      -  [String Download] Attempting to load String from URL 'https://example.test/a.txt'",
            "2026.01.05 20:15:15 Debug      -  [Video Playback] URL 'https://youtu.be/AbCdEfGhIjK' resolved to 'https://example.test/x?ip=1'",
            "2026.01.05 20:15:15 Debug      -  [Video Playback] Attempting to resolve URL 'not a link'",
            "2026.01.05 20:15:15 Debug      -  [<color=#9C6994>USharpVideo</color>] Started video: https://youtu.be/AbCdEfGhIjK",
            "2026.01.05 20:15:15 Debug      -  [<color=#9C6994>USharpVideo</color>] USharpVideo v1.0 Initialized",
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
        let videos = store.videos(2000).unwrap();
        println!(
            "{} videos, {} in a known stay, {} with a world name, {} YouTube, {} with who asked",
            videos.len(),
            videos.iter().filter(|v| v.visit_id.is_some()).count(),
            videos.iter().filter(|v| !v.world_name.is_empty()).count(),
            videos.iter().filter(|v| !v.youtube_id.is_empty()).count(),
            videos.iter().filter(|v| !v.requested_by.is_empty()).count()
        );
        let first = store.first_stay().unwrap().unwrap_or_default();
        let (stays, sights) = store.stats_rows(&first, "2100-01-01T00:00:00Z", "usr_nobody", "").unwrap();
        let jd = |t: &str| chrono::DateTime::parse_from_rfc3339(t).map(|t| t.timestamp_millis() as f64 / 86_400_000.0 + 2_440_587.5).unwrap();
        let now = chrono::Utc::now().timestamp_millis() as f64 / 86_400_000.0 + 2_440_587.5;
        let s = crate::stats::work_out(jd(&first), now, &stays, &sights, false);
        println!(
            "stats: {} h, {} stays, {} play days, {} worlds, {} people, capped {}, hours sum ok {}",
            s.total_seconds / 3600, s.stays, s.play_days, s.worlds_count, s.people_count, s.capped,
            s.hours.iter().sum::<i64>() == s.total_seconds
        );
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

        // A video played in the second session; it belongs to that stay, with who asked for it.
        let mut f = fs::OpenOptions::new().append(true).open(&second).unwrap();
        f.write_all(line("21:31:00", "[Behaviour] Entering Room: Movie Night").as_bytes()).unwrap();
        f.write_all(line("21:31:01", "[Behaviour] Joining wrld_77777777-2222-3333-4444-555555555555:3~region(eu)").as_bytes()).unwrap();
        f.write_all(line("21:31:12", "[<color=#9C6994>USharpVideo</color>] Started video load for URL: https://youtu.be/AbCdEfGhIjK, requested by Pip Halloway").as_bytes()).unwrap();
        f.write_all(line("21:31:12", "[Video Playback] Attempting to resolve URL 'https://youtu.be/AbCdEfGhIjK'").as_bytes()).unwrap();
        drop(f);
        assert!(catch_up(&dir, &store).unwrap());
        let videos = store.videos(10).unwrap();
        assert_eq!(videos.len(), 1);
        assert_eq!(videos[0].world_name, "Movie Night");
        assert_eq!(videos[0].requested_by, "Pip Halloway");
        assert_eq!(videos[0].youtube_id, "AbCdEfGhIjK");
        // Reading the old part again for its videos adds no second copy.
        let size = fs::metadata(&second).unwrap().len();
        let again = videos_in(&second, size).unwrap();
        assert_eq!(again.len(), 2);
        store.record_old_videos(&again).unwrap();
        assert_eq!(store.videos(10).unwrap().len(), 1);

        // A deleted log file has its saved position forgotten.
        fs::remove_file(&first).unwrap();
        catch_up(&dir, &store).unwrap();
        assert!(store.setting("log:output_log_2026-01-05_20-00-00.txt").unwrap().is_none());
        assert!(store.setting("log:output_log_2026-01-05_21-00-00.txt").unwrap().is_some());
        let _ = fs::remove_dir_all(&dir);
    }
}
