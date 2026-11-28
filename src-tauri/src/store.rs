//! Local history in SQLite: cached friends, the activity feed and world names.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{AppError, AppResult};
use crate::vrchat::model::{parse_location, FeedDraft, FeedItem, Friend, World};

pub struct Store {
    db: Connection,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS friends (
    user_id    TEXT PRIMARY KEY,
    data       TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS feed (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    at           TEXT NOT NULL,
    user_id      TEXT NOT NULL,
    display_name TEXT NOT NULL,
    kind         TEXT NOT NULL,
    detail       TEXT NOT NULL DEFAULT '',
    previous     TEXT NOT NULL DEFAULT '',
    location     TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS feed_user ON feed(user_id, id);
CREATE TABLE IF NOT EXISTS worlds (
    world_id   TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    image      TEXT NOT NULL DEFAULT '',
    fetched_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS visits (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    location   TEXT NOT NULL,
    world_id   TEXT NOT NULL DEFAULT '',
    world_name TEXT NOT NULL DEFAULT '',
    joined_at  TEXT NOT NULL,
    left_at    TEXT
);
CREATE TABLE IF NOT EXISTS sightings (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    visit_id     INTEGER NOT NULL,
    user_id      TEXT NOT NULL DEFAULT '',
    display_name TEXT NOT NULL,
    joined_at    TEXT NOT NULL,
    left_at      TEXT
);
CREATE TABLE IF NOT EXISTS tags (
    item_id TEXT NOT NULL,
    tag     TEXT NOT NULL,
    PRIMARY KEY (item_id, tag)
);
CREATE TABLE IF NOT EXISTS mutuals (
    a TEXT NOT NULL,
    b TEXT NOT NULL,
    PRIMARY KEY (a, b)
);
CREATE TABLE IF NOT EXISTS mutuals_asked (
    user_id TEXT PRIMARY KEY,
    at      TEXT NOT NULL,
    hidden  INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS world_stats (
    world_id  TEXT NOT NULL,
    day       TEXT NOT NULL,
    visits    INTEGER NOT NULL,
    favorites INTEGER NOT NULL,
    occupants INTEGER NOT NULL,
    PRIMARY KEY (world_id, day)
);
CREATE TABLE IF NOT EXISTS ratings (
    item_id    TEXT PRIMARY KEY,
    stars      INTEGER NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS notes (
    user_id    TEXT PRIMARY KEY,
    note       TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS handled_notices (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    at         TEXT NOT NULL,
    action     TEXT NOT NULL,
    reply_text TEXT NOT NULL DEFAULT '',
    notice     TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS avatar_history (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    avatar_id      TEXT NOT NULL,
    name           TEXT NOT NULL DEFAULT '',
    author_id      TEXT NOT NULL DEFAULT '',
    author_name    TEXT NOT NULL DEFAULT '',
    image          TEXT NOT NULL DEFAULT '',
    release_status TEXT NOT NULL DEFAULT '',
    worn_at        TEXT NOT NULL,
    ended_at       TEXT,
    imported_seconds INTEGER
);
CREATE INDEX IF NOT EXISTS avatar_history_avatar ON avatar_history(avatar_id);
CREATE TABLE IF NOT EXISTS local_favorites (
    kind        TEXT NOT NULL,
    item_id     TEXT NOT NULL,
    name        TEXT NOT NULL DEFAULT '',
    author_name TEXT NOT NULL DEFAULT '',
    image       TEXT NOT NULL DEFAULT '',
    added_at    TEXT NOT NULL,
    PRIMARY KEY (kind, item_id)
);
CREATE TABLE IF NOT EXISTS own_avatars (
    avatar_id  TEXT PRIMARY KEY,
    name       TEXT NOT NULL DEFAULT '',
    image      TEXT NOT NULL DEFAULT '',
    first_seen TEXT NOT NULL,
    last_seen  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS feed_at ON feed(at);
CREATE INDEX IF NOT EXISTS sightings_visit ON sightings(visit_id);
CREATE INDEX IF NOT EXISTS sightings_user ON sightings(user_id, id);
CREATE INDEX IF NOT EXISTS sightings_name ON sightings(display_name, user_id);
CREATE INDEX IF NOT EXISTS visits_world ON visits(world_id);
CREATE INDEX IF NOT EXISTS visits_name ON visits(world_name);
-- Finds the stay a moment falls in (a photo, a video) without reading every stay. The schema
-- runs on every open, so an older history file gets it the first time it is opened.
CREATE INDEX IF NOT EXISTS visits_joined ON visits(joined_at, id);
CREATE TABLE IF NOT EXISTS videos (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    visit_id     INTEGER,
    at           TEXT NOT NULL,
    url          TEXT NOT NULL,
    youtube_id   TEXT NOT NULL DEFAULT '',
    requested_by TEXT NOT NULL DEFAULT '',
    UNIQUE (at, url)
);
CREATE INDEX IF NOT EXISTS videos_visit ON videos(visit_id);
CREATE INDEX IF NOT EXISTS feed_friend_added ON feed(user_id, at) WHERE kind = 'friendAdded';
-- Let a VRCX import find the entries one person already has of one kind, and the stays
-- already here in one instance, without reading the whole list for every line it brings.
CREATE INDEX IF NOT EXISTS feed_user_kind ON feed(user_id, kind, at);
CREATE INDEX IF NOT EXISTS visits_location ON visits(location, joined_at);
CREATE TABLE IF NOT EXISTS video_titles (
    video_id   TEXT PRIMARY KEY,
    title      TEXT NOT NULL DEFAULT '',
    channel    TEXT NOT NULL DEFAULT '',
    fetched_at TEXT NOT NULL
);
";

/// A video a world played while I was there, taken from the game log.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoPlay {
    pub id: i64,
    pub at: String,
    pub url: String,
    pub youtube_id: String,
    pub requested_by: String,
    pub visit_id: Option<i64>,
    pub world_id: String,
    pub world_name: String,
    pub location: String,
    pub joined_at: Option<String>,
    pub title: String,
    pub channel: String,
    /// When the stay it played in ended; empty while it goes on.
    pub left_at: Option<String>,
}

/// The title YouTube gave a video. Empty when YouTube had none to give (a removed or private video).
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoTitle {
    pub id: String,
    pub title: String,
    pub channel: String,
}

const VIDEO_SELECT: &str = "SELECT p.id, p.at, p.url, p.youtube_id, p.requested_by, p.visit_id,
    COALESCE(v.world_id, ''), COALESCE(NULLIF(v.world_name, ''), w.name, ''), COALESCE(v.location, ''), v.joined_at,
    COALESCE(t.title, ''), COALESCE(t.channel, ''), v.left_at
    FROM videos p LEFT JOIN visits v ON v.id = p.visit_id LEFT JOIN worlds w ON w.world_id = v.world_id
    LEFT JOIN video_titles t ON t.video_id = p.youtube_id AND p.youtube_id <> ''";

fn video_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<VideoPlay> {
    Ok(VideoPlay {
        id: r.get(0)?,
        at: r.get(1)?,
        url: r.get(2)?,
        youtube_id: r.get(3)?,
        requested_by: r.get(4)?,
        visit_id: r.get(5)?,
        world_id: r.get(6)?,
        world_name: r.get(7)?,
        location: r.get(8)?,
        joined_at: r.get(9)?,
        title: r.get(10)?,
        channel: r.get(11)?,
        left_at: r.get(12)?,
    })
}

/// Where the game-log reader keeps the name of whoever put a video on, until its video line is read.
const ASKED_BY: &str = "video:asked-by";

const INSERT_VIDEO: &str = "INSERT OR IGNORE INTO videos (visit_id, at, url, youtube_id, requested_by) VALUES (?1, ?2, ?3, ?4, ?5)";

/// Who asked for a video, when the line just before named the same link.
fn asked_by(pending: &mut Option<(String, String)>, url: &str) -> String {
    match pending.take() {
        Some((asked, name)) if asked == url => name,
        _ => String::new(),
    }
}

/// One stay in an instance, taken from the game log.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Visit {
    pub id: i64,
    pub location: String,
    pub world_id: String,
    pub world_name: String,
    pub joined_at: String,
    pub left_at: Option<String>,
    pub place: crate::vrchat::model::Place,
    /// How many different people were seen during the stay, me included.
    pub people: i64,
}

/// One avatar I wore, with how often and how long.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarWorn {
    pub avatar_id: String,
    pub name: String,
    pub author_id: String,
    pub author_name: String,
    pub image: String,
    pub release_status: String,
    pub last_worn: String,
    pub times: i64,
    /// Time between switching to it and switching away, each stay capped at 12 hours.
    /// Only wearings this app recorded.
    pub seconds: i64,
    /// The total wear time VRCX had noted for it when it was brought over. VRCX keeps no
    /// times for that, so it says nothing about which world stays it was worn in and is kept
    /// apart from the numbers below.
    pub imported_seconds: i64,
    /// Whether it is the avatar being worn now, as far as this app knows.
    pub wearing: bool,
    /// The part of that time I was inside a world, from the world history. This is the wear
    /// time people mean: hours with VRChat closed do not count.
    pub seconds_in_game: i64,
    /// How many stays in a world I wore it in.
    pub visits: i64,
    /// The last moment I had it on inside a world (UTC), empty if never.
    pub last_in_game: String,
}

/// An avatar of mine that was on my account before and is not on it now.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LostAvatar {
    pub avatar_id: String,
    pub name: String,
    pub image: String,
    pub last_seen: String,
}

/// A favourite kept only on this PC, so it has no limit.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalFavorite {
    pub kind: String,
    pub item_id: String,
    pub name: String,
    pub author_name: String,
    pub image: String,
    #[serde(default)]
    pub added_at: String,
}

/// Feed kinds a search may filter on.
const FEED_KINDS: &[&str] = &[
    "online", "offline", "location", "status", "statusText", "name", "avatar", "friendAdded", "friendRemoved",
];

/// A person found in my own history.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundPerson {
    pub user_id: String,
    /// The name they were last seen under.
    pub name: String,
    /// The name that matched the search; another one when they have renamed since.
    pub matched: String,
    pub times: i64,
    pub seconds: i64,
    pub first: String,
    pub last: String,
    pub last_world_id: String,
    pub last_world: String,
    pub last_location: String,
}

/// A world found in my own history.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundWorld {
    pub world_id: String,
    pub name: String,
    pub times: i64,
    pub seconds: i64,
    pub last: String,
    pub last_location: String,
}

/// One stay of mine, as a search result: how long the person and I overlapped, or how long I was there.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogStay {
    pub visit_id: i64,
    pub world_id: String,
    pub world_name: String,
    pub location: String,
    pub from: String,
    pub to: String,
    pub seconds: i64,
    /// Different people seen during the stay, me included.
    pub people: i64,
}

fn stay_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<LogStay> {
    Ok(LogStay {
        visit_id: r.get(0)?,
        world_id: r.get(1)?,
        world_name: r.get(2)?,
        location: r.get(3)?,
        from: r.get(4)?,
        to: r.get(5)?,
        seconds: r.get::<_, f64>(6)?.max(0.0).round() as i64,
        people: r.get(7)?,
    })
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HabitWorld {
    pub world_id: String,
    pub name: String,
    pub times: i64,
    pub seconds: i64,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HabitPerson {
    pub user_id: String,
    pub name: String,
    pub times: i64,
}

/// Where and when I meet a person, and where they go by themselves.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Habits {
    /// Worlds we were in together most.
    pub worlds: Vec<HabitWorld>,
    /// Who else was usually there at the same time.
    pub company: Vec<HabitPerson>,
    /// When we were together: start and seconds, newest first.
    pub together: Vec<(String, i64)>,
    /// Worlds the friend activity shows them going to most.
    pub own_worlds: Vec<HabitWorld>,
    /// When the friend activity saw them come online or move.
    pub seen_at: Vec<String>,
}

/// `fold(text)`: the text in lower case for every alphabet, so a search finds "Über" from "über"
/// and Cyrillic or Greek names whatever their case. SQLite's own LIKE folds only A to Z.
fn add_fold(db: &Connection) -> rusqlite::Result<()> {
    db.create_scalar_function("fold", 1, rusqlite::functions::FunctionFlags::SQLITE_UTF8 | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC, |c| {
        Ok(c.get::<Option<String>>(0)?.map(|s| s.to_lowercase()))
    })
}

fn like_pattern(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    format!("%{escaped}%")
}

/// One person seen during a visit.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sighting {
    pub user_id: String,
    pub display_name: String,
    pub joined_at: String,
    pub left_at: Option<String>,
}

const VISIT_COLUMNS: &str = "v.id, v.location, v.world_id, v.world_name, v.joined_at, v.left_at,
    (SELECT COUNT(DISTINCT CASE WHEN s.user_id = '' THEN s.display_name ELSE s.user_id END)
     FROM sightings s WHERE s.visit_id = v.id)";

fn visit_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Visit> {
    let location: String = r.get(1)?;
    Ok(Visit {
        id: r.get(0)?,
        place: parse_location(&location),
        location,
        world_id: r.get(2)?,
        world_name: r.get(3)?,
        joined_at: r.get(4)?,
        left_at: r.get(5)?,
        people: r.get(6)?,
    })
}

/// Ends every visit that is still open. With no time given, a visit ends at
/// the last thing seen in it, which is the best guess after a crash.
///
/// A stay never ends before it began. When the clocks go back, a log line read after the
/// change can carry an earlier time than the join; the end is then kept at the join.
fn close_visits(db: &Connection, at: Option<&str>) -> rusqlite::Result<()> {
    match at {
        Some(at) => {
            db.execute(
                "UPDATE sightings SET left_at = CASE WHEN julianday(?1) < julianday(joined_at) THEN joined_at ELSE ?1 END
                 WHERE left_at IS NULL AND visit_id IN (SELECT id FROM visits WHERE left_at IS NULL)",
                params![at],
            )?;
            db.execute(
                "UPDATE visits SET left_at = CASE WHEN julianday(?1) < julianday(joined_at) THEN joined_at ELSE ?1 END
                 WHERE left_at IS NULL",
                params![at],
            )?;
        }
        None => {
            db.execute(
                "UPDATE visits SET left_at = COALESCE(
                    (SELECT MAX(COALESCE(s.left_at, s.joined_at)) FROM sightings s WHERE s.visit_id = visits.id
                     AND julianday(COALESCE(s.left_at, s.joined_at)) >= julianday(visits.joined_at)),
                    joined_at)
                 WHERE left_at IS NULL",
                [],
            )?;
            db.execute(
                "UPDATE sightings SET left_at = (SELECT v.left_at FROM visits v WHERE v.id = sightings.visit_id)
                 WHERE left_at IS NULL",
                [],
            )?;
        }
    }
    Ok(())
}

/// Gives an older history file the `imported_seconds` column on avatar_history, once.
///
/// Before it, an avatar brought over from VRCX was written as made-up wearings of twelve hours
/// each, ending when it was last put on. Those claimed world stays they had nothing to do with.
/// They are told apart from wearings this app recorded by how they end: a wearing recorded
/// here always ends at the moment the next avatar was put on, so another avatar's row starts
/// at exactly that time. Rows with no such neighbour are folded into one imported row per
/// avatar, keeping their total time.
fn upgrade_avatar_history(db: &Connection) -> rusqlite::Result<()> {
    let has_column: i64 = db.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('avatar_history') WHERE name = 'imported_seconds'",
        [],
        |r| r.get(0),
    )?;
    if has_column > 0 {
        return Ok(());
    }
    let tx = db.unchecked_transaction()?;
    tx.execute_batch(
        "ALTER TABLE avatar_history ADD COLUMN imported_seconds INTEGER;
         CREATE TEMP TABLE guessed AS
             SELECT h.id, h.avatar_id, h.worn_at, h.ended_at FROM avatar_history h
             WHERE h.ended_at IS NOT NULL
               AND NOT EXISTS (SELECT 1 FROM avatar_history o WHERE o.worn_at = h.ended_at AND o.avatar_id != h.avatar_id);
         CREATE TEMP TABLE brought AS
             SELECT g.avatar_id,
                    (SELECT COALESCE(MAX(name), '') FROM avatar_history WHERE avatar_id = g.avatar_id) AS name,
                    (SELECT COALESCE(MAX(author_id), '') FROM avatar_history WHERE avatar_id = g.avatar_id) AS author_id,
                    (SELECT COALESCE(MAX(author_name), '') FROM avatar_history WHERE avatar_id = g.avatar_id) AS author_name,
                    (SELECT COALESCE(MAX(image), '') FROM avatar_history WHERE avatar_id = g.avatar_id) AS image,
                    (SELECT COALESCE(MAX(release_status), '') FROM avatar_history WHERE avatar_id = g.avatar_id) AS release_status,
                    strftime('%Y-%m-%dT%H:%M:%fZ', MAX(julianday(g.ended_at))) AS last,
                    CAST(ROUND(SUM(MAX(0, (julianday(g.ended_at) - julianday(g.worn_at)) * 86400))) AS INTEGER) AS seconds
             FROM guessed g GROUP BY g.avatar_id;
         DELETE FROM avatar_history WHERE id IN (SELECT id FROM guessed);
         INSERT INTO avatar_history (avatar_id, name, author_id, author_name, image, release_status, worn_at, ended_at, imported_seconds)
             SELECT avatar_id, name, author_id, author_name, image, release_status, last, last, seconds FROM brought ORDER BY julianday(last);
         DROP TABLE guessed;
         DROP TABLE brought;",
    )?;
    tx.commit()
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn feed_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<FeedItem> {
    let location: String = r.get(7)?;
    Ok(FeedItem {
        id: r.get(0)?,
        at: r.get(1)?,
        user_id: r.get(2)?,
        display_name: r.get(3)?,
        kind: r.get(4)?,
        detail: r.get(5)?,
        previous: r.get(6)?,
        world_id: parse_location(&location).world_id,
        location,
    })
}

fn world_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<World> {
    Ok(World {
        id: r.get(0)?,
        name: r.get(1)?,
        image: r.get(2)?,
    })
}

impl Store {
    pub fn open(path: &Path) -> AppResult<Store> {
        let mut db = Connection::open(path)?;
        // A VRCX import writes through a second connection to the same file. A write that finds
        // the file busy waits for it up to ten seconds instead of failing at once.
        db.busy_timeout(std::time::Duration::from_secs(10))?;
        // A transaction takes the write lock when it begins. One that read first and wanted to
        // write later could otherwise fail outright when the other connection wrote in between,
        // because waiting would not help it.
        db.set_transaction_behavior(rusqlite::TransactionBehavior::Immediate);
        add_fold(&db)?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        // After a big write the log file is cut back to this size, so it does not stay as
        // large as the biggest job ever was while the app is open.
        db.pragma_update(None, "journal_size_limit", 64 * 1024 * 1024)?;
        db.execute_batch(SCHEMA)?;
        upgrade_avatar_history(&db)?;
        let store = Store { db };
        // An import that was cut off before it put the lists back in order of time is finished
        // here. When nothing was left undone this is one quick look. If the file is busy just
        // then, the history still opens and the next import finishes the job instead.
        let _ = store.finish_renumber();
        Ok(store)
    }

    /// Opens a history file that must already exist, without changing it.
    pub fn open_existing(path: &Path) -> AppResult<Store> {
        let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        add_fold(&db)?;
        db.query_row("SELECT COUNT(*) FROM feed", [], |r| r.get::<_, i64>(0))?;
        Ok(Store { db })
    }

    #[cfg(test)]
    pub fn in_memory() -> AppResult<Store> {
        let db = Connection::open_in_memory()?;
        add_fold(&db)?;
        db.execute_batch(SCHEMA)?;
        Ok(Store { db })
    }

    /// Runs plain SQL on a test store, for tests in other files that need rows no public
    /// function writes.
    #[cfg(test)]
    pub fn run_sql(&self, sql: &str) -> AppResult<()> {
        self.db.execute_batch(sql)?;
        Ok(())
    }

    /// Replaces the cached friend list with the given one.
    pub fn replace_friends(&mut self, friends: &[Friend]) -> AppResult<()> {
        let tx = self.db.transaction()?;
        tx.execute("DELETE FROM friends", [])?;
        {
            let mut stmt =
                tx.prepare("INSERT INTO friends (user_id, data, updated_at) VALUES (?1, ?2, ?3)")?;
            let at = now();
            for f in friends {
                let data = serde_json::to_string(f).unwrap_or_default();
                stmt.execute(params![f.id, data, at])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn save_friend(&self, f: &Friend) -> AppResult<()> {
        let data = serde_json::to_string(f).unwrap_or_default();
        self.db.execute(
            "INSERT INTO friends (user_id, data, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(user_id) DO UPDATE SET data = excluded.data, updated_at = excluded.updated_at",
            params![f.id, data, now()],
        )?;
        Ok(())
    }

    pub fn remove_friend(&self, user_id: &str) -> AppResult<()> {
        self.db
            .execute("DELETE FROM friends WHERE user_id = ?1", params![user_id])?;
        Ok(())
    }

    pub fn friends(&self) -> AppResult<Vec<Friend>> {
        let mut stmt = self.db.prepare("SELECT data FROM friends")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            if let Ok(f) = serde_json::from_str::<Friend>(&row?) {
                out.push(f);
            }
        }
        Ok(out)
    }

    pub fn add_feed(&self, d: &FeedDraft) -> AppResult<FeedItem> {
        let at = now();
        self.db.execute(
            "INSERT INTO feed (at, user_id, display_name, kind, detail, previous, location)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![at, d.user_id, d.display_name, d.kind, d.detail, d.previous, d.location],
        )?;
        Ok(FeedItem {
            id: self.db.last_insert_rowid(),
            at,
            user_id: d.user_id.clone(),
            display_name: d.display_name.clone(),
            kind: d.kind.to_string(),
            detail: d.detail.clone(),
            previous: d.previous.clone(),
            world_id: parse_location(&d.location).world_id,
            location: d.location.clone(),
        })
    }

    /// My own note about someone. Empty when there is none.
    pub fn note(&self, user_id: &str) -> AppResult<String> {
        Ok(self
            .db
            .query_row("SELECT note FROM notes WHERE user_id = ?1", params![user_id], |r| r.get(0))
            .optional()?
            .unwrap_or_default())
    }

    /// Saves a note, or removes it when the text is empty.
    pub fn set_note(&self, user_id: &str, note: &str) -> AppResult<()> {
        if note.trim().is_empty() {
            self.db.execute("DELETE FROM notes WHERE user_id = ?1", params![user_id])?;
        } else {
            self.db.execute(
                "INSERT INTO notes (user_id, note, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(user_id) DO UPDATE SET note = excluded.note, updated_at = excluded.updated_at",
                params![user_id, note, now()],
            )?;
        }
        Ok(())
    }

    /// Every user id that has a note, for marking them in lists.
    pub fn noted(&self) -> AppResult<Vec<String>> {
        let mut stmt = self.db.prepare("SELECT user_id FROM notes")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Time spent in the same instance as someone, from the game-log history. Rows of the same
    /// person that overlap (a doubled log line, an imported copy) count once.
    pub fn together(&self, user_id: &str) -> AppResult<crate::vrchat::people::Together> {
        let at = now();
        let seconds: i64 = self.spans("s.user_id = ?1", user_id, &at)?.iter().map(|s| s.2).sum();
        let mut found: crate::vrchat::people::Together = self.db.query_row(
            "SELECT COUNT(DISTINCT s.visit_id),
                    COALESCE(SUM((julianday(COALESCE(s.left_at, v.left_at, ?2)) - julianday(s.joined_at)) * 86400), 0),
                    MIN(s.joined_at),
                    MAX(COALESCE(s.left_at, v.left_at, ?2))
             FROM sightings s JOIN visits v ON v.id = s.visit_id
             WHERE s.user_id = ?1",
            params![user_id, at],
            |r| {
                Ok(crate::vrchat::people::Together {
                    times: r.get(0)?,
                    seconds: r.get::<_, f64>(1)?.max(0.0).round() as i64,
                    first: r.get(2)?,
                    last: r.get(3)?,
                })
            },
        )?;
        found.seconds = seconds;
        Ok(found)
    }

    /// The first time each of my current friends shows up as added in the friend log: who,
    /// and when (UTC). A friend removed and added again keeps the first date.
    pub fn friends_since_all(&self) -> AppResult<Vec<(String, String)>> {
        let mut stmt = self.db.prepare(
            "SELECT fe.user_id, MIN(fe.at) FROM feed fe JOIN friends f ON f.user_id = fe.user_id
             WHERE fe.kind = 'friendAdded' GROUP BY fe.user_id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The first time one person shows up as added in the friend log.
    pub fn friends_since(&self, user_id: &str) -> AppResult<Option<String>> {
        Ok(self
            .db
            .query_row("SELECT MIN(at) FROM feed WHERE user_id = ?1 AND kind = 'friendAdded'", params![user_id], |r| r.get(0))?)
    }

    /// My stays and the people in them between two times (UTC), for the numbers page.
    pub fn stats_rows(&self, from: &str, to: &str, my_id: &str, my_name: &str) -> AppResult<(Vec<crate::stats::StayRow>, Vec<crate::stats::SightRow>)> {
        let mut stmt = self.db.prepare(
            "SELECT v.id, v.world_id, COALESCE(NULLIF(v.world_name, ''), w.name, ''), v.joined_at, julianday(v.joined_at), julianday(v.left_at),
                    (SELECT julianday(MAX(COALESCE(s.left_at, s.joined_at))) FROM sightings s WHERE s.visit_id = v.id)
             FROM visits v LEFT JOIN worlds w ON w.world_id = v.world_id
             WHERE julianday(COALESCE(v.left_at, ?2)) > julianday(?1) AND julianday(v.joined_at) < julianday(?2)
             ORDER BY julianday(v.joined_at), v.id",
        )?;
        let stays = stmt
            .query_map(params![from, to], |r| {
                Ok(crate::stats::StayRow {
                    visit_id: r.get(0)?,
                    world_id: r.get(1)?,
                    world_name: r.get(2)?,
                    joined_at: r.get(3)?,
                    start: r.get::<_, Option<f64>>(4)?.unwrap_or(0.0),
                    left: r.get(5)?,
                    last_seen: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|s| s.start > 0.0)
            .collect();
        let mut stmt = self.db.prepare(
            "SELECT CASE WHEN s.user_id <> '' THEN s.user_id ELSE ' ' || s.display_name END AS k, s.user_id, s.display_name, s.visit_id,
                    julianday(s.joined_at), julianday(COALESCE(s.left_at, v.left_at, ?2)), f.user_id IS NOT NULL
             FROM sightings s JOIN visits v ON v.id = s.visit_id
             LEFT JOIN friends f ON f.user_id = s.user_id AND s.user_id <> ''
             WHERE s.user_id <> ?3
               -- An old log line without an id that carries one of my own names is me.
               AND NOT (s.user_id = '' AND (s.display_name = ?4 OR s.display_name IN (SELECT display_name FROM sightings WHERE user_id = ?3 AND ?3 <> '')))
               AND julianday(COALESCE(s.left_at, v.left_at, ?2)) > julianday(?1) AND julianday(s.joined_at) < julianday(?2)
             ORDER BY k, julianday(s.joined_at), s.id",
        )?;
        let sights = stmt
            .query_map(params![from, to, my_id, my_name], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?, r.get::<_, Option<f64>>(4)?, r.get::<_, Option<f64>>(5)?, r.get::<_, bool>(6)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter_map(|(key, user_id, name, visit_id, start, end, friend)| Some(crate::stats::SightRow { key, user_id, name, visit_id, start: start?, end: end?, friend }))
            .collect();
        Ok((stays, sights))
    }

    /// When the oldest stay began (UTC), for "all time".
    pub fn first_stay(&self) -> AppResult<Option<String>> {
        Ok(self.db.query_row("SELECT joined_at FROM visits ORDER BY julianday(joined_at) LIMIT 1", [], |r| r.get(0)).optional()?)
    }

    /// One person's feed entries, newest first.
    pub fn feed_for(&self, user_id: &str, limit: u32) -> AppResult<Vec<FeedItem>> {
        let mut stmt = self.db.prepare(
            "SELECT id, at, user_id, display_name, kind, detail, previous, location
             FROM feed WHERE user_id = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![user_id, limit.min(500)], feed_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The friend log: friends added, removed and renamed, newest first.
    pub fn friend_log(&self, limit: u32) -> AppResult<Vec<FeedItem>> {
        let mut stmt = self.db.prepare(
            "SELECT id, at, user_id, display_name, kind, detail, previous, location
             FROM feed WHERE kind IN ('friendAdded', 'friendRemoved', 'name') ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit.min(2000)], feed_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// When the feed last recorded this friend arriving at this location.
    pub fn arrived_at(&self, user_id: &str, location: &str) -> AppResult<Option<String>> {
        Ok(self
            .db
            .query_row(
                "SELECT at, location FROM feed WHERE user_id = ?1 AND kind IN ('location', 'online')
                 ORDER BY id DESC LIMIT 1",
                params![user_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
            .filter(|(_, last)| last == location)
            .map(|(at, _)| at))
    }

    /// Newest first. `before` pages back through older entries.
    pub fn feed(&self, limit: u32, before: Option<i64>) -> AppResult<Vec<FeedItem>> {
        let mut stmt = self.db.prepare(
            "SELECT id, at, user_id, display_name, kind, detail, previous, location
             FROM feed WHERE id < ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![before.unwrap_or(i64::MAX), limit.min(500)], |r| {
            let location: String = r.get(7)?;
            Ok(FeedItem {
                id: r.get(0)?,
                at: r.get(1)?,
                user_id: r.get(2)?,
                display_name: r.get(3)?,
                kind: r.get(4)?,
                detail: r.get(5)?,
                previous: r.get(6)?,
                world_id: parse_location(&location).world_id,
                location,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The feed with filters: a name or text to look for, kinds, and an upper time bound.
    /// Newest first; `before` pages back by id.
    pub fn feed_search(
        &self,
        query: &str,
        kinds: &[String],
        before: Option<i64>,
        until: Option<&str>,
        limit: u32,
    ) -> AppResult<Vec<FeedItem>> {
        let mut sql = String::from(
            "SELECT id, at, user_id, display_name, kind, detail, previous, location FROM feed WHERE id < ? AND at < ?",
        );
        let mut args: Vec<String> = vec![before.unwrap_or(i64::MAX).to_string(), until.unwrap_or("9999").to_string()];
        let q = query.trim();
        if !q.is_empty() {
            // fold() and the lower-case pattern ignore case in every alphabet, as the other
            // history searches do; LIKE alone only folds A to Z.
            let pattern = like_pattern(&q.to_lowercase());
            sql.push_str(
                " AND (fold(display_name) LIKE ? ESCAPE '\\' OR fold(detail) LIKE ? ESCAPE '\\' OR fold(previous) LIKE ? ESCAPE '\\'",
            );
            args.extend([pattern.clone(), pattern.clone(), pattern.clone()]);
            // World names live in their own table; match those worlds by id.
            let mut stmt = self.db.prepare("SELECT world_id FROM worlds WHERE fold(name) LIKE ?1 ESCAPE '\\' LIMIT 40")?;
            let ids = stmt
                .query_map(params![pattern], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            for id in ids {
                sql.push_str(" OR location LIKE ?");
                args.push(format!("{id}%"));
            }
            sql.push(')');
        }
        let wanted: Vec<&String> = kinds.iter().filter(|k| FEED_KINDS.contains(&k.as_str())).collect();
        if !wanted.is_empty() {
            sql.push_str(" AND kind IN (");
            sql.push_str(&vec!["?"; wanted.len()].join(", "));
            sql.push(')');
            args.extend(wanted.into_iter().cloned());
        }
        sql.push_str(" ORDER BY id DESC LIMIT ?");
        args.push(limit.clamp(1, 500).to_string());
        let mut stmt = self.db.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), feed_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Records that I switched to an avatar. Returns false when it is the one already being worn.
    pub fn wear_avatar(&self, avatar_id: &str, at: &str) -> AppResult<bool> {
        let current: Option<String> = self
            .db
            .query_row(
                "SELECT avatar_id FROM avatar_history WHERE ended_at IS NULL ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if current.as_deref() == Some(avatar_id) {
            return Ok(false);
        }
        self.db
            .execute("UPDATE avatar_history SET ended_at = ?1 WHERE ended_at IS NULL", params![at])?;
        // Whatever is already known about this avatar is copied onto the new row.
        self.db.execute(
            "INSERT INTO avatar_history (avatar_id, name, author_id, author_name, image, release_status, worn_at)
             SELECT ?1, COALESCE(MAX(name), ''), COALESCE(MAX(author_id), ''), COALESCE(MAX(author_name), ''),
                    COALESCE(MAX(image), ''), COALESCE(MAX(release_status), ''), ?2
             FROM avatar_history WHERE avatar_id = ?1",
            params![avatar_id, at],
        )?;
        Ok(true)
    }

    /// Fills in an avatar's details on every row for it.
    pub fn fill_avatar(
        &self,
        avatar_id: &str,
        name: &str,
        author_id: &str,
        author_name: &str,
        image: &str,
        release_status: &str,
    ) -> AppResult<()> {
        self.db.execute(
            "UPDATE avatar_history SET name = ?2, author_id = ?3, author_name = ?4, image = ?5, release_status = ?6
             WHERE avatar_id = ?1",
            params![avatar_id, name, author_id, author_name, image, release_status],
        )?;
        Ok(())
    }

    /// Avatars I wore, one row per avatar: the one on now first, then by when each was last
    /// put on. That time decides, not the order rows were written in, because history brought
    /// over from VRCX is written after wearings this app already recorded.
    pub fn avatar_history(&self, limit: u32) -> AppResult<Vec<AvatarWorn>> {
        let mut stmt = self.db.prepare(
            "SELECT avatar_id, MAX(name), MAX(author_id), MAX(author_name), MAX(image), MAX(release_status),
                    MAX(worn_at), COUNT(*),
                    CAST(SUM(CASE WHEN imported_seconds IS NULL THEN MIN(43200, MAX(0,
                        (julianday(COALESCE(ended_at, ?1)) - julianday(worn_at)) * 86400)) ELSE 0 END) AS INTEGER),
                    COALESCE(SUM(imported_seconds), 0),
                    MAX(ended_at IS NULL AND imported_seconds IS NULL)
             FROM avatar_history GROUP BY avatar_id
             ORDER BY MAX(ended_at IS NULL AND imported_seconds IS NULL) DESC, MAX(julianday(worn_at)) DESC, MAX(id) DESC
             LIMIT ?2",
        )?;
        let in_game = self.avatar_time_in_worlds(&now())?;
        let rows = stmt.query_map(params![now(), limit.clamp(1, 1000)], |r| {
            let avatar_id: String = r.get(0)?;
            let (seconds_in_game, visits, last_in_game) = in_game.get(&avatar_id).cloned().unwrap_or_default();
            Ok(AvatarWorn {
                seconds_in_game,
                visits,
                last_in_game,
                avatar_id: r.get(0)?,
                name: r.get(1)?,
                author_id: r.get(2)?,
                author_name: r.get(3)?,
                image: r.get(4)?,
                release_status: r.get(5)?,
                last_worn: r.get(6)?,
                times: r.get(7)?,
                seconds: r.get::<_, Option<i64>>(8)?.unwrap_or(0),
                imported_seconds: r.get(9)?,
                wearing: r.get::<_, i64>(10)? != 0,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// For each avatar, the seconds its wearing times overlap my stays in worlds, how many
    /// stays that was, and the last moment of overlap. A stay or a wearing that was never
    /// closed runs until `now`, and each overlap counts 12 hours at most. Rows brought over
    /// from VRCX carry no real wearing times, so they claim no stays.
    ///
    /// Joining every wearing to every stay in SQL took seconds on a long history, so the stays
    /// are sorted once by when they began and each wearing looks only at the stays that can
    /// touch it. Times are turned into day numbers by SQLite itself, as the join did.
    fn avatar_time_in_worlds(&self, now: &str) -> AppResult<std::collections::HashMap<String, (i64, i64, String)>> {
        use std::collections::{HashMap, HashSet};
        // A stay of more than a day is rare (one never closed, a crash); those are checked
        // against every wearing, the rest only when they began at most a day before it.
        const DAY: f64 = 1.0;
        let mut short: Vec<(f64, f64, i64)> = Vec::new();
        let mut long: Vec<(f64, f64, i64)> = Vec::new();
        {
            let mut stmt = self.db.prepare("SELECT id, julianday(joined_at), julianday(COALESCE(left_at, ?1)) FROM visits")?;
            let mut rows = stmt.query(params![now])?;
            while let Some(r) = rows.next()? {
                // A time SQLite cannot read never overlaps anything, as in the join.
                let (Some(from), Some(to)) = (r.get::<_, Option<f64>>(1)?, r.get::<_, Option<f64>>(2)?) else { continue };
                let stay = (from, to, r.get::<_, i64>(0)?);
                if to - from > DAY {
                    long.push(stay);
                } else {
                    short.push(stay);
                }
            }
        }
        short.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Per avatar: the summed seconds, the stays it touched and the latest end of an overlap.
        let mut found: HashMap<String, (f64, HashSet<i64>, f64)> = HashMap::new();
        let mut stmt = self.db.prepare(
            "SELECT avatar_id, julianday(worn_at), julianday(COALESCE(ended_at, ?1)) FROM avatar_history WHERE imported_seconds IS NULL",
        )?;
        let mut rows = stmt.query(params![now])?;
        while let Some(r) = rows.next()? {
            let (Some(on), Some(off)) = (r.get::<_, Option<f64>>(1)?, r.get::<_, Option<f64>>(2)?) else { continue };
            let avatar: String = r.get(0)?;
            // Short stays that ended after `on` began after `on` minus a day; the small margin
            // keeps a stay of exactly one day that rounding pushed a hair over the line.
            let first = short.partition_point(|s| s.0 < on - DAY - 1e-6);
            let past = short.partition_point(|s| s.0 < off);
            for &(from, to, id) in short[first..past].iter().chain(long.iter()) {
                if !(from < off && to > on) {
                    continue;
                }
                let end = off.min(to);
                let seconds = ((end - on.max(from)) * 86400.0).clamp(0.0, 43200.0);
                let entry = found.entry(avatar.clone()).or_insert_with(|| (0.0, HashSet::new(), f64::MIN));
                entry.0 += seconds;
                entry.1.insert(id);
                entry.2 = entry.2.max(end);
            }
        }
        let mut stamp = self.db.prepare("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', ?1)")?;
        let mut out = HashMap::new();
        for (avatar, (seconds, stays, last)) in found {
            let last: Option<String> = stamp.query_row(params![last], |r| r.get(0))?;
            out.insert(avatar, (seconds.round() as i64, stays.len() as i64, last.unwrap_or_default()));
        }
        Ok(out)
    }

    /// How many separate visits I shared with each person, from the game log.
    pub fn together_counts(&self) -> AppResult<Vec<(String, i64)>> {
        let mut stmt = self.db.prepare(
            "SELECT user_id, COUNT(DISTINCT visit_id) FROM sightings WHERE user_id != '' GROUP BY user_id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// When this PC last saw each person do anything: a feed entry, or being in an instance
    /// with me. Nothing is asked of VRChat.
    pub fn last_seen(&self) -> AppResult<Vec<(String, String)>> {
        let mut stmt = self.db.prepare(
            "SELECT user_id, MAX(at) FROM (
                SELECT user_id, at FROM feed WHERE kind IN ('online', 'offline', 'location', 'status', 'statusText', 'avatar')
                UNION ALL
                SELECT user_id, COALESCE(left_at, joined_at) AS at FROM sightings WHERE user_id != ''
             ) GROUP BY user_id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Every tag I put on something, as (item, tag) pairs. Nothing leaves this PC.
    pub fn tags(&self) -> AppResult<Vec<(String, String)>> {
        let mut stmt = self.db.prepare("SELECT item_id, tag FROM tags ORDER BY item_id, tag COLLATE NOCASE")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Replaces the tags on one item with `tags`.
    pub fn set_tags(&self, item_id: &str, tags: &[String]) -> AppResult<()> {
        let tx = self.db.unchecked_transaction()?;
        tx.execute("DELETE FROM tags WHERE item_id = ?1", params![item_id])?;
        for tag in tags {
            tx.execute("INSERT OR IGNORE INTO tags (item_id, tag) VALUES (?1, ?2)", params![item_id, tag])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// My own stars for a world or an avatar, 1 to 5. Nothing leaves this PC.
    /// Keeps which of my friends `user_id` is friends with, replacing what was known before.
    /// `hidden` notes that this person does not let others see it.
    pub fn set_mutuals(&self, user_id: &str, others: &[String], at: &str, hidden: bool) -> AppResult<()> {
        let tx = self.db.unchecked_transaction()?;
        // Someone who hides their friends tells nothing new; what others said about them stays.
        if !hidden {
            tx.execute("DELETE FROM mutuals WHERE a = ?1 OR b = ?1", params![user_id])?;
        }
        for other in others.iter().filter(|o| o.as_str() != user_id) {
            let (a, b) = if user_id < other.as_str() { (user_id, other.as_str()) } else { (other.as_str(), user_id) };
            tx.execute("INSERT OR IGNORE INTO mutuals (a, b) VALUES (?1, ?2)", params![a, b])?;
        }
        tx.execute(
            "INSERT INTO mutuals_asked (user_id, at, hidden) VALUES (?1, ?2, ?3)
             ON CONFLICT(user_id) DO UPDATE SET at = excluded.at, hidden = excluded.hidden",
            params![user_id, at, hidden as i64],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Every pair of my friends known to be friends with each other.
    pub fn mutuals(&self) -> AppResult<Vec<(String, String)>> {
        let mut stmt = self.db.prepare("SELECT a, b FROM mutuals")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Who was asked about, when, and whether they keep it hidden.
    pub fn mutuals_asked(&self) -> AppResult<Vec<(String, String, bool)>> {
        let mut stmt = self.db.prepare("SELECT user_id, at, hidden FROM mutuals_asked")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Which of the given people I saw in the same stay at least `least` times: two ids and
    /// the number of stays, most often first. Only stays of mine count, since that is all
    /// the history knows.
    pub fn pairs(&self, ids: &[String], least: i64, limit: u32) -> AppResult<Vec<(String, String, i64)>> {
        let ids: Vec<&String> = ids.iter().filter(|id| id.starts_with("usr_") && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')).take(200).collect();
        if ids.len() < 2 {
            return Ok(Vec::new());
        }
        // The ids were checked above to be plain, so they can stand in the question itself.
        let list = ids.iter().map(|id| format!("'{id}'")).collect::<Vec<_>>().join(",");
        let mut stmt = self.db.prepare(&format!(
            "SELECT a.user_id, b.user_id, COUNT(DISTINCT a.visit_id) AS times
             FROM sightings a JOIN sightings b ON b.visit_id = a.visit_id AND b.user_id > a.user_id
             WHERE a.user_id IN ({list}) AND b.user_id IN ({list})
             GROUP BY a.user_id, b.user_id HAVING times >= ?1
             ORDER BY times DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![least, limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Notes how my own worlds stand on `day` ("2026-11-25"): visits and favourites so far, and
    /// the most people seen in them that day. A second note on the same day replaces the first.
    pub fn record_world_stats(&self, day: &str, worlds: &[(String, i64, i64, i64)]) -> AppResult<()> {
        let tx = self.db.unchecked_transaction()?;
        for (id, visits, favorites, occupants) in worlds {
            tx.execute(
                "INSERT INTO world_stats (world_id, day, visits, favorites, occupants) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(world_id, day) DO UPDATE SET visits = excluded.visits, favorites = excluded.favorites,
                     occupants = MAX(world_stats.occupants, excluded.occupants)",
                params![id, day, visits, favorites, occupants],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// What was noted about my worlds from `since` on, oldest day first:
    /// world, day, visits, favourites, most people that day.
    pub fn world_stats(&self, since: &str) -> AppResult<Vec<(String, String, i64, i64, i64)>> {
        let mut stmt = self.db.prepare("SELECT world_id, day, visits, favorites, occupants FROM world_stats WHERE day >= ?1 ORDER BY day, world_id")?;
        let rows = stmt.query_map(params![since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// People and worlds in my own history whose name has `query` in it, the ones seen
    /// latest first. A person counts with every name they were seen under. Capitals and
    /// small letters are the same, in every alphabet.
    pub fn log_find(&self, query: &str, my_id: &str) -> AppResult<(Vec<FoundPerson>, Vec<FoundWorld>)> {
        let q = query.trim().to_lowercase();
        if q.chars().count() < 2 {
            return Ok((Vec::new(), Vec::new()));
        }
        let at = now();
        // The names are compared here and not by the database, which only folds a to z.
        let names = |sql: &str| -> AppResult<Vec<String>> {
            let mut stmt = self.db.prepare(sql)?;
            let all = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
            Ok(all.into_iter().filter(|n| n.to_lowercase().contains(&q)).take(400).collect())
        };

        // Who these names belong to: an id where the log gave one, the bare name otherwise.
        let mut keys: Vec<(String, String, i64)> = Vec::new();
        for name in names("SELECT DISTINCT display_name FROM sightings")? {
            let mut stmt = self.db.prepare(
                "SELECT user_id, MAX(id) FROM sightings WHERE display_name = ?1 AND (user_id = '' OR user_id != ?2) GROUP BY user_id",
            )?;
            let rows = stmt.query_map(params![name, my_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
            for (user_id, newest) in rows {
                match keys.iter_mut().find(|(id, n, _)| if user_id.is_empty() { id.is_empty() && *n == name } else { *id == user_id }) {
                    Some(known) => {
                        if newest > known.2 {
                            *known = (user_id, name.clone(), newest);
                        }
                    }
                    None => keys.push((user_id, name.clone(), newest)),
                }
            }
        }
        keys.sort_by(|a, b| b.2.cmp(&a.2));
        keys.truncate(15);

        let mut people = Vec::new();
        for (user_id, matched, _) in keys {
            // Two questions with the same answer shape: by id where there is one, so the index on it is used.
            let whose = if user_id.is_empty() { "s.user_id = '' AND s.display_name = ?1" } else { "s.user_id = ?1" };
            let key = if user_id.is_empty() { &matched } else { &user_id };
            let found = self.db.query_row(
                &format!(
                    "SELECT COUNT(DISTINCT s.visit_id), MIN(s.joined_at) FROM sightings s WHERE {whose}"
                ),
                params![key],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?)),
            )?;
            let last = self.db.query_row(
                &format!(
                    "SELECT s.display_name, COALESCE(s.left_at, v.left_at, ?2), v.world_id, v.world_name, v.location
                     FROM sightings s JOIN visits v ON v.id = s.visit_id WHERE {whose} ORDER BY s.id DESC LIMIT 1"
                ),
                params![key, at],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?)),
            )?;
            people.push(FoundPerson {
                seconds: self.spans(whose, key, &at)?.iter().map(|(_, _, secs)| secs).sum(),
                user_id,
                name: last.0,
                matched,
                times: found.0,
                first: found.1.unwrap_or_default(),
                last: last.1,
                last_world_id: last.2,
                last_world: last.3,
                last_location: last.4,
            });
        }

        let mut worlds: Vec<FoundWorld> = Vec::new();
        let mut newest_of: Vec<i64> = Vec::new();
        for name in names("SELECT DISTINCT world_name FROM visits")? {
            let mut stmt = self.db.prepare(
                "SELECT v.world_id, COUNT(*),
                        COALESCE(SUM(MAX(0, (julianday(COALESCE(v.left_at, ?2)) - julianday(v.joined_at)) * 86400)), 0),
                        MAX(v.id)
                 FROM visits v WHERE v.world_name = ?1 GROUP BY v.world_id",
            )?;
            let rows = stmt
                .query_map(params![name, at], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, f64>(2)?, r.get::<_, i64>(3)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            for (world_id, times, seconds, newest) in rows {
                let seconds = seconds.max(0.0).round() as i64;
                // A world that was renamed is one world: its visits under both names add up.
                match worlds.iter().position(|w| !world_id.is_empty() && w.world_id == world_id) {
                    Some(i) => {
                        worlds[i].times += times;
                        worlds[i].seconds += seconds;
                        if newest > newest_of[i] {
                            newest_of[i] = newest;
                            worlds[i].name = name.clone();
                        }
                    }
                    None => {
                        worlds.push(FoundWorld { world_id, name: name.clone(), times, seconds, last: String::new(), last_location: String::new() });
                        newest_of.push(newest);
                    }
                }
            }
        }
        let mut order: Vec<usize> = (0..worlds.len()).collect();
        order.sort_by(|a, b| newest_of[*b].cmp(&newest_of[*a]));
        let mut listed = Vec::new();
        for i in order.into_iter().take(15) {
            let mut world = worlds[i].clone();
            (world.last, world.last_location) = self.db.query_row("SELECT joined_at, location FROM visits WHERE id = ?1", params![newest_of[i]], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            listed.push(world);
        }
        Ok((people, listed))
    }

    /// When one person was in my instance, stay by stay: (visit, start, seconds). Rows of the
    /// same person that overlap, as a doubled log line makes them, count once.
    fn spans(&self, whose: &str, key: &str, at: &str) -> AppResult<Vec<(i64, String, i64)>> {
        let mut stmt = self.db.prepare(&format!(
            "SELECT s.visit_id, s.joined_at, julianday(s.joined_at), julianday(COALESCE(s.left_at, v.left_at, ?2))
             FROM sightings s JOIN visits v ON v.id = s.visit_id WHERE {whose} ORDER BY s.visit_id, s.joined_at"
        ))?;
        let rows = stmt
            .query_map(params![key, at], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<f64>>(2)?, r.get::<_, Option<f64>>(3)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut out: Vec<(i64, String, i64)> = Vec::new();
        let mut reach = (0i64, f64::MIN);
        for (visit, start_text, start, end) in rows {
            let (Some(start), Some(end)) = (start, end) else { continue };
            if visit != reach.0 {
                reach = (visit, f64::MIN);
            }
            // Only the part after what was already counted in this stay is new.
            let from = start.max(reach.1);
            let secs = ((end - from) * 86400.0).max(0.0).round() as i64;
            reach.1 = reach.1.max(end);
            out.push((visit, start_text, secs));
        }
        Ok(out)
    }

    /// Every stay of mine in which one person was seen, newest first. A person without an
    /// id (an old log line) is found by name. `people` counts the others there, without
    /// that person and without me.
    pub fn stays_with(&self, user_id: &str, name: &str, my_id: &str, limit: u32) -> AppResult<Vec<LogStay>> {
        let at = now();
        let whose = if user_id.is_empty() { "s.user_id = '' AND s.display_name = ?1" } else { "s.user_id = ?1" };
        let key = if user_id.is_empty() { name } else { user_id };
        let mut seconds: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
        for (visit, _, secs) in self.spans(whose, key, &at)? {
            *seconds.entry(visit).or_default() += secs;
        }
        let mut stmt = self.db.prepare(&format!(
            "SELECT v.id, v.world_id, v.world_name, v.location, MIN(s.joined_at), MAX(COALESCE(s.left_at, v.left_at, ?2)), 0.0,
                    (SELECT COUNT(DISTINCT CASE WHEN o.user_id != '' THEN o.user_id ELSE o.display_name END) FROM sightings o
                      WHERE o.visit_id = v.id AND o.user_id != ?3 AND NOT ((?4 != '' AND o.user_id = ?4) OR (?4 = '' AND o.user_id = '' AND o.display_name = ?1)))
             FROM sightings s JOIN visits v ON v.id = s.visit_id
             WHERE {whose} GROUP BY v.id ORDER BY v.id DESC LIMIT ?5"
        ))?;
        let rows = stmt.query_map(params![key, at, my_id, user_id, limit.min(500)], stay_row)?;
        let mut stays = rows.collect::<Result<Vec<_>, _>>()?;
        for stay in &mut stays {
            stay.seconds = seconds.get(&stay.visit_id).copied().unwrap_or(0);
        }
        Ok(stays)
    }

    /// Every stay of mine in one world, newest first. `people` counts the others seen there, without me.
    pub fn stays_in(&self, world_id: &str, name: &str, my_id: &str, limit: u32) -> AppResult<Vec<LogStay>> {
        let which = if world_id.is_empty() { "v.world_id = '' AND v.world_name = ?1" } else { "v.world_id = ?1" };
        let mut stmt = self.db.prepare(&format!(
            "SELECT v.id, v.world_id, v.world_name, v.location, v.joined_at, COALESCE(v.left_at, ?2),
                    MAX(0, (julianday(COALESCE(v.left_at, ?2)) - julianday(v.joined_at)) * 86400),
                    (SELECT COUNT(DISTINCT CASE WHEN o.user_id != '' THEN o.user_id ELSE o.display_name END) FROM sightings o
                      WHERE o.visit_id = v.id AND o.user_id != ?3)
             FROM visits v WHERE {which} ORDER BY v.id DESC LIMIT ?4"
        ))?;
        let rows = stmt.query_map(params![if world_id.is_empty() { name } else { world_id }, now(), my_id, limit.min(500)], stay_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Where and when I meet one person, and where they go by themselves as far as the
    /// friend activity on this PC shows. Nothing here asks VRChat.
    pub fn habits(&self, user_id: &str, my_id: &str, since: &str) -> AppResult<Habits> {
        let at = now();
        let spans = self.spans("s.user_id = ?1", user_id, &at)?;
        let mut per_visit: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
        for (visit, _, secs) in &spans {
            *per_visit.entry(*visit).or_default() += secs;
        }
        // Each stay's world, newest stay first, so a renamed world carries its newest name.
        let mut stmt = self.db.prepare(
            "SELECT DISTINCT v.id, v.world_id, v.world_name FROM sightings s JOIN visits v ON v.id = s.visit_id WHERE s.user_id = ?1 ORDER BY v.id DESC",
        )?;
        let visited = stmt.query_map(params![user_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?.collect::<Result<Vec<_>, _>>()?;
        let mut worlds: Vec<HabitWorld> = Vec::new();
        for (visit, world_id, name) in visited {
            let secs = per_visit.get(&visit).copied().unwrap_or(0);
            match worlds.iter_mut().find(|w| if world_id.is_empty() { w.world_id.is_empty() && w.name == name } else { w.world_id == world_id }) {
                Some(w) => {
                    w.times += 1;
                    w.seconds += secs;
                }
                None => worlds.push(HabitWorld { world_id, name, times: 1, seconds: secs }),
            }
        }
        worlds.sort_by(|a, b| b.times.cmp(&a.times).then(b.seconds.cmp(&a.seconds)));
        worlds.truncate(6);

        // Others who were in the instance at the same time as this person, in at least two stays.
        let mut stmt = self.db.prepare(
            "SELECT o.user_id, o.display_name, MAX(o.id), COUNT(DISTINCT o.visit_id) AS times
             FROM sightings s JOIN sightings o ON o.visit_id = s.visit_id
             WHERE s.user_id = ?1 AND o.user_id != '' AND o.user_id != ?1 AND o.user_id != ?2
               AND o.joined_at < COALESCE(s.left_at, '9999') AND COALESCE(o.left_at, '9999') > s.joined_at
             GROUP BY o.user_id HAVING times >= 2
             ORDER BY times DESC, 3 DESC LIMIT 8",
        )?;
        let company = stmt
            .query_map(params![user_id, my_id], |r| Ok(HabitPerson { user_id: r.get(0)?, name: r.get(1)?, times: r.get(3)? }))?
            .collect::<Result<Vec<_>, _>>()?;

        let together: Vec<(String, i64)> = spans.into_iter().rev().filter(|(_, _, secs)| *secs > 0).take(1500).map(|(_, start, secs)| (start, secs)).collect();

        let mut stmt = self.db.prepare(
            "SELECT f.wid, COALESCE(w.name, ''), f.n FROM (
                 SELECT substr(location, 1, instr(location || ':', ':') - 1) AS wid, COUNT(*) AS n
                 FROM feed WHERE user_id = ?1 AND kind = 'location' AND location LIKE 'wrld\\_%' ESCAPE '\\' AND at >= ?2
                 GROUP BY wid ORDER BY n DESC LIMIT 6
             ) f LEFT JOIN worlds w ON w.world_id = f.wid ORDER BY f.n DESC",
        )?;
        let own_worlds = stmt
            .query_map(params![user_id, since], |r| Ok(HabitWorld { world_id: r.get(0)?, name: r.get(1)?, times: r.get(2)?, seconds: 0 }))?
            .collect::<Result<Vec<_>, _>>()?;

        let mut stmt = self.db.prepare("SELECT at FROM feed WHERE user_id = ?1 AND kind IN ('online', 'location') AND at >= ?2 ORDER BY id DESC LIMIT 1500")?;
        let seen_at = stmt.query_map(params![user_id, since], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        Ok(Habits { worlds, company, together, own_worlds, seen_at })
    }

    pub fn ratings(&self) -> AppResult<Vec<(String, i64)>> {
        let mut stmt = self.db.prepare("SELECT item_id, stars FROM ratings")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Sets my stars for something; 0 takes the rating away.
    pub fn set_rating(&self, item_id: &str, stars: i64, at: &str) -> AppResult<()> {
        if stars <= 0 {
            self.db.execute("DELETE FROM ratings WHERE item_id = ?1", params![item_id])?;
        } else {
            self.db.execute(
                "INSERT INTO ratings (item_id, stars, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(item_id) DO UPDATE SET stars = excluded.stars, updated_at = excluded.updated_at",
                params![item_id, stars.min(5), at],
            )?;
        }
        Ok(())
    }

    /// Every world I visited, with how often and when last, from the game log's history.
    pub fn world_visits(&self) -> AppResult<Vec<(String, i64, String)>> {
        let mut stmt = self.db.prepare(
            "SELECT world_id, COUNT(*), MAX(joined_at) FROM visits WHERE world_id != '' GROUP BY world_id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Names this PC saw someone use before: name changes in the feed, and names the game
    /// log recorded for the same account. Newest first, each with when it was last seen.
    pub fn past_names(&self, user_id: &str) -> AppResult<Vec<(String, String)>> {
        let mut stmt = self.db.prepare(
            "SELECT name, MAX(at) FROM (
                SELECT previous AS name, at FROM feed WHERE user_id = ?1 AND kind = 'name' AND previous != ''
                UNION ALL
                SELECT display_name AS name, COALESCE(left_at, joined_at) AS at FROM sightings
                WHERE user_id = ?1 AND display_name != ''
             ) GROUP BY name ORDER BY MAX(at) DESC LIMIT 30",
        )?;
        let rows = stmt.query_map(params![user_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Remembers the avatars on my account right now: (id, name, picture) each.
    pub fn record_own_avatars(&self, avatars: &[(String, String, String)]) -> AppResult<()> {
        let at = now();
        let tx = self.db.unchecked_transaction()?;
        for (id, name, image) in avatars {
            tx.execute(
                "INSERT INTO own_avatars (avatar_id, name, image, first_seen, last_seen) VALUES (?1, ?2, ?3, ?4, ?4)
                 ON CONFLICT(avatar_id) DO UPDATE SET name = excluded.name, image = excluded.image, last_seen = excluded.last_seen",
                params![id, name, image, at],
            )?;
        }
        tx.execute(
            "INSERT INTO settings (key, value) VALUES ('own_avatars_at', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![at],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Avatars of mine that were not in the newest list from VRChat.
    pub fn lost_avatars(&self) -> AppResult<Vec<LostAvatar>> {
        let Some(latest) = self.setting("own_avatars_at")? else { return Ok(Vec::new()) };
        let mut stmt = self.db.prepare(
            "SELECT avatar_id, name, image, last_seen FROM own_avatars WHERE last_seen < ?1 ORDER BY last_seen DESC",
        )?;
        let rows = stmt.query_map(params![latest], |r| {
            Ok(LostAvatar { avatar_id: r.get(0)?, name: r.get(1)?, image: r.get(2)?, last_seen: r.get(3)? })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn local_favorites(&self, kind: &str) -> AppResult<Vec<LocalFavorite>> {
        let mut stmt = self.db.prepare(
            "SELECT kind, item_id, name, author_name, image, added_at FROM local_favorites
             WHERE kind = ?1 ORDER BY added_at DESC",
        )?;
        let rows = stmt.query_map(params![kind], |r| {
            Ok(LocalFavorite {
                kind: r.get(0)?,
                item_id: r.get(1)?,
                name: r.get(2)?,
                author_name: r.get(3)?,
                image: r.get(4)?,
                added_at: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn add_local_favorite(&self, f: &LocalFavorite) -> AppResult<()> {
        self.db.execute(
            "INSERT INTO local_favorites (kind, item_id, name, author_name, image, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(kind, item_id) DO UPDATE SET name = excluded.name, author_name = excluded.author_name,
             image = excluded.image",
            params![f.kind, f.item_id, f.name, f.author_name, f.image, now()],
        )?;
        Ok(())
    }

    pub fn remove_local_favorite(&self, kind: &str, item_id: &str) -> AppResult<()> {
        self.db.execute(
            "DELETE FROM local_favorites WHERE kind = ?1 AND item_id = ?2",
            params![kind, item_id],
        )?;
        Ok(())
    }

    pub fn save_world(&self, w: &World) -> AppResult<()> {
        self.db.execute(
            "INSERT INTO worlds (world_id, name, image, fetched_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(world_id) DO UPDATE SET name = excluded.name, image = excluded.image,
             fetched_at = excluded.fetched_at",
            params![w.id, w.name, w.image, now()],
        )?;
        Ok(())
    }

    pub fn world(&self, world_id: &str) -> AppResult<Option<World>> {
        Ok(self
            .db
            .query_row(
                "SELECT world_id, name, image FROM worlds WHERE world_id = ?1",
                params![world_id],
                world_row,
            )
            .optional()?)
    }

    /// Writes a complete, consistent copy of the history to a new file.
    pub fn copy_to(&self, target: &Path) -> AppResult<()> {
        self.db
            .execute("VACUUM INTO ?1", params![target.to_string_lossy()])?;
        Ok(())
    }

    /// Remembers what the user did with a notification.
    pub fn log_handled(
        &self,
        notice: &crate::vrchat::notices::Notice,
        action: &str,
        reply_text: &str,
    ) -> AppResult<()> {
        let json = serde_json::to_string(notice).unwrap_or_default();
        self.db.execute(
            "INSERT INTO handled_notices (at, action, reply_text, notice) VALUES (?1, ?2, ?3, ?4)",
            params![now(), action, reply_text, json],
        )?;
        Ok(())
    }

    /// Handled notifications, newest first.
    pub fn handled(&self, limit: u32) -> AppResult<Vec<crate::vrchat::notices::Handled>> {
        let mut stmt = self.db.prepare(
            "SELECT id, at, action, reply_text, notice FROM handled_notices ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit.min(500)], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, at, action, reply_text, json) = row?;
            if let Ok(notice) = serde_json::from_str(&json) {
                out.push(crate::vrchat::notices::Handled { id, at, action, reply_text, notice });
            }
        }
        Ok(out)
    }

    pub fn setting(&self, key: &str) -> AppResult<Option<String>> {
        Ok(self
            .db
            .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> AppResult<()> {
        self.db.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Drops settings that start with `prefix` unless their key is in `keep`.
    pub fn forget_settings_with_prefix(&self, prefix: &str, keep: &[String]) -> AppResult<()> {
        let mut stmt = self
            .db
            .prepare("SELECT key FROM settings WHERE substr(key, 1, ?1) = ?2")?;
        let keys: Vec<String> = stmt
            .query_map(params![prefix.len() as i64, prefix], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        for key in keys.iter().filter(|k| !keep.contains(k)) {
            self.db
                .execute("DELETE FROM settings WHERE key = ?1", params![key])?;
        }
        Ok(())
    }

    /// Records a batch of game-log events and the file position they came from,
    /// all or nothing. Returns true when any event changed the history.
    pub fn record_log(
        &self,
        events: &[(String, crate::gamelog::LogEvent)],
        fresh_file: bool,
        position_key: &str,
        position: u64,
    ) -> AppResult<bool> {
        use crate::gamelog::LogEvent;
        let tx = self.db.unchecked_transaction()?;
        let mut changed = false;
        if fresh_file {
            // A new log file means the game was started again. A stay that this ends counts
            // as a change, so the window stops showing the instance of the last session.
            changed = tx.query_row("SELECT EXISTS(SELECT 1 FROM visits WHERE left_at IS NULL)", [], |r| r.get::<_, bool>(0))?;
            close_visits(&tx, None)?;
        }
        let mut room_name = String::new();
        // The line naming who put a video on comes just before VRChat's own video line, and a read
        // can end between the two, so it is kept with the reading position until its video comes.
        let mut pending_by: Option<(String, String)> = if fresh_file {
            None
        } else {
            tx.query_row("SELECT value FROM settings WHERE key = ?1", params![ASKED_BY], |r| r.get::<_, String>(0))
                .optional()?
                .and_then(|v| v.split_once('\u{1f}').map(|(u, n)| (u.to_string(), n.to_string())))
        };
        for (at, event) in events {
            match event {
                LogEvent::VideoBy { url, name } => pending_by = Some((url.clone(), name.clone())),
                LogEvent::Video { url } => {
                    let by = asked_by(&mut pending_by, url);
                    let stay: Option<i64> = tx
                        .query_row("SELECT id FROM visits WHERE left_at IS NULL ORDER BY id DESC LIMIT 1", [], |r| r.get(0))
                        .optional()?;
                    let done = tx.execute(INSERT_VIDEO, params![stay, at, url, crate::gamelog::youtube_id(url).unwrap_or_default(), by])?;
                    changed |= done > 0;
                }
                LogEvent::Room(name) => room_name = name.clone(),
                LogEvent::Joining(location) => {
                    pending_by = None;
                    close_visits(&tx, Some(at))?;
                    let world_id = parse_location(location).world_id.unwrap_or_default();
                    tx.execute(
                        "INSERT INTO visits (location, world_id, world_name, joined_at) VALUES (?1, ?2, ?3, ?4)",
                        params![location, world_id, room_name, at],
                    )?;
                    room_name.clear();
                    changed = true;
                }
                LogEvent::LeftRoom | LogEvent::Quit => {
                    pending_by = None;
                    close_visits(&tx, Some(at))?;
                    changed = true;
                }
                LogEvent::PlayerJoined { name, id } => {
                    let done = tx.execute(
                        "INSERT INTO sightings (visit_id, user_id, display_name, joined_at)
                         SELECT v.id, ?1, ?2, ?3 FROM (SELECT id FROM visits WHERE left_at IS NULL ORDER BY id DESC LIMIT 1) v
                         WHERE NOT EXISTS (SELECT 1 FROM sightings s WHERE s.visit_id = v.id AND s.left_at IS NULL
                                           AND ((?1 <> '' AND s.user_id = ?1) OR (?1 = '' AND s.user_id = '' AND s.display_name = ?2)))",
                        params![id, name, at],
                    )?;
                    changed |= done > 0;
                }
                LogEvent::PlayerLeft { name, id } => {
                    // Never before they arrived, the same as a stay (see close_visits).
                    let done = tx.execute(
                        "UPDATE sightings SET left_at = CASE WHEN julianday(?1) < julianday(joined_at) THEN joined_at ELSE ?1 END
                         WHERE left_at IS NULL
                         AND visit_id = (SELECT id FROM visits WHERE left_at IS NULL ORDER BY id DESC LIMIT 1)
                         AND ((?2 <> '' AND user_id = ?2) OR (?2 = '' AND display_name = ?3))",
                        params![at, id, name],
                    )?;
                    changed |= done > 0;
                }
            }
        }
        match &pending_by {
            Some((url, name)) => tx.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![ASKED_BY, format!("{url}\u{1f}{name}")],
            )?,
            None => tx.execute("DELETE FROM settings WHERE key = ?1", params![ASKED_BY])?,
        };
        tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![position_key, position.to_string()],
        )?;
        tx.commit()?;
        Ok(changed)
    }

    /// The instance the game is in right now, if any.
    /// Ends the stay that is still open, for when the game is gone without saying so.
    pub fn close_open_visits(&self, at: &str) -> AppResult<()> {
        close_visits(&self.db, Some(at))?;
        Ok(())
    }

    pub fn open_visit(&self) -> AppResult<Option<Visit>> {
        let sql = format!(
            "SELECT {VISIT_COLUMNS} FROM visits v WHERE v.left_at IS NULL ORDER BY v.id DESC LIMIT 1"
        );
        Ok(self.db.query_row(&sql, [], visit_row).optional()?)
    }

    /// Newest first. `before` pages back through older visits.
    pub fn visits(&self, limit: u32, before: Option<i64>) -> AppResult<Vec<Visit>> {
        let sql = format!(
            "SELECT {VISIT_COLUMNS} FROM visits v WHERE v.id < ?1 ORDER BY v.id DESC LIMIT ?2"
        );
        let mut stmt = self.db.prepare(&sql)?;
        let rows = stmt.query_map(params![before.unwrap_or(i64::MAX), limit.min(500)], visit_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The stay I was in at a moment (UTC, RFC 3339 to the second), if the history has one.
    pub fn visit_at(&self, at: &str) -> AppResult<Option<Visit>> {
        let sql = format!(
            "SELECT {VISIT_COLUMNS} FROM visits v
             WHERE v.joined_at <= ?1 ORDER BY v.joined_at DESC, v.id DESC LIMIT 1"
        );
        let mut stmt = self.db.prepare(&sql)?;
        let mut rows = stmt.query_map(params![at], visit_row)?;
        // The stay that began last before that moment. A stay that was never closed (the game
        // crashed) only counts when nothing newer began before the moment, which this gives.
        let latest: Option<Visit> = rows.next().transpose()?;
        Ok(latest.filter(|v| v.left_at.as_deref().is_none_or(|left| left >= at)))
    }

    /// Who was in a stay with me at a moment: already arrived, not yet gone.
    pub fn people_at(&self, visit_id: i64, at: &str) -> AppResult<Vec<(String, String)>> {
        let mut stmt = self.db.prepare(
            "SELECT user_id, display_name FROM sightings
             WHERE visit_id = ?1 AND joined_at <= ?2 AND (left_at IS NULL OR left_at >= ?2)
             GROUP BY CASE WHEN user_id = '' THEN display_name ELSE user_id END
             ORDER BY display_name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map(params![visit_id, at], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn visit_people(&self, visit_id: i64) -> AppResult<Vec<Sighting>> {
        let mut stmt = self.db.prepare(
            "SELECT user_id, display_name, joined_at, left_at FROM sightings
             WHERE visit_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![visit_id], |r| {
            Ok(Sighting {
                user_id: r.get(0)?,
                display_name: r.get(1)?,
                joined_at: r.get(2)?,
                left_at: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Videos from the part of the game log that was read before videos were kept. Each one
    /// goes with the stay that was open at its time. Returns true when any was new.
    pub fn record_old_videos(&self, events: &[(String, crate::gamelog::LogEvent)]) -> AppResult<bool> {
        use crate::gamelog::LogEvent;
        let tx = self.db.unchecked_transaction()?;
        let mut changed = false;
        let mut pending_by: Option<(String, String)> = None;
        for (at, event) in events {
            match event {
                LogEvent::VideoBy { url, name } => pending_by = Some((url.clone(), name.clone())),
                LogEvent::Video { url } => {
                    let by = asked_by(&mut pending_by, url);
                    let stay = self.visit_at(at)?.map(|v| v.id);
                    let done = tx.execute(INSERT_VIDEO, params![stay, at, url, crate::gamelog::youtube_id(url).unwrap_or_default(), by])?;
                    changed |= done > 0;
                }
                _ => {}
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// The videos played around me, newest first.
    pub fn videos(&self, limit: u32) -> AppResult<Vec<VideoPlay>> {
        let sql = format!("{VIDEO_SELECT} ORDER BY p.at DESC, p.id DESC LIMIT ?1");
        let mut stmt = self.db.prepare(&sql)?;
        let rows = stmt.query_map(params![limit.clamp(1, 2000)], video_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// One page of the videos played around me, newest first, with how many match in all.
    /// `query` looks in the link, who put it on, the world and the YouTube title and channel;
    /// `skip_arrival` leaves out videos already on as I arrived; `before` is the time and id
    /// of the last video of the page before.
    pub fn videos_page(&self, query: &str, skip_arrival: bool, before: Option<(String, i64)>, limit: u32) -> AppResult<(Vec<VideoPlay>, i64)> {
        let pattern = if query.trim().is_empty() { String::new() } else { like_pattern(&query.trim().to_lowercase()) };
        let filter = "(?1 = '' OR fold(p.url) LIKE ?1 ESCAPE '\\' OR fold(p.requested_by) LIKE ?1 ESCAPE '\\' OR fold(t.title) LIKE ?1 ESCAPE '\\'
                       OR fold(t.channel) LIKE ?1 ESCAPE '\\' OR fold(COALESCE(NULLIF(v.world_name, ''), w.name, '')) LIKE ?1 ESCAPE '\\')
            AND NOT (?2 = 1 AND p.requested_by = '' AND v.joined_at IS NOT NULL AND (julianday(p.at) - julianday(v.joined_at)) * 86400 <= 30)";
        let total: i64 = self.db.query_row(
            &format!(
                "SELECT COUNT(*) FROM videos p LEFT JOIN visits v ON v.id = p.visit_id LEFT JOIN worlds w ON w.world_id = v.world_id
                 LEFT JOIN video_titles t ON t.video_id = p.youtube_id AND p.youtube_id <> '' WHERE {filter}"
            ),
            params![pattern, skip_arrival],
            |r| r.get(0),
        )?;
        let (at, id) = before.unwrap_or_else(|| (String::new(), i64::MAX));
        let sql = format!(
            "{VIDEO_SELECT} WHERE {filter} AND (?3 = '' OR p.at < ?3 OR (p.at = ?3 AND p.id < ?4))
             ORDER BY p.at DESC, p.id DESC LIMIT ?5"
        );
        let mut stmt = self.db.prepare(&sql)?;
        let rows = stmt.query_map(params![pattern, skip_arrival, at, id, limit.clamp(1, 200)], video_row)?;
        Ok((rows.collect::<Result<Vec<_>, _>>()?, total))
    }

    /// The videos played during one stay, in the order they played.
    pub fn visit_videos(&self, visit_id: i64) -> AppResult<Vec<VideoPlay>> {
        let sql = format!("{VIDEO_SELECT} WHERE p.visit_id = ?1 ORDER BY p.at, p.id");
        let mut stmt = self.db.prepare(&sql)?;
        let rows = stmt.query_map(params![visit_id], video_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The titles already known for these YouTube ids. An id YouTube was never asked about is left
    /// out, and so is one YouTube gave no title for over a week ago, so it is asked again.
    pub fn video_titles(&self, ids: &[String]) -> AppResult<Vec<VideoTitle>> {
        let week_ago = (chrono::Utc::now() - chrono::Duration::days(7)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let mut stmt = self.db.prepare(
            "SELECT video_id, title, channel FROM video_titles
             WHERE video_id = ?1 AND NOT (title = '' AND channel = '' AND fetched_at < ?2)",
        )?;
        let mut out = Vec::new();
        for id in ids {
            if let Some(t) = stmt
                .query_row(params![id, week_ago], |r| Ok(VideoTitle { id: r.get(0)?, title: r.get(1)?, channel: r.get(2)? }))
                .optional()?
            {
                out.push(t);
            }
        }
        Ok(out)
    }

    pub fn save_video_title(&self, id: &str, title: &str, channel: &str) -> AppResult<()> {
        self.db.execute(
            "INSERT INTO video_titles (video_id, title, channel, fetched_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(video_id) DO UPDATE SET title = excluded.title, channel = excluded.channel, fetched_at = excluded.fetched_at",
            params![id, title, channel, now()],
        )?;
        Ok(())
    }

    /// Runs a query and hands each row over as text, one row at a time, for an export.
    /// Returns how many rows there were.
    pub fn each_row(&self, sql: &str, mut each: impl FnMut(&[Option<String>]) -> AppResult<()>) -> AppResult<u64> {
        use rusqlite::types::ValueRef;
        let mut stmt = self.db.prepare(sql)?;
        let n = stmt.column_count();
        let mut rows = stmt.query([])?;
        let mut count = 0u64;
        let mut cells: Vec<Option<String>> = Vec::with_capacity(n);
        while let Some(row) = rows.next()? {
            cells.clear();
            for i in 0..n {
                cells.push(match row.get_ref(i)? {
                    ValueRef::Null => None,
                    ValueRef::Integer(v) => Some(v.to_string()),
                    ValueRef::Real(v) => Some(v.to_string()),
                    ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
                    ValueRef::Blob(_) => Some(String::new()),
                });
            }
            each(&cells)?;
            count += 1;
        }
        Ok(count)
    }

    pub fn worlds(&self) -> AppResult<Vec<World>> {
        let mut stmt = self.db.prepare("SELECT world_id, name, image FROM worlds")?;
        let rows = stmt.query_map([], world_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

/// A VRCX import writes in pieces, each its own transaction, so the app's own writes wait for
/// one piece at most, not for the whole import. A piece takes rows in steps of this many until
/// it has held the write lock for `PIECE_TIME`, then lets go for `PIECE_PAUSE`.
const IMPORT_ROWS: i64 = 1000;
const PIECE_TIME: std::time::Duration = std::time::Duration::from_millis(200);
const PIECE_PAUSE: std::time::Duration = std::time::Duration::from_millis(120);

/// Settings rows that say an import added feed entries or stays and the list may still have
/// to be numbered again in order of time. Every piece that adds rows sets its row in the same
/// write, and the renumber removes it in its own write. When a run stops in between, because
/// the app was closed, it crashed or the file stayed busy, the next import or the next time
/// the history is opened finds the row and finishes the job.
const RENUMBER_FEED: &str = "vrcx_import_renumber_feed";
const RENUMBER_VISITS: &str = "vrcx_import_renumber_visits";

#[cfg(test)]
thread_local! {
    /// A test sets this to make the next import stop after its pieces are written and before
    /// the renumber, the way a run stops when the app is closed at that moment.
    static STOP_BEFORE_RENUMBER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Tells whether a list read by id, from just after `last_id` on, has an entry older than the
/// one before it.
fn out_of_order(db: &Connection, table: &str, column: &str, last_id: i64) -> AppResult<bool> {
    Ok(db.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM (SELECT julianday({column}) AS t, LAG(julianday({column})) OVER (ORDER BY id) AS p
                                          FROM {table} WHERE id > ?1) WHERE t < p)"
        ),
        params![last_id],
        |r| r.get(0),
    )?)
}

/// Numbers the whole feed again in order of time, inside the caller's write, and removes the
/// mark that said it was needed.
fn renumber_feed(db: &Connection) -> AppResult<()> {
    // The sorted copy is a temporary table, so the list is not written to the file twice.
    // The feed's own indexes are dropped while it is filled again and then built anew from the
    // schema: building one in a single pass is much quicker than keeping it up to date row by
    // row, and this write is the longest the app's own writes wait for.
    let indexes: Vec<String> = db
        .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'feed' AND sql IS NOT NULL")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for index in &indexes {
        db.execute_batch(&format!("DROP INDEX \"{}\";", index.replace('"', "\"\"")))?;
    }
    db.execute_batch(
        "DROP TABLE IF EXISTS temp.feed_sorted;
         CREATE TEMP TABLE feed_sorted AS SELECT at, user_id, display_name, kind, detail, previous, location FROM feed ORDER BY at, id;
         DELETE FROM feed;
         DELETE FROM sqlite_sequence WHERE name = 'feed';
         INSERT INTO feed (at, user_id, display_name, kind, detail, previous, location)
             SELECT at, user_id, display_name, kind, detail, previous, location FROM temp.feed_sorted ORDER BY rowid;
         DROP TABLE temp.feed_sorted;",
    )?;
    db.execute_batch(SCHEMA)?;
    db.execute("DELETE FROM settings WHERE key = ?1", params![RENUMBER_FEED])?;
    Ok(())
}

/// Numbers all stays again in order of time, inside the caller's write, so the people seen and
/// the videos played in each keep pointing at their stay, and removes the mark that said it
/// was needed.
fn renumber_visits(db: &Connection) -> AppResult<()> {
    db.execute_batch(
        "DROP TABLE IF EXISTS visit_order;
         DROP TABLE IF EXISTS visits_sorted;
         CREATE TABLE visit_order (was INTEGER PRIMARY KEY, now INTEGER NOT NULL);
         INSERT INTO visit_order (was, now) SELECT id, ROW_NUMBER() OVER (ORDER BY joined_at, id) FROM visits;
         CREATE TABLE visits_sorted AS SELECT o.now AS id, v.location, v.world_id, v.world_name, v.joined_at, v.left_at
             FROM visits v JOIN visit_order o ON o.was = v.id;
         UPDATE sightings SET visit_id = (SELECT now FROM visit_order WHERE was = sightings.visit_id)
             WHERE visit_id IN (SELECT was FROM visit_order);
         UPDATE videos SET visit_id = (SELECT now FROM visit_order WHERE was = videos.visit_id)
             WHERE visit_id IN (SELECT was FROM visit_order);
         DELETE FROM visits;
         DELETE FROM sqlite_sequence WHERE name = 'visits';
         INSERT INTO visits (id, location, world_id, world_name, joined_at, left_at)
             SELECT id, location, world_id, world_name, joined_at, left_at FROM visits_sorted ORDER BY id;
         DROP TABLE visits_sorted;
         DROP TABLE visit_order;",
    )?;
    db.execute("DELETE FROM settings WHERE key = ?1", params![RENUMBER_VISITS])?;
    Ok(())
}

/// What was brought over from another app's history, counted by kind.
#[derive(Clone, Debug, Default, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BroughtOver {
    /// Friend activity: online, offline, world changes, status, avatar.
    pub feed: usize,
    /// Friends added, removed and renamed.
    pub friend_log: usize,
    pub visits: usize,
    /// People seen in those visits.
    pub people: usize,
    pub notes: usize,
    pub favourites: usize,
    /// Avatars I wore, with the time on each.
    #[serde(default)]
    pub avatars: usize,
    /// My own tags on avatars.
    #[serde(default)]
    pub tags: usize,
}

impl Store {
    /// Finishes putting the feed and the stays back in order of time when an import stopped
    /// after writing rows but before numbering the list again. When no import left that job
    /// behind this is one quick look and writes nothing.
    fn finish_renumber(&self) -> AppResult<()> {
        use rusqlite::{Transaction, TransactionBehavior};
        let marked = |db: &Connection, key: &str| -> AppResult<bool> {
            Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM settings WHERE key = ?1)", params![key], |r| r.get(0))?)
        };
        if !marked(&self.db, RENUMBER_FEED)? && !marked(&self.db, RENUMBER_VISITS)? {
            return Ok(());
        }
        let tx = Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)?;
        type Renumber = fn(&Connection) -> AppResult<()>;
        let lists: [(&str, &str, &str, Renumber); 2] =
            [(RENUMBER_FEED, "feed", "at", renumber_feed), (RENUMBER_VISITS, "visits", "joined_at", renumber_visits)];
        for (key, table, column, renumber) in lists {
            // Looked at again inside the write, in case another connection finished it meanwhile.
            if !marked(&tx, key)? {
                continue;
            }
            // A list that is already in order only loses its mark: reading it through once is
            // much quicker than writing all of it again.
            if out_of_order(&tx, table, column, 0)? {
                renumber(&tx)?;
            } else {
                tx.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
            }
        }
        tx.commit()?;
        // A passive fold never waits for anyone, so the app's own writes are not held up.
        let _ = self.db.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()));
        Ok(())
    }

    /// Copies the history VRCX kept for this account into this one. `other` is VRCX's own
    /// database file, opened read-only and never changed. Only the tables of `my_id` are
    /// read, so another account's history in the same file stays where it is.
    ///
    /// Running it again brings nothing over twice: a feed entry is skipped when this history
    /// already has the same kind of entry for the same person within five seconds, and a
    /// visit when it already has one in the same instance within ten. Afterwards the feed
    /// and the visits are put back in order of time, because both are read newest-id first.
    ///
    /// `history_only` leaves notes and favourites alone. The run the app makes by itself uses
    /// it, so a note I cleared or a favourite I removed here does not come back from VRCX.
    ///
    /// `since` (a time as VRCX writes it, "2026-11-25T03:12:45.123Z") reads only what VRCX
    /// recorded from then on, which keeps the hourly look short however long the history is.
    ///
    /// The app runs it on a connection of its own, so it never holds up the one the rest of
    /// the app uses. It first gathers what is new while only reading the history, then writes
    /// it in short pieces, each its own transaction (see `IMPORT_ROWS`).
    pub fn import_vrcx(&self, other: &Path, my_id: &str, history_only: bool, since: Option<&str>) -> AppResult<BroughtOver> {
        use rusqlite::{Transaction, TransactionBehavior};
        use std::collections::hash_map::Entry;
        use std::collections::{BTreeSet, HashMap, HashSet};
        // Only a plain time may go into the questions below.
        let from = match since.filter(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_digit() || "-:.TZ ".contains(c))) {
            Some(t) => format!("WHERE created_at >= '{t}'"),
            None => String::new(),
        };
        let old = Connection::open_with_flags(other, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let has = |name: &str| -> bool {
            old.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1", params![name], |r| r.get::<_, i64>(0))
                .map(|n| n > 0)
                .unwrap_or(false)
        };
        // VRCX names an account's tables after its id with the punctuation taken out.
        let prefix: String = my_id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        if prefix.len() < 8 {
            return Err(AppError::CannotDo("That is not an account id.".into()));
        }
        let text = |r: &rusqlite::Row<'_>, i: usize| -> String { r.get::<_, Option<String>>(i).ok().flatten().unwrap_or_default() };
        // A time as whole seconds, worked out by SQLite in the question itself so it is the same
        // number the duplicate check always used. Empty when SQLite cannot read the time.
        let secs = |r: &rusqlite::Row<'_>, i: usize| -> Option<i64> { r.get::<_, Option<i64>>(i).ok().flatten() };
        // A run that was cut off earlier may have left a list out of order; that is put right
        // first, so this run starts from a history read newest-id first as it should be.
        self.finish_renumber()?;
        // The run reads and writes many pages of the history and of its own temporary tables;
        // a larger page cache for this connection saves reading the same pages from disk again.
        self.db.execute_batch("PRAGMA cache_size = -65536; PRAGMA temp.cache_size = -65536;")?;
        let mut done = BroughtOver::default();

        // First everything new is gathered in this connection's own temporary tables. That
        // only reads the history, as it stood when the run began, so the rest of the app keeps
        // writing to it meanwhile.
        let gather = Transaction::new_unchecked(&self.db, TransactionBehavior::Deferred)?;
        // Where both lists end before anything is added, to tell afterwards whether the new
        // entries landed after everything already here or in among it.
        let end = |sql: &str| -> AppResult<(i64, Option<f64>)> { Ok(gather.query_row(sql, [], |r| Ok((r.get(0)?, r.get(1)?)))?) };
        let (feed_last_id, feed_last_at) = end("SELECT COALESCE(MAX(id), 0), julianday(MAX(at)) FROM feed")?;
        let (visits_last_id, visits_last_at) = end("SELECT COALESCE(MAX(id), 0), MAX(julianday(joined_at)) FROM visits")?;
        gather.execute_batch(
            "DROP TABLE IF EXISTS temp.import_feed;
             DROP TABLE IF EXISTS temp.import_feed_sorted;
             DROP TABLE IF EXISTS temp.import_visits;
             DROP TABLE IF EXISTS temp.import_people;
             DROP TABLE IF EXISTS temp.import_seen;
             CREATE TEMP TABLE import_feed (at TEXT, user_id TEXT, display_name TEXT, kind TEXT, detail TEXT, previous TEXT, location TEXT, secs INTEGER);
             CREATE INDEX temp.import_feed_key ON import_feed(user_id, kind, secs);
             CREATE TEMP TABLE import_visits (location TEXT, world_id TEXT, world_name TEXT, joined_at TEXT, left_at TEXT, secs INTEGER);
             CREATE INDEX temp.import_visits_key ON import_visits(location, secs);
             CREATE TEMP TABLE import_people (visit INTEGER, user_id TEXT, display_name TEXT, joined_at TEXT, left_at TEXT);
             CREATE INDEX temp.import_people_visit ON import_people(visit);
             CREATE TEMP TABLE import_seen (location TEXT, created_at TEXT, type TEXT, display_name TEXT, user_id TEXT);",
        )?;

        // The times one person already has entries of one kind at, in whole seconds, read once
        // per person and kind through the index and kept for the run, with this run's own
        // entries added as they are taken. A line is skipped when one of these is within five
        // seconds of it. A time SQLite cannot read matches nothing, as before.
        let mut feed_times: HashMap<(String, String), BTreeSet<i64>> = HashMap::new();
        let mut add = |at: &str, at_secs: Option<i64>, user: &str, name: &str, kind: &str, detail: &str, previous: &str, location: &str| -> AppResult<bool> {
            if at.is_empty() || user.is_empty() {
                return Ok(false);
            }
            if let Some(s) = at_secs {
                let times = match feed_times.entry((user.to_string(), kind.to_string())) {
                    Entry::Occupied(e) => e.into_mut(),
                    Entry::Vacant(e) => {
                        let mut stmt = gather.prepare_cached("SELECT CAST(strftime('%s', at) AS INTEGER) FROM feed WHERE user_id = ?1 AND kind = ?2")?;
                        let known = stmt.query_map(params![user, kind], |r| r.get::<_, Option<i64>>(0))?.collect::<Result<Vec<_>, _>>()?;
                        e.insert(known.into_iter().flatten().collect())
                    }
                };
                if times.range(s - 5..=s + 5).next().is_some() {
                    return Ok(false);
                }
                times.insert(s);
            }
            gather
                .prepare_cached("INSERT INTO import_feed (at, user_id, display_name, kind, detail, previous, location, secs) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")?
                .execute(params![at, user, name, kind, detail, previous, location, at_secs])?;
            Ok(true)
        };
        const SECS: &str = "CAST(strftime('%s', created_at) AS INTEGER)";

        let table = format!("{prefix}_feed_gps");
        if has(&table) {
            let mut rows = old.prepare(&format!("SELECT created_at, user_id, display_name, location, previous_location, {SECS} FROM \"{table}\" {from} ORDER BY created_at"))?;
            let mut it = rows.query([])?;
            while let Some(r) = it.next()? {
                if add(&text(r, 0), secs(r, 5), &text(r, 1), &text(r, 2), "location", "", &text(r, 4), &text(r, 3))? {
                    done.feed += 1;
                }
            }
        }
        let table = format!("{prefix}_feed_online_offline");
        if has(&table) {
            let mut rows = old.prepare(&format!("SELECT created_at, user_id, display_name, type, location, {SECS} FROM \"{table}\" {from} ORDER BY created_at"))?;
            let mut it = rows.query([])?;
            while let Some(r) = it.next()? {
                let kind = if text(r, 3).eq_ignore_ascii_case("online") { "online" } else { "offline" };
                if add(&text(r, 0), secs(r, 5), &text(r, 1), &text(r, 2), kind, "", "", &text(r, 4))? {
                    done.feed += 1;
                }
            }
        }
        let table = format!("{prefix}_feed_status");
        if has(&table) {
            let mut rows = old.prepare(&format!(
                "SELECT created_at, user_id, display_name, status, status_description, previous_status, previous_status_description, {SECS} FROM \"{table}\" {from} ORDER BY created_at"
            ))?;
            let mut it = rows.query([])?;
            while let Some(r) = it.next()? {
                let (at, at_secs, user, name) = (text(r, 0), secs(r, 7), text(r, 1), text(r, 2));
                let (status, words, was, were) = (text(r, 3), text(r, 4), text(r, 5), text(r, 6));
                if !status.is_empty() && status != was && add(&at, at_secs, &user, &name, "status", &status, &was, "")? {
                    done.feed += 1;
                }
                if words != were && add(&at, at_secs, &user, &name, "statusText", &words, &were, "")? {
                    done.feed += 1;
                }
            }
        }
        let table = format!("{prefix}_feed_avatar");
        if has(&table) {
            let mut rows = old.prepare(&format!("SELECT created_at, user_id, display_name, {SECS} FROM \"{table}\" {from} ORDER BY created_at"))?;
            let mut it = rows.query([])?;
            while let Some(r) = it.next()? {
                if add(&text(r, 0), secs(r, 3), &text(r, 1), &text(r, 2), "avatar", "", "", "")? {
                    done.feed += 1;
                }
            }
        }
        let table = format!("{prefix}_friend_log_history");
        if has(&table) {
            let mut rows = old.prepare(&format!("SELECT created_at, type, user_id, display_name, previous_display_name, {SECS} FROM \"{table}\" {from} ORDER BY created_at"))?;
            let mut it = rows.query([])?;
            while let Some(r) = it.next()? {
                let (at, at_secs, kind, user, name, was) = (text(r, 0), secs(r, 5), text(r, 1), text(r, 2), text(r, 3), text(r, 4));
                let added = match kind.as_str() {
                    "Friend" => add(&at, at_secs, &user, &name, "friendAdded", "", "", "")?,
                    "Unfriend" => add(&at, at_secs, &user, &name, "friendRemoved", "", "", "")?,
                    "DisplayName" => add(&at, at_secs, &user, &name, "name", &name, &was, "")?,
                    _ => false,
                };
                if added {
                    done.friend_log += 1;
                }
            }
        }

        // Visits. A stay is skipped when this history, or this run, already has one in the same
        // instance within ten seconds; the times per instance are read once, like the feed's.
        let mut stay_times: HashMap<String, BTreeSet<i64>> = HashMap::new();
        let mut places: HashSet<String> = HashSet::new();
        if has("gamelog_location") {
            let mut rows = old.prepare(&format!("SELECT created_at, location, world_id, world_name, time, {SECS} FROM gamelog_location {from} ORDER BY created_at"))?;
            let mut it = rows.query([])?;
            while let Some(r) = it.next()? {
                let (joined, location, world_id, world_name) = (text(r, 0), text(r, 1), text(r, 2), text(r, 3));
                let stayed_ms: i64 = r.get::<_, Option<i64>>(4).ok().flatten().unwrap_or(0).max(0);
                if joined.is_empty() || location.is_empty() {
                    continue;
                }
                let joined_secs = secs(r, 5);
                if let Some(s) = joined_secs {
                    let times = match stay_times.entry(location.clone()) {
                        Entry::Occupied(e) => e.into_mut(),
                        Entry::Vacant(e) => {
                            let mut stmt = gather.prepare_cached("SELECT CAST(strftime('%s', joined_at) AS INTEGER) FROM visits WHERE location = ?1")?;
                            let known = stmt.query_map(params![location], |r| r.get::<_, Option<i64>>(0))?.collect::<Result<Vec<_>, _>>()?;
                            e.insert(known.into_iter().flatten().collect())
                        }
                    };
                    if times.range(s - 10..=s + 10).next().is_some() {
                        continue;
                    }
                    times.insert(s);
                }
                // VRCX keeps how long the stay lasted; a stay it never closed ends where it began.
                let left: String = gather.prepare_cached("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', ?1, ?2)")?.query_row(
                    params![joined, format!("+{:.3} seconds", stayed_ms as f64 / 1000.0)],
                    |r| r.get(0),
                )?;
                gather
                    .prepare_cached("INSERT INTO import_visits (location, world_id, world_name, joined_at, left_at, secs) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?
                    .execute(params![location, world_id, world_name, joined, left, joined_secs])?;
                places.insert(location);
                done.visits += 1;
            }
        }

        // The people the game log saw in those stays. VRCX's file need not have an index on
        // where a line was written, so asking it once per stay read the whole list each time.
        // Its lines are read once instead, those for the instances being brought over are kept
        // here with an index, and each stay then finds its own.
        if !places.is_empty() && has("gamelog_join_leave") {
            {
                let mut keep = gather.prepare("INSERT INTO import_seen (location, created_at, type, display_name, user_id) VALUES (?1, ?2, ?3, ?4, ?5)")?;
                let mut rows = old.prepare(&format!("SELECT location, created_at, type, display_name, user_id FROM gamelog_join_leave {from}"))?;
                let mut it = rows.query([])?;
                while let Some(p) = it.next()? {
                    let location = text(p, 0);
                    if places.contains(&location) {
                        keep.execute(params![location, text(p, 1), text(p, 2), text(p, 3), text(p, 4)])?;
                    }
                }
            }
            gather.execute_batch("CREATE INDEX temp.import_seen_at ON import_seen(location, created_at);")?;
            let mut stays = gather.prepare("SELECT rowid, location, joined_at, left_at FROM import_visits ORDER BY rowid")?;
            let mut seen = gather.prepare(
                "SELECT created_at, type, display_name, user_id FROM import_seen
                 WHERE location = ?1 AND created_at >= ?2 AND created_at <= ?3 ORDER BY created_at, rowid",
            )?;
            let mut put = gather.prepare("INSERT INTO import_people (visit, user_id, display_name, joined_at, left_at) VALUES (?1, ?2, ?3, ?4, ?5)")?;
            let mut it = stays.query([])?;
            while let Some(v) = it.next()? {
                let (visit, location, joined, left): (i64, String, String, String) = (v.get(0)?, v.get(1)?, v.get(2)?, v.get(3)?);
                // Each join is a new sighting; a leave closes the latest open one of that name.
                // Whoever is still there when the stay ends leaves with it.
                let mut people: Vec<(String, String, String, Option<String>)> = Vec::new();
                let mut lines = seen.query(params![location, joined, left])?;
                while let Some(p) = lines.next()? {
                    let (at, kind, name, user) = (text(p, 0), text(p, 1), text(p, 2), text(p, 3));
                    if name.is_empty() {
                        continue;
                    }
                    if kind == "OnPlayerJoined" {
                        people.push((user, name, at, None));
                    } else if kind == "OnPlayerLeft" {
                        if let Some(open) = people.iter_mut().rev().find(|p| p.1 == name && p.3.is_none()) {
                            open.3 = Some(at);
                        }
                    }
                }
                for (user, name, at, gone) in &people {
                    put.execute(params![visit, user, name, at, gone.as_deref().unwrap_or(&left)])?;
                }
                done.people += people.len();
            }
        }
        gather.commit()?;

        // Then it goes into the history in pieces, each in its own short write, so a write the
        // app makes meanwhile waits for one piece at most. If the run stops half way, what was
        // written stays, and the next run skips it as already here.
        // Between two pieces the run waits a moment. A write that found the file busy tries
        // again at least every tenth of a second, so it gets in during that pause instead of
        // waiting until the whole import is done. A history kept in memory has nobody else
        // writing to it, so it goes straight on.
        let shared = self.db.path().is_some_and(|p| !p.is_empty());
        let wrote = std::cell::Cell::new(false);
        let write = || {
            if shared && wrote.replace(true) {
                std::thread::sleep(PIECE_PAUSE);
            }
            Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)
        };
        {
            let tx = write()?;
            // Entries and stays the app wrote itself while the run was reading count as already
            // here, by the same rules.
            {
                let mut fresh = tx.prepare("SELECT user_id, kind, CAST(strftime('%s', at) AS INTEGER) FROM feed WHERE id > ?1")?;
                let mut rows = fresh.query(params![feed_last_id])?;
                while let Some(r) = rows.next()? {
                    let (user, kind, s): (String, String, Option<i64>) = (r.get(0)?, r.get(1)?, r.get(2)?);
                    let Some(s) = s else { continue };
                    let gone = tx.execute("DELETE FROM import_feed WHERE user_id = ?1 AND kind = ?2 AND secs BETWEEN ?3 - 5 AND ?3 + 5", params![user, kind, s])?;
                    if matches!(kind.as_str(), "friendAdded" | "friendRemoved" | "name") {
                        done.friend_log -= gone.min(done.friend_log);
                    } else {
                        done.feed -= gone.min(done.feed);
                    }
                }
                let mut fresh = tx.prepare("SELECT location, CAST(strftime('%s', joined_at) AS INTEGER) FROM visits WHERE id > ?1")?;
                let mut rows = fresh.query(params![visits_last_id])?;
                while let Some(r) = rows.next()? {
                    let (location, s): (String, Option<i64>) = (r.get(0)?, r.get(1)?);
                    let Some(s) = s else { continue };
                    let staged: Vec<i64> = tx
                        .prepare_cached("SELECT rowid FROM import_visits WHERE location = ?1 AND secs BETWEEN ?2 - 10 AND ?2 + 10")?
                        .query_map(params![location, s], |r| r.get(0))?
                        .collect::<Result<_, _>>()?;
                    for visit in staged {
                        let people = tx.execute("DELETE FROM import_people WHERE visit = ?1", params![visit])?;
                        tx.execute("DELETE FROM import_visits WHERE rowid = ?1", params![visit])?;
                        done.people -= people.min(done.people);
                        done.visits -= 1.min(done.visits);
                    }
                }
            }

            // The avatars I wore. VRCX keeps one line per avatar: when it was last put on, and the
            // time it was worn in total. An avatar this app already knows is left alone, so a later
            // look never adds the same hours twice. Each one gets a single row at the moment it
            // was last put on, with the total kept as imported time. VRCX does not say when those
            // hours were, so they are never matched against world stays.
            let worn = format!("{prefix}_avatar_history");
            if has(&worn) {
                let known = has("cache_avatar");
                let mut rows = old.prepare(&format!("SELECT avatar_id, created_at, time FROM \"{worn}\" {from} ORDER BY created_at"))?;
                let mut it = rows.query([])?;
                while let Some(r) = it.next()? {
                    let (id, at) = (text(r, 0), text(r, 1));
                    let millis = r.get::<_, Option<i64>>(2).ok().flatten().unwrap_or(0).max(0);
                    let Ok(last) = chrono::DateTime::parse_from_rfc3339(&at) else {
                        continue;
                    };
                    if !id.starts_with("avtr_") {
                        continue;
                    }
                    let already: i64 = tx.query_row("SELECT COUNT(*) FROM avatar_history WHERE avatar_id = ?1", params![id], |c| c.get(0))?;
                    if already > 0 {
                        continue;
                    }
                    let (name, author_id, author, image, status) = if known {
                        old.query_row("SELECT name, author_id, author_name, thumbnail_image_url, release_status FROM cache_avatar WHERE id = ?1", params![id], |c| {
                            let get = |i: usize| c.get::<_, Option<String>>(i).map(Option::unwrap_or_default);
                            Ok((get(0)?, get(1)?, get(2)?, get(3)?, get(4)?))
                        })
                        .unwrap_or_default()
                    } else {
                        Default::default()
                    };
                    let stamp = last.with_timezone(&chrono::Utc).format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
                    tx.execute(
                        "INSERT INTO avatar_history (avatar_id, name, author_id, author_name, image, release_status, worn_at, ended_at, imported_seconds)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)",
                        params![id, name, author_id, author, image, status, stamp, millis / 1000],
                    )?;
                    done.avatars += 1;
                }
            }

            // My own tags on avatars. One I already have here is kept.
            if !history_only && has("avatar_tags") {
                let mut rows = old.prepare("SELECT avatar_id, tag FROM avatar_tags")?;
                let mut it = rows.query([])?;
                while let Some(r) = it.next()? {
                    let (id, tag) = (text(r, 0), text(r, 1));
                    let tag = tag.trim();
                    if !id.starts_with("avtr_") || tag.is_empty() || tag.chars().count() > 40 {
                        continue;
                    }
                    done.tags += tx.execute("INSERT OR IGNORE INTO tags (item_id, tag) VALUES (?1, ?2)", params![id, tag])?;
                }
            }

            // Notes on people, worlds and avatars. One I already wrote here is kept.
            for (table, key) in [("memos", "user_id"), ("world_memos", "world_id"), ("avatar_memos", "avatar_id")] {
                if history_only || !has(table) {
                    continue;
                }
                let mut rows = old.prepare(&format!("SELECT {key}, memo, edited_at FROM {table}"))?;
                let mut it = rows.query([])?;
                while let Some(r) = it.next()? {
                    let (id, memo, at) = (text(r, 0), text(r, 1), text(r, 2));
                    if id.is_empty() || memo.trim().is_empty() {
                        continue;
                    }
                    let at = if at.is_empty() { now() } else { at };
                    done.notes += tx.execute("INSERT OR IGNORE INTO notes (user_id, note, updated_at) VALUES (?1, ?2, ?3)", params![id, memo, at])?;
                }
            }

            // The favourites VRCX kept only on this PC, with the name and picture it had for each.
            for (table, key, cache, kind) in [("favorite_world", "world_id", "cache_world", "world"), ("favorite_avatar", "avatar_id", "cache_avatar", "avatar")] {
                if history_only || !has(table) {
                    continue;
                }
                let named = has(cache);
                let mut rows = old.prepare(&format!("SELECT {key}, created_at FROM {table}"))?;
                let mut it = rows.query([])?;
                while let Some(r) = it.next()? {
                    let (id, at) = (text(r, 0), text(r, 1));
                    if id.is_empty() {
                        continue;
                    }
                    let (name, author, image) = if named {
                        old.query_row(&format!("SELECT name, author_name, thumbnail_image_url FROM {cache} WHERE id = ?1"), params![id], |c| {
                            Ok((
                                c.get::<_, Option<String>>(0)?.unwrap_or_default(),
                                c.get::<_, Option<String>>(1)?.unwrap_or_default(),
                                c.get::<_, Option<String>>(2)?.unwrap_or_default(),
                            ))
                        })
                        .unwrap_or_default()
                    } else {
                        Default::default()
                    };
                    let at = if at.is_empty() { now() } else { at };
                    done.favourites += tx.execute(
                        "INSERT OR IGNORE INTO local_favorites (kind, item_id, name, author_name, image, added_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![kind, id, name, author, image, at],
                    )?;
                }
            }
            tx.commit()?;
        }

        // New feed entries go in after the rest, sorted among themselves, a piece at a time.
        let feed_oldest_new: Option<f64> = self.db.query_row("SELECT MIN(julianday(at)) FROM import_feed", [], |r| r.get(0))?;
        self.db.execute_batch(
            "CREATE TEMP TABLE import_feed_sorted AS
                 SELECT at, user_id, display_name, kind, detail, previous, location FROM import_feed ORDER BY julianday(at), rowid;",
        )?;
        let total: i64 = self.db.query_row("SELECT COALESCE(MAX(rowid), 0) FROM import_feed_sorted", [], |r| r.get(0))?;
        let mut upto = 0;
        while upto < total {
            let tx = write()?;
            // The mark goes into the same write as the rows, so it is on the file whenever they are.
            tx.prepare_cached("INSERT INTO settings (key, value) VALUES (?1, '1') ON CONFLICT(key) DO NOTHING")?
                .execute(params![RENUMBER_FEED])?;
            let began = std::time::Instant::now();
            while upto < total && began.elapsed() < PIECE_TIME {
                tx.prepare_cached(
                    "INSERT INTO feed (at, user_id, display_name, kind, detail, previous, location)
                         SELECT at, user_id, display_name, kind, detail, previous, location FROM import_feed_sorted
                         WHERE rowid > ?1 AND rowid <= ?2 ORDER BY rowid",
                )?
                .execute(params![upto, upto + IMPORT_ROWS])?;
                upto += IMPORT_ROWS;
            }
            tx.commit()?;
        }

        // The stays, each with the people seen in it, a piece at a time in the order they were read.
        let mut after = 0i64;
        let mut more: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM import_visits)", [], |r| r.get(0))?;
        while more {
            let tx = write()?;
            tx.prepare_cached("INSERT INTO settings (key, value) VALUES (?1, '1') ON CONFLICT(key) DO NOTHING")?
                .execute(params![RENUMBER_VISITS])?;
            let began = std::time::Instant::now();
            while began.elapsed() < PIECE_TIME {
                type Staged = (i64, String, String, String, String, String);
                let rows: Vec<Staged> = tx
                    .prepare_cached("SELECT rowid, location, world_id, world_name, joined_at, left_at FROM import_visits WHERE rowid > ?1 ORDER BY rowid LIMIT ?2")?
                    .query_map(params![after, IMPORT_ROWS / 4], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
                    .collect::<Result<_, _>>()?;
                let Some(last) = rows.last() else {
                    more = false;
                    break;
                };
                after = last.0;
                for (staged, location, world_id, world_name, joined, left) in &rows {
                    tx.prepare_cached("INSERT INTO visits (location, world_id, world_name, joined_at, left_at) VALUES (?1, ?2, ?3, ?4, ?5)")?
                        .execute(params![location, world_id, world_name, joined, left])?;
                    let visit = tx.last_insert_rowid();
                    tx.prepare_cached(
                        "INSERT INTO sightings (visit_id, user_id, display_name, joined_at, left_at)
                             SELECT ?1, user_id, display_name, joined_at, left_at FROM import_people WHERE visit = ?2 ORDER BY rowid",
                    )?
                    .execute(params![visit, staged])?;
                }
            }
            tx.commit()?;
        }

        #[cfg(test)]
        if STOP_BEFORE_RENUMBER.with(|stop| stop.get()) {
            return Err(AppError::CannotDo("The import stopped before the lists were put in order.".into()));
        }

        // Both lists are read newest id first, so the ids have to follow time. When something
        // new is older than what was already here, the list is numbered again in order of time,
        // and people keep pointing at their visit. That rewrites the whole list, which takes long
        // on a big history, so it is skipped when everything new came after what was there.
        // Something the app wrote itself between two pieces can also sit out of order among the
        // new ones; that is checked only for what was added, which is quick.
        let visits_oldest_new: Option<f64> =
            self.db.query_row("SELECT MIN(julianday(joined_at)) FROM visits WHERE id > ?1", params![visits_last_id], |r| r.get(0))?;
        let older = |new: Option<f64>, last: Option<f64>| matches!((new, last), (Some(new), Some(last)) if new < last);
        let resort_feed = older(feed_oldest_new, feed_last_at) || out_of_order(&self.db, "feed", "at", feed_last_id)?;
        let resort_visits = older(visits_oldest_new, visits_last_at) || out_of_order(&self.db, "visits", "joined_at", visits_last_id)?;
        if resort_feed {
            let tx = write()?;
            renumber_feed(&tx)?;
            tx.commit()?;
        }
        if resort_visits {
            let tx = write()?;
            renumber_visits(&tx)?;
            tx.commit()?;
        }
        // A list the new rows left in order needs no renumber, so its mark is taken off. A run
        // that added nothing has no mark to take off and writes nothing here.
        let marked: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM settings WHERE key IN (?1, ?2))",
            params![RENUMBER_FEED, RENUMBER_VISITS],
            |r| r.get(0),
        )?;
        if marked {
            let tx = write()?;
            tx.execute("DELETE FROM settings WHERE key IN (?1, ?2)", params![RENUMBER_FEED, RENUMBER_VISITS])?;
            tx.commit()?;
        }
        self.db.execute_batch(
            "DROP TABLE IF EXISTS temp.import_feed;
             DROP TABLE IF EXISTS temp.import_feed_sorted;
             DROP TABLE IF EXISTS temp.import_visits;
             DROP TABLE IF EXISTS temp.import_people;
             DROP TABLE IF EXISTS temp.import_seen;",
        )?;
        if resort_feed || resort_visits {
            // A rewrite leaves a log file as big as the list. A passive fold copies what it can
            // into the history without waiting for anyone, so the app's own writes are never
            // held up by it; the log is cut back to its size limit at later folds.
            let _ = self.db.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()));
        }
        Ok(done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatar_history_counts_switches_and_skips_repeats() {
        let s = Store::in_memory().unwrap();
        assert!(s.wear_avatar("avtr_a", "2026-01-01T10:00:00.000Z").unwrap());
        assert!(!s.wear_avatar("avtr_a", "2026-01-01T10:05:00.000Z").unwrap(), "same avatar again is no switch");
        assert!(s.wear_avatar("avtr_b", "2026-01-01T11:00:00.000Z").unwrap());
        assert!(s.wear_avatar("avtr_a", "2026-01-01T11:30:00.000Z").unwrap());
        s.fill_avatar("avtr_a", "Robot", "usr_x", "Rook", "https://example.test/a.png", "public").unwrap();
        assert!(s.wear_avatar("avtr_b", "2026-01-01T12:00:00.000Z").unwrap());
        let list = s.avatar_history(10).unwrap();
        assert_eq!(list[0].avatar_id, "avtr_b");
        let a = list.iter().find(|w| w.avatar_id == "avtr_a").unwrap();
        assert_eq!((a.name.as_str(), a.times, a.seconds), ("Robot", 2, 3600 + 1800));
        assert_eq!(a.seconds_in_game, 0, "no world visited yet, so no time in VRChat");
        // In a world from 10:30 to 11:45: half an hour of the first wearing and a quarter of the second.
        s.db.execute(
            "INSERT INTO visits (location, world_id, world_name, joined_at, left_at)
             VALUES ('wrld_x:1', 'wrld_x', 'Pier', '2026-01-01T10:30:00Z', '2026-01-01T11:45:00Z')",
            [],
        )
        .unwrap();
        let list = s.avatar_history(10).unwrap();
        let a = list.iter().find(|w| w.avatar_id == "avtr_a").unwrap();
        assert_eq!(a.seconds_in_game, 1800 + 900);
        assert_eq!((a.visits, a.last_in_game.as_str()), (1, "2026-01-01T11:45:00Z"), "worn until I left the world");
        let b = list.iter().find(|w| w.avatar_id == "avtr_b").unwrap();
        assert_eq!(b.seconds_in_game, 1800, "avatar b was worn 11:00 to 11:30, all of it in the world");
        // A new row for an avatar seen before starts with its known details.
        assert!(s.wear_avatar("avtr_a", "2026-01-01T13:00:00.000Z").unwrap());
        assert_eq!(s.avatar_history(1).unwrap()[0].name, "Robot");
    }

    /// The wear time in worlds as one SQL join worked it out before the sorted sweep took over.
    fn wear_time_by_join(s: &Store, now: &str) -> std::collections::HashMap<String, (i64, i64, String)> {
        let mut stmt = s
            .db
            .prepare(
                "SELECT h.avatar_id,
                        CAST(ROUND(SUM(MIN(43200, MAX(0,
                            (MIN(julianday(COALESCE(h.ended_at, ?1)), julianday(COALESCE(v.left_at, ?1)))
                             - MAX(julianday(h.worn_at), julianday(v.joined_at))) * 86400)))) AS INTEGER),
                        COUNT(DISTINCT v.id),
                        strftime('%Y-%m-%dT%H:%M:%SZ', MAX(MIN(julianday(COALESCE(h.ended_at, ?1)), julianday(COALESCE(v.left_at, ?1)))))
                 FROM avatar_history h
                 JOIN visits v
                   ON julianday(v.joined_at) < julianday(COALESCE(h.ended_at, ?1))
                  AND julianday(COALESCE(v.left_at, ?1)) > julianday(h.worn_at)
                 WHERE h.imported_seconds IS NULL
                 GROUP BY h.avatar_id",
            )
            .unwrap();
        let rows = stmt
            .query_map(params![now], |r| {
                Ok((r.get::<_, String>(0)?, (r.get::<_, Option<i64>>(1)?.unwrap_or(0), r.get::<_, i64>(2)?, r.get::<_, Option<String>>(3)?.unwrap_or_default())))
            })
            .unwrap();
        rows.map(Result::unwrap).collect()
    }

    #[test]
    fn wear_time_in_worlds_matches_the_old_join() {
        let s = Store::in_memory().unwrap();
        // A made-up history with short and long stays, stays never closed, stays inside one
        // wearing and wearings inside one stay, times written two ways, and one unreadable time.
        let mut seed: u64 = 7;
        let mut next = |n: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        let start = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let stamp = |t: chrono::DateTime<chrono::Utc>, plain: bool| {
            if plain { t.format("%Y-%m-%dT%H:%M:%SZ").to_string() } else { t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string() }
        };
        let mut at = start;
        for i in 0..600 {
            at += chrono::Duration::seconds(next(4 * 3600) as i64);
            let length = match next(20) {
                0 => chrono::Duration::hours(30 + next(50) as i64),
                1 => chrono::Duration::days(3),
                _ => chrono::Duration::seconds(60 + next(3 * 3600) as i64),
            };
            let left = if next(40) == 0 { "NULL".to_string() } else { format!("'{}'", stamp(at + length, i % 2 == 0)) };
            s.db.execute_batch(&format!(
                "INSERT INTO visits (location, world_id, world_name, joined_at, left_at) VALUES ('wrld_{i}:1', 'wrld_{i}', '', '{}', {left});",
                stamp(at, i % 3 == 0)
            ))
            .unwrap();
        }
        s.db.execute_batch("INSERT INTO visits (location, world_id, world_name, joined_at, left_at) VALUES ('wrld_bad:1', 'wrld_bad', '', 'not a time', NULL);").unwrap();
        let mut at = start;
        for i in 0..400 {
            at += chrono::Duration::seconds(next(6 * 3600) as i64);
            let ended = if next(30) == 0 { "NULL".to_string() } else { format!("'{}'", stamp(at + chrono::Duration::seconds(next(20 * 3600) as i64), false)) };
            let imported = if next(10) == 0 { "3600" } else { "NULL" };
            s.db.execute_batch(&format!(
                "INSERT INTO avatar_history (avatar_id, worn_at, ended_at, imported_seconds) VALUES ('avtr_{}', '{}', {ended}, {imported});",
                next(25),
                stamp(at, i % 2 == 1)
            ))
            .unwrap();
        }
        let now = "2026-06-01T00:00:00.000Z";
        let old = wear_time_by_join(&s, now);
        let new = s.avatar_time_in_worlds(now).unwrap();
        assert!(old.len() > 10, "the made-up history overlaps");
        assert_eq!(new, old);
    }

    /// 20,000 stays and 2,000 wearings: times the old join against the sweep and checks they
    /// agree. Run it with `cargo test --release -- --ignored wear_time_at_scale --nocapture`.
    #[test]
    #[ignore]
    fn wear_time_at_scale() {
        let s = Store::in_memory().unwrap();
        s.db.execute_batch(
            "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 19999)
             INSERT INTO visits (location, world_id, world_name, joined_at, left_at)
                 SELECT 'wrld_' || i || ':1', 'wrld_' || i, '', strftime('%Y-%m-%dT%H:%M:%fZ', '2024-01-01', '+' || (i * 3600) || ' seconds'),
                        strftime('%Y-%m-%dT%H:%M:%fZ', '2024-01-01', '+' || (i * 3600 + 2400) || ' seconds') FROM n;
             WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 1999)
             INSERT INTO avatar_history (avatar_id, worn_at, ended_at)
                 SELECT 'avtr_' || (i % 50), strftime('%Y-%m-%dT%H:%M:%fZ', '2024-01-01', '+' || (i * 36000) || ' seconds'),
                        strftime('%Y-%m-%dT%H:%M:%fZ', '2024-01-01', '+' || (i * 36000 + 36000) || ' seconds') FROM n;",
        )
        .unwrap();
        let now = "2026-06-01T00:00:00.000Z";
        let began = std::time::Instant::now();
        let old = wear_time_by_join(&s, now);
        let join = began.elapsed();
        let began = std::time::Instant::now();
        let new = s.avatar_time_in_worlds(now).unwrap();
        println!("join {join:?}, sweep {:?}", began.elapsed());
        assert_eq!(new, old);
    }

    #[test]
    fn vrcx_duplicates_are_judged_by_the_same_seconds_as_before() {
        let dir = std::env::temp_dir().join(format!("anonymous-vrx-vrcx-edges-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let other = dir.join("VRCX.sqlite3");
        {
            let old = Connection::open(&other).unwrap();
            old.execute_batch(
                "CREATE TABLE usrme0000_feed_online_offline (id INTEGER PRIMARY KEY, created_at TEXT, user_id TEXT, display_name TEXT, type TEXT, location TEXT, world_name TEXT, time INTEGER, group_name TEXT);
                 INSERT INTO usrme0000_feed_online_offline (created_at, user_id, display_name, type, location) VALUES
                    ('2025-01-02T10:00:05.900Z', 'usr_a', 'Ana', 'Online', ''),
                    ('2025-01-02T12:00:06.000Z', 'usr_a', 'Ana', 'Online', ''),
                    ('2025-01-02T12:00:00.000Z', 'usr_a', 'Ana', 'Offline', ''),
                    ('2025-01-02T11:00:00+01:00', 'usr_b', 'Bo', 'Online', ''),
                    ('garbled', 'usr_b', 'Bo', 'Online', ''),
                    ('2025-01-02T14:00:00.000Z', 'usr_c', 'Cy', 'Online', ''),
                    ('2025-01-02T14:00:04.000Z', 'usr_c', 'Cy', 'Online', '');
                 CREATE TABLE gamelog_location (id INTEGER PRIMARY KEY, created_at TEXT, location TEXT, world_id TEXT, world_name TEXT, time INTEGER, group_name TEXT);
                 INSERT INTO gamelog_location (created_at, location, world_id, world_name, time) VALUES
                    ('2025-01-02T10:00:10.000Z', 'wrld_x:1', 'wrld_x', 'Pier', 60000),
                    ('2025-01-02T10:00:10.000Z', 'wrld_y:1', 'wrld_y', 'Hall', 60000),
                    ('2025-01-02T10:00:11.000Z', 'wrld_x:1', 'wrld_x', 'Pier', 60000);",
            )
            .unwrap();
        }
        let s = Store::in_memory().unwrap();
        s.db.execute_batch(
            "INSERT INTO feed (at, user_id, display_name, kind) VALUES
                ('2025-01-02T10:00:00.000Z', 'usr_a', 'Ana', 'online'),
                ('2025-01-02T12:00:00.000Z', 'usr_a', 'Ana', 'online'),
                ('2025-01-02T10:00:00Z', 'usr_b', 'Bo', 'online');
             INSERT INTO visits (location, world_id, world_name, joined_at, left_at) VALUES
                ('wrld_x:1', 'wrld_x', 'Pier', '2025-01-02T10:00:00.000Z', '2025-01-02T10:30:00.000Z'),
                ('wrld_y:1', 'wrld_y', 'Hall', '2025-01-02 10:00:00', NULL);",
        )
        .unwrap();
        let got = s.import_vrcx(&other, "usr_me-0000", true, None).unwrap();
        // Ana online 5.9 s later counts as 5 whole seconds: already here. Six seconds is new, and
        // an offline is another kind. Bo's time in another zone is the same moment: already here;
        // a time that cannot be read is always new. Cy's two lines four seconds apart are one.
        assert_eq!(got.feed, 4);
        let kept: Vec<(String, String)> = {
            let mut stmt = s.db.prepare("SELECT user_id, at FROM feed WHERE id > 3 ORDER BY id").unwrap();
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
            rows.map(Result::unwrap).collect()
        };
        assert!(kept.contains(&("usr_a".to_string(), "2025-01-02T12:00:06.000Z".to_string())));
        assert!(kept.contains(&("usr_b".to_string(), "garbled".to_string())));
        assert_eq!(kept.iter().filter(|k| k.0 == "usr_c").count(), 1);
        // Ten seconds from a stay in the same instance is the same stay, written either way;
        // eleven seconds is another one.
        assert_eq!(got.visits, 1);
        // A second run finds everything again except the line with no readable time, which
        // matches nothing, as it never did.
        assert_eq!(s.import_vrcx(&other, "usr_me-0000", true, None).unwrap(), BroughtOver { feed: 1, ..BroughtOver::default() });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn feed_search_filters_by_text_kind_and_time() {
        let s = Store::in_memory().unwrap();
        s.save_world(&World { id: "wrld_pier".into(), name: "Quiet Pier".into(), image: String::new() }).unwrap();
        let add = |name: &str, kind: &'static str, detail: &str, location: &str| {
            s.add_feed(&FeedDraft {
                user_id: format!("usr_{name}"),
                display_name: name.into(),
                kind,
                detail: detail.into(),
                previous: String::new(),
                location: location.into(),
            })
            .unwrap()
        };
        add("Mossy", "online", "", "");
        add("Pip", "location", "", "wrld_pier:1~region(eu)");
        add("Mossy", "statusText", "100% tired", "");
        assert_eq!(s.feed_search("moss", &[], None, None, 50).unwrap().len(), 2);
        assert_eq!(s.feed_search("pier", &[], None, None, 50).unwrap()[0].display_name, "Pip");
        assert_eq!(s.feed_search("100%", &[], None, None, 50).unwrap().len(), 1);
        assert_eq!(s.feed_search("", &["online".into(), "bogus".into()], None, None, 50).unwrap().len(), 1);
        assert!(s.feed_search("", &[], None, Some("2000-01-01"), 50).unwrap().is_empty());
        let all = s.feed_search("", &[], None, None, 50).unwrap();
        assert_eq!(s.feed_search("", &[], Some(all[0].id), None, 50).unwrap().len(), 2);
    }

    #[test]
    fn feed_search_ignores_case_in_every_alphabet() {
        let s = Store::in_memory().unwrap();
        s.save_world(&World { id: "wrld_hall".into(), name: "Зал Über".into(), image: String::new() }).unwrap();
        for (name, kind, location) in [("Мария", "online", ""), ("Έλενα", "online", ""), ("Pip", "location", "wrld_hall:1")] {
            s.add_feed(&FeedDraft {
                user_id: format!("usr_{name}"),
                display_name: name.into(),
                kind,
                detail: String::new(),
                previous: String::new(),
                location: location.into(),
            })
            .unwrap();
        }
        assert_eq!(s.feed_search("мария", &[], None, None, 50).unwrap()[0].display_name, "Мария");
        assert_eq!(s.feed_search("έλενα", &[], None, None, 50).unwrap()[0].display_name, "Έλενα");
        assert_eq!(s.feed_search("über", &[], None, None, 50).unwrap()[0].display_name, "Pip", "world names fold too");
        assert_eq!(s.feed_search("ЗАЛ", &[], None, None, 50).unwrap().len(), 1);
    }

    #[test]
    fn past_names_come_from_the_feed_and_the_game_log() {
        let s = Store::in_memory().unwrap();
        s.add_feed(&FeedDraft { user_id: "usr_a".into(), display_name: "Ghost".into(), kind: "name",
            detail: "Ghost".into(), previous: "Fallen".into(), location: String::new() }).unwrap();
        s.db.execute("INSERT INTO visits (location, joined_at) VALUES ('wrld_x:1', '2025-01-01T00:00:00.000Z')", []).unwrap();
        s.db.execute("INSERT INTO sightings (visit_id, user_id, display_name, joined_at) VALUES (1, 'usr_a', 'Oldest', '2025-01-01T00:00:00.000Z')", []).unwrap();
        s.db.execute("INSERT INTO sightings (visit_id, user_id, display_name, joined_at) VALUES (1, 'usr_b', 'Other', '2025-01-01T00:00:00.000Z')", []).unwrap();
        let names: Vec<String> = s.past_names("usr_a").unwrap().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, vec!["Fallen", "Oldest"]);
    }

    #[test]
    fn an_avatar_that_drops_off_my_list_is_kept_as_lost() {
        let s = Store::in_memory().unwrap();
        assert!(s.lost_avatars().unwrap().is_empty());
        let a = ("avtr_a".to_string(), "Robot".to_string(), String::new());
        let b = ("avtr_b".to_string(), "Fox".to_string(), String::new());
        s.record_own_avatars(&[a.clone(), b.clone()]).unwrap();
        assert!(s.lost_avatars().unwrap().is_empty());
        std::thread::sleep(std::time::Duration::from_millis(5));
        s.record_own_avatars(&[a.clone()]).unwrap();
        let lost = s.lost_avatars().unwrap();
        assert_eq!((lost.len(), lost[0].avatar_id.as_str(), lost[0].name.as_str()), (1, "avtr_b", "Fox"));
        // Back on the account: no longer lost.
        std::thread::sleep(std::time::Duration::from_millis(5));
        s.record_own_avatars(&[a, b]).unwrap();
        assert!(s.lost_avatars().unwrap().is_empty());
    }

    #[test]
    fn local_favourites_add_update_remove() {
        let s = Store::in_memory().unwrap();
        let f = LocalFavorite { kind: "avatar".into(), item_id: "avtr_a".into(), name: "Robot".into(),
            author_name: "Rook".into(), image: String::new(), added_at: String::new() };
        s.add_local_favorite(&f).unwrap();
        s.add_local_favorite(&LocalFavorite { name: "Robot 2".into(), ..f.clone() }).unwrap();
        let list = s.local_favorites("avatar").unwrap();
        assert_eq!((list.len(), list[0].name.as_str()), (1, "Robot 2"));
        assert!(s.local_favorites("world").unwrap().is_empty());
        s.remove_local_favorite("avatar", "avtr_a").unwrap();
        assert!(s.local_favorites("avatar").unwrap().is_empty());
    }

    fn friend(id: &str, name: &str) -> Friend {
        Friend {
            id: id.into(),
            display_name: name.into(),
            state: "online".into(),
            ..Default::default()
        }
    }

    #[test]
    fn friends_round_trip() {
        let mut s = Store::in_memory().unwrap();
        s.replace_friends(&[friend("usr_1", "Mossy"), friend("usr_2", "Pip")])
            .unwrap();
        assert_eq!(s.friends().unwrap().len(), 2);
        let mut changed = friend("usr_2", "Pippin");
        changed.state = "offline".into();
        s.save_friend(&changed).unwrap();
        s.remove_friend("usr_1").unwrap();
        let left = s.friends().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].display_name, "Pippin");
    }

    #[test]
    fn feed_is_newest_first_and_pages() {
        let s = Store::in_memory().unwrap();
        for i in 0..5 {
            s.add_feed(&FeedDraft {
                user_id: "usr_1".into(),
                display_name: "Mossy".into(),
                kind: "location",
                detail: format!("{i}"),
                previous: String::new(),
                location: "wrld_a:1~region(eu)".into(),
            })
            .unwrap();
        }
        let first = s.feed(2, None).unwrap();
        let details = |items: &[FeedItem]| items.iter().map(|f| f.detail.clone()).collect::<Vec<_>>();
        assert_eq!(details(&first), ["4", "3"]);
        assert_eq!(first[0].world_id.as_deref(), Some("wrld_a"));
        let next = s.feed(2, Some(first[1].id)).unwrap();
        assert_eq!(details(&next), ["2", "1"]);
    }

    #[test]
    fn handled_notices_are_kept_newest_first() {
        let s = Store::in_memory().unwrap();
        let first = crate::vrchat::notices::Notice::from_v1(&serde_json::json!({
            "id": "frq_1", "type": "friendRequest", "senderUserId": "usr_5", "senderUsername": "Rook",
            "message": "", "created_at": "2026-01-05T20:00:00.000Z" }))
        .unwrap();
        let second = crate::vrchat::notices::Notice::from_v2(&serde_json::json!({
            "id": "not_g", "type": "group.invite", "title": "Night Owls", "message": "invited you",
            "createdAt": "2026-01-05T22:00:00.000Z",
            "responses": [ { "type": "decline", "data": "grp_1", "text": "Decline group invite" } ] }))
        .unwrap();
        s.log_handled(&first, "accept", "").unwrap();
        s.log_handled(&second, "reply", "Decline group invite").unwrap();
        let log = s.handled(10).unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].notice.title, "Night Owls");
        assert_eq!(log[0].reply_text, "Decline group invite");
        assert_eq!(log[1].action, "accept");
        assert_eq!(log[1].notice.sender_name, "Rook");
    }

    #[test]
    fn copy_holds_everything_and_opens_alone() {
        let dir = std::env::temp_dir().join(format!("anonymous-vrx-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let s = Store::open(&dir.join("live.sqlite")).unwrap();
        s.save_world(&World { id: "wrld_a".into(), name: "Quiet Pier".into(), image: String::new() })
            .unwrap();
        s.set_setting("favorites", "{}").unwrap();
        let target = dir.join("copy.sqlite");
        s.copy_to(&target).unwrap();
        // A second copy to the same place must not silently overwrite.
        assert!(s.copy_to(&target).is_err());
        let copy = Store::open(&target).unwrap();
        assert_eq!(copy.world("wrld_a").unwrap().unwrap().name, "Quiet Pier");
        assert_eq!(copy.setting("favorites").unwrap().as_deref(), Some("{}"));
        drop(copy);
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn arrival_comes_from_the_latest_move_only() {
        let s = Store::in_memory().unwrap();
        let go = |location: &str| {
            s.add_feed(&FeedDraft {
                user_id: "usr_1".into(),
                display_name: "Mossy".into(),
                kind: "location",
                detail: String::new(),
                previous: String::new(),
                location: location.into(),
            })
            .unwrap()
        };
        go("wrld_a:1");
        let second = go("wrld_b:2");
        assert_eq!(s.arrived_at("usr_1", "wrld_b:2").unwrap(), Some(second.at));
        // They left wrld_a since, so an old visit there says nothing about now.
        assert_eq!(s.arrived_at("usr_1", "wrld_a:1").unwrap(), None);
        assert_eq!(s.arrived_at("usr_2", "wrld_b:2").unwrap(), None);
    }

    #[test]
    fn notes_save_change_and_clear() {
        let s = Store::in_memory().unwrap();
        assert_eq!(s.note("usr_1").unwrap(), "");
        s.set_note("usr_1", "met at the pier").unwrap();
        s.set_note("usr_1", "met at the pier, plays drums").unwrap();
        assert_eq!(s.note("usr_1").unwrap(), "met at the pier, plays drums");
        assert_eq!(s.noted().unwrap(), ["usr_1"]);
        s.set_note("usr_1", "   ").unwrap();
        assert_eq!(s.note("usr_1").unwrap(), "");
        assert!(s.noted().unwrap().is_empty());
    }

    #[test]
    fn together_counts_overlapping_rows_once() {
        use crate::gamelog::LogEvent;
        let s = Store::in_memory().unwrap();
        let at = |t: &str| format!("2026-01-05T{t}Z");
        s.record_log(
            &[
                (at("20:00:00"), LogEvent::Joining("wrld_a:1".into())),
                (at("20:00:00"), LogEvent::PlayerJoined { name: "Pip".into(), id: "usr_pip".into() }),
                (at("21:00:00"), LogEvent::LeftRoom),
            ],
            true,
            "log:test",
            1,
        )
        .unwrap();
        // A second row for the same hour, as an import can leave.
        s.db.execute(
            "INSERT INTO sightings (visit_id, user_id, display_name, joined_at, left_at) VALUES (1, 'usr_pip', 'Pip', '2026-01-05T20:30:00.000Z', '2026-01-05T21:00:00.000Z')",
            [],
        )
        .unwrap();
        assert_eq!(s.together("usr_pip").unwrap().seconds, 3600);
    }

    #[test]
    fn a_stay_never_ends_before_it_began() {
        use crate::gamelog::LogEvent;
        let s = Store::in_memory().unwrap();
        let at = |t: &str| format!("2026-10-25T{t}Z");
        // The clocks went back between the join and the leave, so the leave reads earlier.
        s.record_log(
            &[
                (at("02:10:00"), LogEvent::Joining("wrld_a:1".into())),
                (at("02:12:00"), LogEvent::PlayerJoined { name: "Pip".into(), id: "usr_pip".into() }),
                (at("02:05:00"), LogEvent::PlayerLeft { name: "Pip".into(), id: "usr_pip".into() }),
                (at("02:20:00"), LogEvent::PlayerJoined { name: "Ash".into(), id: "usr_ash".into() }),
                (at("02:01:00"), LogEvent::LeftRoom),
            ],
            true,
            "log:test",
            1,
        )
        .unwrap();
        let stay = s.visits(10, None).unwrap().remove(0);
        assert_eq!(stay.left_at.as_deref(), Some(stay.joined_at.as_str()));
        let mut people = s.visit_people(stay.id).unwrap();
        people.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        for p in &people {
            assert_eq!(p.left_at.as_deref(), Some(p.joined_at.as_str()), "{}", p.display_name);
        }

        // A stay closed after a crash, where only an earlier time was seen in it.
        s.record_log(&[(at("03:30:00"), LogEvent::Joining("wrld_b:1".into()))], true, "log:test", 2).unwrap();
        s.db.execute(
            "INSERT INTO sightings (visit_id, user_id, display_name, joined_at) VALUES (2, 'usr_old', 'Old', '2026-10-25T02:50:00Z')",
            [],
        )
        .unwrap();
        s.record_log(&[], true, "log:test", 3).unwrap();
        let crashed = s.visits(10, None).unwrap().remove(0);
        assert_eq!((crashed.id, crashed.left_at.as_deref()), (2, Some(at("03:30:00").as_str())));
    }

    #[test]
    fn finding_the_stay_at_a_moment_uses_an_index() {
        let s = Store::in_memory().unwrap();
        let sql = format!(
            "EXPLAIN QUERY PLAN SELECT {VISIT_COLUMNS} FROM visits v
             WHERE v.joined_at <= ?1 ORDER BY v.joined_at DESC, v.id DESC LIMIT 1"
        );
        let mut stmt = s.db.prepare(&sql).unwrap();
        let plan: Vec<String> = stmt.query_map(params!["2026-01-01T00:00:00Z"], |r| r.get::<_, String>(3)).unwrap().map(Result::unwrap).collect();
        assert!(plan.iter().any(|p| p.contains("visits_joined")), "{plan:?}");
        // The order comes from the index too, with no sort of every stay.
        assert!(!plan.iter().any(|p| p.contains("TEMP B-TREE FOR ORDER BY")), "{plan:?}");
    }

    #[test]
    fn friends_since_takes_the_first_add_of_a_current_friend() {
        let s = Store::in_memory().unwrap();
        s.db.execute_batch(
            "INSERT INTO friends (user_id, data, updated_at) VALUES ('usr_a', '{}', 'x');
             INSERT INTO feed (at, user_id, display_name, kind) VALUES
               ('2024-03-01T10:00:00.000Z', 'usr_a', 'Ana', 'friendAdded'),
               ('2025-01-01T10:00:00.000Z', 'usr_a', 'Ana', 'friendRemoved'),
               ('2025-02-01T10:00:00.000Z', 'usr_a', 'Ana', 'friendAdded'),
               ('2024-05-01T10:00:00.000Z', 'usr_gone', 'Gone', 'friendAdded');",
        )
        .unwrap();
        assert_eq!(s.friends_since_all().unwrap(), vec![("usr_a".to_string(), "2024-03-01T10:00:00.000Z".to_string())]);
        assert_eq!(s.friends_since("usr_gone").unwrap().as_deref(), Some("2024-05-01T10:00:00.000Z"));
        assert_eq!(s.friends_since("usr_none").unwrap(), None);
    }

    #[test]
    fn video_pages_search_and_follow_on() {
        use crate::gamelog::LogEvent;
        let s = Store::in_memory().unwrap();
        let at = |t: &str| format!("2026-01-05T{t}Z");
        let mut events = vec![(at("20:00:00"), LogEvent::Room("100% Movies".into())), (at("20:00:00"), LogEvent::Joining("wrld_a:1".into()))];
        for n in 0..5 {
            events.push((at(&format!("20:0{n}:30")), LogEvent::Video { url: format!("https://example.test/clip_{n}.mp4") }));
        }
        s.record_log(&events, true, "log:test", 1).unwrap();
        let (first, total) = s.videos_page("", false, None, 2).unwrap();
        assert_eq!((first.len(), total), (2, 5));
        let last = first.last().unwrap();
        let (next, _) = s.videos_page("", false, Some((last.at.clone(), last.id)), 10).unwrap();
        assert_eq!(next.len(), 3);
        assert!(next.iter().all(|v| v.at < last.at));
        // % and _ are plain letters in a search.
        assert_eq!(s.videos_page("100%", false, None, 10).unwrap().1, 5);
        assert_eq!(s.videos_page("clip_3", false, None, 10).unwrap().1, 1);
        assert_eq!(s.videos_page("clip%3", false, None, 10).unwrap().1, 0);
        // The first one played 30 s after I arrived, with nobody named: left out when asked.
        assert_eq!(s.videos_page("", true, None, 10).unwrap().1, 4);
        // Case does not matter, in any alphabet.
        s.save_video_title("AbCdEfGhIjK", "Über Ночь", "").unwrap();
        s.db.execute("UPDATE videos SET youtube_id = 'AbCdEfGhIjK' WHERE id = 1", []).unwrap();
        assert_eq!(s.videos_page("über ночь", false, None, 10).unwrap().1, 1);
        assert_eq!(s.videos_page("100% MOVIES", false, None, 10).unwrap().1, 5);
    }

    #[test]
    fn vrcx_import_keeps_videos_with_their_stay() {
        use crate::gamelog::LogEvent;
        let dir = std::env::temp_dir().join(format!("anonymous-vrx-vrcx-videos-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let other = dir.join("VRCX.sqlite3");
        {
            let old = Connection::open(&other).unwrap();
            old.execute_batch(
                "CREATE TABLE gamelog_location (id INTEGER PRIMARY KEY, created_at TEXT, location TEXT, world_id TEXT, world_name TEXT, time INTEGER, group_name TEXT);
                 INSERT INTO gamelog_location (created_at, location, world_id, world_name, time) VALUES ('2025-01-02T12:00:00.000Z', 'wrld_old:1', 'wrld_old', 'Old Pier', 3600000);",
            )
            .unwrap();
        }
        let s = Store::in_memory().unwrap();
        let at = |t: &str| format!("2026-01-05T{t}Z");
        let events = vec![
            (at("20:00:00"), LogEvent::Room("Movie Night".into())),
            (at("20:00:00"), LogEvent::Joining("wrld_new:1".into())),
            (at("20:00:12"), LogEvent::Video { url: "https://youtu.be/AbCdEfGhIjK".into() }),
            (at("21:00:00"), LogEvent::LeftRoom),
        ];
        s.record_log(&events, true, "log:test", 1).unwrap();
        assert_eq!(s.import_vrcx(&other, "usr_me-0000", false, None).unwrap().visits, 1);
        assert_eq!(s.videos(10).unwrap()[0].world_name, "Movie Night");
        let stay = s.visits(10, None).unwrap().into_iter().find(|v| v.world_name == "Movie Night").unwrap();
        assert_eq!(s.visit_videos(stay.id).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn who_put_a_video_on_survives_a_split_read() {
        use crate::gamelog::LogEvent;
        let s = Store::in_memory().unwrap();
        let url = "https://youtu.be/AbCdEfGhIjK".to_string();
        s.record_log(
            &[
                ("2026-01-05T20:00:00Z".into(), LogEvent::Joining("wrld_a:1".into())),
                ("2026-01-05T20:01:00Z".into(), LogEvent::VideoBy { url: url.clone(), name: "Pip".into() }),
            ],
            true,
            "log:test",
            1,
        )
        .unwrap();
        s.record_log(&[("2026-01-05T20:01:00Z".into(), LogEvent::Video { url })], false, "log:test", 2).unwrap();
        assert_eq!(s.videos(10).unwrap()[0].requested_by, "Pip");
        assert!(s.setting("video:asked-by").unwrap().is_none());
    }

    #[test]
    fn empty_titles_are_asked_again_after_a_week() {
        let s = Store::in_memory().unwrap();
        s.save_video_title("AbCdEfGhIjK", "", "").unwrap();
        s.save_video_title("ZyXwVuTsRqP", "A title", "A channel").unwrap();
        let ids = vec!["AbCdEfGhIjK".to_string(), "ZyXwVuTsRqP".to_string()];
        assert_eq!(s.video_titles(&ids).unwrap().len(), 2);
        s.db.execute("UPDATE video_titles SET fetched_at = '2020-01-01T00:00:00.000Z'", []).unwrap();
        let left = s.video_titles(&ids).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].title, "A title");
    }

    #[test]
    fn together_adds_up_shared_time() {
        use crate::gamelog::LogEvent;
        let s = Store::in_memory().unwrap();
        let at = |t: &str| format!("2026-01-05T{t}Z");
        let join = |id: &str| LogEvent::PlayerJoined { name: id.into(), id: id.into() };
        let events = vec![
            (at("20:00:00"), LogEvent::Joining("wrld_a:1".into())),
            (at("20:00:10"), join("usr_pip")),
            (at("20:30:10"), LogEvent::PlayerLeft { name: "usr_pip".into(), id: "usr_pip".into() }),
            (at("21:00:00"), LogEvent::Joining("wrld_b:2".into())),
            (at("21:00:00"), join("usr_pip")),
            (at("21:10:00"), LogEvent::LeftRoom),
        ];
        s.record_log(&events, true, "log:test", 1).unwrap();
        let t = s.together("usr_pip").unwrap();
        assert_eq!(t.times, 2);
        assert_eq!(t.seconds, 30 * 60 + 10 * 60);
        assert_eq!(t.first.as_deref(), Some("2026-01-05T20:00:10Z"));
        assert_eq!(t.last.as_deref(), Some("2026-01-05T21:10:00Z"));
        let nobody = s.together("usr_never").unwrap();
        assert_eq!((nobody.times, nobody.seconds, nobody.first), (0, 0, None));
    }

    #[test]
    fn friend_log_holds_only_list_changes() {
        let s = Store::in_memory().unwrap();
        for kind in ["online", "friendAdded", "location", "name", "friendRemoved"] {
            s.add_feed(&FeedDraft {
                user_id: "usr_1".into(),
                display_name: "Mossy".into(),
                kind,
                detail: String::new(),
                previous: String::new(),
                location: String::new(),
            })
            .unwrap();
        }
        let kinds: Vec<String> = s.friend_log(10).unwrap().into_iter().map(|f| f.kind).collect();
        assert_eq!(kinds, ["friendRemoved", "name", "friendAdded"]);
        assert_eq!(s.feed_for("usr_1", 2).unwrap().len(), 2);
        assert!(s.feed_for("usr_2", 10).unwrap().is_empty());
    }

    #[test]
    fn worlds_are_cached() {
        let s = Store::in_memory().unwrap();
        assert!(s.world("wrld_a").unwrap().is_none());
        s.save_world(&World {
            id: "wrld_a".into(),
            name: "Quiet Pier".into(),
            image: String::new(),
        })
        .unwrap();
        assert_eq!(s.world("wrld_a").unwrap().unwrap().name, "Quiet Pier");
    }

    #[test]
    fn ratings_last_seen_and_world_visits() {
        let s = Store::in_memory().unwrap();
        assert!(s.ratings().unwrap().is_empty());
        s.set_rating("wrld_a", 4, "2026-01-01T00:00:00Z").unwrap();
        s.set_rating("wrld_a", 9, "2026-01-02T00:00:00Z").unwrap();
        s.set_rating("wrld_b", 2, "2026-01-02T00:00:00Z").unwrap();
        s.set_rating("wrld_b", 0, "2026-01-03T00:00:00Z").unwrap();
        assert_eq!(s.ratings().unwrap(), vec![("wrld_a".to_string(), 5)]);

        s.add_feed(&FeedDraft { user_id: "usr_a".into(), display_name: "Ana".into(), kind: "online", detail: String::new(), previous: String::new(), location: String::new() }).unwrap();
        // A name change says nothing about when they were last around.
        s.add_feed(&FeedDraft { user_id: "usr_b".into(), display_name: "Bo".into(), kind: "name", detail: "Bo".into(), previous: "B".into(), location: String::new() }).unwrap();
        let seen = s.last_seen().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "usr_a");
        assert!(s.world_visits().unwrap().is_empty());

        s.set_tags("avtr_a", &["Dance".to_string(), "club".to_string()]).unwrap();
        s.set_tags("avtr_b", &["club".to_string()]).unwrap();
        s.set_tags("avtr_a", &["club".to_string()]).unwrap();
        assert_eq!(s.tags().unwrap(), vec![("avtr_a".to_string(), "club".to_string()), ("avtr_b".to_string(), "club".to_string())]);
        s.set_tags("avtr_b", &[]).unwrap();
        assert_eq!(s.tags().unwrap().len(), 1);
    }

    #[test]
    fn friends_of_friends_are_kept_as_pairs_and_replaced_on_a_new_look() {
        let s = Store::in_memory().unwrap();
        s.set_mutuals("usr_b", &["usr_a".into(), "usr_c".into(), "usr_b".into()], "2026-11-25T10:00:00Z", false).unwrap();
        let mut all = s.mutuals().unwrap();
        all.sort();
        assert_eq!(all, vec![("usr_a".to_string(), "usr_b".to_string()), ("usr_b".to_string(), "usr_c".to_string())]);
        // The other side reporting the same friendship does not double it.
        s.set_mutuals("usr_a", &["usr_b".into()], "2026-11-25T10:00:01Z", false).unwrap();
        assert_eq!(s.mutuals().unwrap().len(), 2);
        // Someone who now hides their friends keeps the pairs others reported.
        s.set_mutuals("usr_b", &[], "2026-11-26T09:00:00Z", true).unwrap();
        assert_eq!(s.mutuals().unwrap().len(), 2);
        // A new look at usr_b that does answer replaces what was known about them.
        s.set_mutuals("usr_b", &[], "2026-11-26T10:00:00Z", false).unwrap();
        assert!(s.mutuals().unwrap().is_empty());
        s.set_mutuals("usr_b", &[], "2026-11-26T10:00:00Z", true).unwrap();
        let asked = s.mutuals_asked().unwrap();
        assert!(asked.contains(&("usr_b".to_string(), "2026-11-26T10:00:00Z".to_string(), true)));
    }

    #[test]
    fn my_history_is_searched_by_person_and_by_world() {
        let s = Store::in_memory().unwrap();
        let visit = |id: i64, world: &str, name: &str, from: &str, to: &str| {
            s.db.execute("INSERT INTO visits (id, location, world_id, world_name, joined_at, left_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![id, format!("{world}:1~region(eu)"), world, name, from, to]).unwrap();
        };
        let seen = |visit: i64, user: &str, name: &str, from: &str, to: &str| {
            s.db.execute("INSERT INTO sightings (visit_id, user_id, display_name, joined_at, left_at) VALUES (?1, ?2, ?3, ?4, ?5)", params![visit, user, name, from, to]).unwrap();
        };
        visit(1, "wrld_pier", "Quiet Pier", "2026-11-01T10:00:00Z", "2026-11-01T12:00:00Z");
        visit(2, "wrld_arcade", "Basement Arcade", "2026-11-02T10:00:00Z", "2026-11-02T11:00:00Z");
        visit(3, "wrld_pier", "Quiet Pier", "2026-11-03T10:00:00Z", "2026-11-03T10:30:00Z");
        for v in 1..=3 {
            seen(v, "usr_me", "Me", "2026-11-01T10:00:00Z", "2026-11-03T10:30:00Z");
        }
        seen(1, "usr_a", "Mossy", "2026-11-01T10:00:00Z", "2026-11-01T11:00:00Z");
        seen(1, "usr_b", "Wren", "2026-11-01T10:10:00Z", "2026-11-01T10:50:00Z");
        seen(2, "usr_a", "Mossy", "2026-11-02T10:00:00Z", "2026-11-02T10:30:00Z");
        seen(2, "usr_b", "Wren", "2026-11-02T10:05:00Z", "2026-11-02T10:20:00Z");
        // Renamed since: found under the old name, shown under the new one, counted once.
        seen(3, "usr_a", "Mossy Stone", "2026-11-03T10:00:00Z", "2026-11-03T10:30:00Z");
        // Someone who came after Mossy had left is no company of theirs.
        seen(1, "usr_c", "Latecomer", "2026-11-01T11:30:00Z", "2026-11-01T11:50:00Z");
        seen(2, "usr_c", "Latecomer", "2026-11-02T10:40:00Z", "2026-11-02T10:50:00Z");

        let (people, worlds) = s.log_find("moss", "usr_me").unwrap();
        assert_eq!(people.len(), 1);
        assert_eq!((people[0].name.as_str(), people[0].times, people[0].seconds), ("Mossy Stone", 3, 3600 + 1800 + 1800));
        assert_eq!((people[0].last_world.as_str(), people[0].first.as_str()), ("Quiet Pier", "2026-11-01T10:00:00Z"));
        assert!(worlds.is_empty());
        // I am not a find in my own history, and one letter is no search.
        assert!(s.log_find("Me", "usr_me").unwrap().0.iter().all(|p| p.user_id != "usr_me"));
        assert!(s.log_find("m", "usr_me").unwrap().0.is_empty());
        // A percent sign is a plain character.
        assert!(s.log_find("%%", "usr_me").unwrap().0.is_empty());

        let (_, worlds) = s.log_find("pier", "usr_me").unwrap();
        assert_eq!((worlds.len(), worlds[0].times, worlds[0].seconds), (1, 2, 7200 + 1800));
        assert_eq!(worlds[0].last, "2026-11-03T10:00:00Z");

        let stays = s.stays_with("usr_a", "", "usr_me", 50).unwrap();
        assert_eq!(stays.iter().map(|x| (x.visit_id, x.seconds)).collect::<Vec<_>>(), vec![(3, 1800), (2, 1800), (1, 3600)]);
        // Others there: Wren and the latecomer, not Mossy and not me.
        assert_eq!((stays[2].people, stays[0].people), (2, 0));
        let stays = s.stays_in("wrld_pier", "", "usr_me", 50).unwrap();
        assert_eq!(stays.iter().map(|x| (x.visit_id, x.people)).collect::<Vec<_>>(), vec![(3, 1), (1, 3)]);

        // A doubled log line for someone already there counts their time once.
        seen(2, "usr_b", "Wren", "2026-11-02T10:05:00Z", "2026-11-02T10:20:00Z");
        let (people, _) = s.log_find("WREN", "usr_me").unwrap();
        assert_eq!((people[0].times, people[0].seconds), (2, 2400 + 900));
        // Letters outside a to z fold too.
        seen(3, "usr_e", "Émile", "2026-11-03T10:00:00Z", "2026-11-03T10:10:00Z");
        assert_eq!(s.log_find("émi", "usr_me").unwrap().0.len(), 1);

        s.db.execute("INSERT INTO worlds (world_id, name, fetched_at) VALUES ('wrld_club', 'Night Club', '2026-11-01T00:00:00Z')", []).unwrap();
        for at in ["2026-11-02T20:00:00Z", "2026-11-03T21:00:00Z"] {
            s.db.execute("INSERT INTO feed (at, user_id, display_name, kind, location) VALUES (?1, 'usr_a', 'Mossy', 'location', 'wrld_club:9~region(eu)')", params![at]).unwrap();
        }
        s.db.execute("INSERT INTO feed (at, user_id, display_name, kind, location) VALUES ('2026-10-01T20:00:00Z', 'usr_a', 'Mossy', 'location', 'wrld_old:9')", []).unwrap();
        let h = s.habits("usr_a", "usr_me", "2026-11-01T00:00:00Z").unwrap();
        assert_eq!((h.worlds[0].name.as_str(), h.worlds[0].times), ("Quiet Pier", 2));
        assert_eq!(h.company.iter().map(|p| (p.user_id.as_str(), p.times)).collect::<Vec<_>>(), vec![("usr_b", 2)]);
        assert_eq!(h.together.len(), 3);
        assert_eq!(h.worlds[0].seconds, 3600 + 1800);
        assert_eq!(h.own_worlds.iter().map(|w| (w.name.as_str(), w.times)).collect::<Vec<_>>(), vec![("Night Club", 2)]);
        assert_eq!(h.seen_at.len(), 2);
    }

    #[test]
    fn friends_seen_together_often_are_paired() {
        let s = Store::in_memory().unwrap();
        for visit in 1..=4 {
            for user in ["usr_a", "usr_b"] {
                s.db.execute("INSERT INTO sightings (visit_id, user_id, display_name, joined_at) VALUES (?1, ?2, 'x', '2026-01-01T00:00:00Z')", params![visit, user]).unwrap();
            }
        }
        // Twice in one stay counts once, and one shared stay is not "often".
        s.db.execute("INSERT INTO sightings (visit_id, user_id, display_name, joined_at) VALUES (1, 'usr_a', 'x', '2026-01-01T01:00:00Z')", []).unwrap();
        s.db.execute("INSERT INTO sightings (visit_id, user_id, display_name, joined_at) VALUES (1, 'usr_c', 'x', '2026-01-01T00:00:00Z')", []).unwrap();
        let all: Vec<String> = ["usr_a", "usr_b", "usr_c", "usr_d"].iter().map(|s| s.to_string()).collect();
        assert_eq!(s.pairs(&all, 3, 40).unwrap(), vec![("usr_a".to_string(), "usr_b".to_string(), 4)]);
        // People not asked about are left out, and an id that is not one is ignored.
        assert!(s.pairs(&["usr_a".to_string(), "usr_c".to_string()], 3, 40).unwrap().is_empty());
        assert!(s.pairs(&["usr_a".to_string(), "x') OR 1=1 --".to_string()], 1, 40).unwrap().is_empty());
    }

    #[test]
    fn my_worlds_are_noted_once_a_day_and_read_back_in_order() {
        let s = Store::in_memory().unwrap();
        s.record_world_stats("2026-11-24", &[("wrld_a".into(), 100, 10, 3)]).unwrap();
        s.record_world_stats("2026-11-25", &[("wrld_a".into(), 130, 12, 7), ("wrld_b".into(), 5, 0, 0)]).unwrap();
        // Later the same day: the totals move on, the most people seen stays the day's peak.
        s.record_world_stats("2026-11-25", &[("wrld_a".into(), 140, 12, 2)]).unwrap();
        let all = s.world_stats("2026-11-01").unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0], ("wrld_a".into(), "2026-11-24".into(), 100, 10, 3));
        assert_eq!(all[1], ("wrld_a".into(), "2026-11-25".into(), 140, 12, 7));
        assert_eq!(s.world_stats("2026-11-25").unwrap().len(), 2);
    }

    #[test]
    fn avatars_worn_in_vrcx_come_over_with_their_time_and_tags() {
        let dir = std::env::temp_dir().join(format!("anonymous-vrx-bring-avatars-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let other = dir.join("other.sqlite3");
        {
            let old = Connection::open(&other).unwrap();
            old.execute_batch(
                "CREATE TABLE usrme0000_avatar_history (avatar_id TEXT PRIMARY KEY, created_at TEXT, time INTEGER);
                 INSERT INTO usrme0000_avatar_history VALUES ('avtr_fox', '2025-03-01T20:00:00.000Z', 5400000), ('avtr_owl', '2025-03-02T20:00:00.000Z', NULL), ('avtr_long', '2025-03-03T20:00:00.000Z', 108000000), ('not an avatar', '2025-03-02T20:00:00.000Z', 5);
                 CREATE TABLE cache_avatar (id TEXT, name TEXT, author_id TEXT, author_name TEXT, thumbnail_image_url TEXT, release_status TEXT);
                 INSERT INTO cache_avatar VALUES ('avtr_fox', 'Sunset Fox', 'usr_r', 'Rook', 'https://example.test/fox.png', 'public');
                 CREATE TABLE avatar_tags (avatar_id TEXT, tag TEXT, color TEXT);
                 INSERT INTO avatar_tags VALUES ('avtr_fox', 'dance', '#fff'), ('avtr_fox', '  ', '#fff');",
            )
            .unwrap();
        }
        let s = Store::in_memory().unwrap();
        let got = s.import_vrcx(&other, "usr_me-0000", false, None).unwrap();
        assert_eq!((got.avatars, got.tags), (3, 1));
        let worn = s.avatar_history(10).unwrap();
        let fox = worn.iter().find(|a| a.avatar_id == "avtr_fox").unwrap();
        assert_eq!((fox.name.as_str(), fox.author_name.as_str()), ("Sunset Fox", "Rook"));
        // One row at the moment it was last put on, with the total kept apart as imported time.
        let (start, end, kept): (String, String, i64) = s
            .db
            .query_row("SELECT worn_at, ended_at, imported_seconds FROM avatar_history WHERE avatar_id = 'avtr_fox'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap();
        assert_eq!((start.as_str(), end.as_str(), kept), ("2025-03-01T20:00:00.000Z", "2025-03-01T20:00:00.000Z", 5400));
        assert_eq!((fox.imported_seconds, fox.seconds, fox.last_worn.as_str()), (5400, 0, "2025-03-01T20:00:00.000Z"));
        let long = worn.iter().find(|a| a.avatar_id == "avtr_long").unwrap();
        assert_eq!((long.times, long.imported_seconds), (1, 108000));
        assert!(worn.iter().all(|a| !a.wearing), "nothing brought over is being worn now");
        // Imported time says nothing about when it was, so it claims no world stays.
        s.db.execute(
            "INSERT INTO visits (location, world_id, world_name, joined_at, left_at)
             VALUES ('wrld_x:1', 'wrld_x', 'Pier', '2025-03-03T08:00:00.000Z', '2025-03-03T21:00:00.000Z')",
            [],
        )
        .unwrap();
        let long = s.avatar_history(10).unwrap().into_iter().find(|a| a.avatar_id == "avtr_long").unwrap();
        assert_eq!((long.seconds_in_game, long.visits, long.last_in_game.as_str()), (0, 0, ""));
        // VRCX notes a later wearing by changing the same line; nothing is added for an avatar already here.
        {
            let old = Connection::open(&other).unwrap();
            old.execute("UPDATE usrme0000_avatar_history SET created_at = '2025-04-01T20:00:00.000Z', time = 9000000 WHERE avatar_id = 'avtr_fox'", []).unwrap();
        }
        assert_eq!(s.tags().unwrap().len(), 1);
        // A second run adds nothing, and the quiet run leaves tags alone.
        assert_eq!(s.import_vrcx(&other, "usr_me-0000", false, None).unwrap(), BroughtOver::default());
        let quiet = Store::in_memory().unwrap();
        let got = quiet.import_vrcx(&other, "usr_me-0000", true, None).unwrap();
        assert_eq!((got.avatars, got.tags), (3, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_avatar_on_now_stays_first_after_a_vrcx_import() {
        let dir = std::env::temp_dir().join(format!("anonymous-vrx-wearing-first-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let other = dir.join("other.sqlite3");
        {
            let old = Connection::open(&other).unwrap();
            old.execute_batch(
                "CREATE TABLE usrme0000_avatar_history (avatar_id TEXT PRIMARY KEY, created_at TEXT, time INTEGER);
                 INSERT INTO usrme0000_avatar_history VALUES ('avtr_old', '2024-05-01T20:00:00.000Z', 3600000), ('avtr_newer', '2026-01-01T09:30:00.000Z', 3600000);",
            )
            .unwrap();
        }
        let s = Store::in_memory().unwrap();
        assert!(s.wear_avatar("avtr_before", "2025-12-31T10:00:00.000Z").unwrap());
        assert!(s.wear_avatar("avtr_now", "2026-01-01T08:00:00.000Z").unwrap());
        assert_eq!(s.import_vrcx(&other, "usr_me-0000", true, None).unwrap().avatars, 2);
        let list = s.avatar_history(10).unwrap();
        let order: Vec<&str> = list.iter().map(|a| a.avatar_id.as_str()).collect();
        // The one on now leads, then the rest by when each was last put on, not by when it was written.
        assert_eq!(order, ["avtr_now", "avtr_newer", "avtr_before", "avtr_old"]);
        assert_eq!(list.iter().filter(|a| a.wearing).count(), 1);
        assert!(list[0].wearing);
        // Asking for fewer still keeps the one on now.
        let two: Vec<String> = s.avatar_history(2).unwrap().into_iter().map(|a| a.avatar_id).collect();
        assert_eq!(two, ["avtr_now", "avtr_newer"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn older_vrcx_avatar_rows_fold_into_imported_time() {
        let db = Connection::open_in_memory().unwrap();
        // The table as it was before imported time had its own column.
        db.execute_batch(
            "CREATE TABLE avatar_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT, avatar_id TEXT NOT NULL, name TEXT NOT NULL DEFAULT '',
                author_id TEXT NOT NULL DEFAULT '', author_name TEXT NOT NULL DEFAULT '', image TEXT NOT NULL DEFAULT '',
                release_status TEXT NOT NULL DEFAULT '', worn_at TEXT NOT NULL, ended_at TEXT);
             INSERT INTO avatar_history (avatar_id, name, worn_at, ended_at) VALUES
                ('avtr_a', 'A', '2026-01-01T10:00:00.000Z', '2026-01-01T11:00:00.000Z'),
                ('avtr_b', 'B', '2026-01-01T11:00:00.000Z', '2026-01-01T12:00:00.000Z'),
                ('avtr_a', 'A', '2026-01-01T12:00:00.000Z', NULL),
                ('avtr_fox', 'Fox', '2025-03-01T08:00:00.000Z', '2025-03-01T20:00:00.000Z'),
                ('avtr_fox', 'Fox', '2025-02-28T20:00:00.000Z', '2025-03-01T08:00:00.000Z'),
                ('avtr_fox', 'Fox', '2025-02-28T14:00:00.000Z', '2025-02-28T20:00:00.000Z'),
                ('avtr_owl', 'Owl', '2025-03-02T20:00:00.000Z', '2025-03-02T20:00:00.000Z');",
        )
        .unwrap();
        upgrade_avatar_history(&db).unwrap();
        // A second open finds the column and changes nothing.
        upgrade_avatar_history(&db).unwrap();
        let rows = |id: &str| -> Vec<(String, Option<String>, Option<i64>)> {
            let mut stmt = db.prepare("SELECT worn_at, ended_at, imported_seconds FROM avatar_history WHERE avatar_id = ?1 ORDER BY id").unwrap();
            let found = stmt.query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(Result::unwrap).collect();
            found
        };
        // Wearings this app recorded stay as they were.
        assert_eq!(rows("avtr_a").len(), 2);
        assert!(rows("avtr_a").iter().all(|r| r.2.is_none()));
        assert_eq!(rows("avtr_b"), [("2026-01-01T11:00:00.000Z".to_string(), Some("2026-01-01T12:00:00.000Z".to_string()), None)]);
        // The made-up pieces become one row with their total.
        let last = "2025-03-01T20:00:00.000Z".to_string();
        assert_eq!(rows("avtr_fox"), [(last.clone(), Some(last), Some(30 * 3600))]);
        assert_eq!(rows("avtr_owl")[0].2, Some(0));
        let name: String = db.query_row("SELECT name FROM avatar_history WHERE avatar_id = 'avtr_fox'", [], |r| r.get(0)).unwrap();
        assert_eq!(name, "Fox");
    }

    #[test]
    fn an_import_newer_than_everything_keeps_the_ids_already_given() {
        let dir = std::env::temp_dir().join(format!("anonymous-vrx-append-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let other = dir.join("other.sqlite3");
        {
            let old = Connection::open(&other).unwrap();
            old.execute_batch(
                "CREATE TABLE usrme0000_feed_gps (id INTEGER PRIMARY KEY, created_at TEXT, user_id TEXT, display_name TEXT, location TEXT, world_name TEXT, previous_location TEXT, time INTEGER, group_name TEXT);
                 INSERT INTO usrme0000_feed_gps (created_at, user_id, display_name, location, previous_location) VALUES ('2025-01-02T10:00:00.000Z', 'usr_a', 'Ana', 'wrld_x:1', 'private');
                 CREATE TABLE usrme0000_feed_online_offline (id INTEGER PRIMARY KEY, created_at TEXT, user_id TEXT, display_name TEXT, type TEXT, location TEXT, world_name TEXT, time INTEGER, group_name TEXT);
                 INSERT INTO usrme0000_feed_online_offline (created_at, user_id, display_name, type, location) VALUES ('2025-01-02T09:00:00.000Z', 'usr_a', 'Ana', 'Online', 'private');
                 CREATE TABLE gamelog_location (id INTEGER PRIMARY KEY, created_at TEXT, location TEXT, world_id TEXT, world_name TEXT, time INTEGER, group_name TEXT);
                 INSERT INTO gamelog_location (created_at, location, world_id, world_name, time) VALUES ('2025-01-02T12:00:00.000Z', 'wrld_x:1', 'wrld_x', 'Pier', 3600000);",
            )
            .unwrap();
        }
        let s = Store::in_memory().unwrap();
        s.db.execute_batch(
            "INSERT INTO feed (at, user_id, display_name, kind) VALUES ('2024-06-01T10:00:00.000Z', 'usr_b', 'Bo', 'online');
             INSERT INTO visits (location, world_id, world_name, joined_at, left_at) VALUES ('wrld_y:1', 'wrld_y', 'Hall', '2024-06-01T10:00:00.000Z', NULL);",
        )
        .unwrap();
        let got = s.import_vrcx(&other, "usr_me-0000", true, None).unwrap();
        assert_eq!((got.feed, got.visits), (2, 1));
        let bo: i64 = s.db.query_row("SELECT id FROM feed WHERE user_id = 'usr_b'", [], |r| r.get(0)).unwrap();
        assert_eq!(bo, 1, "nothing was numbered again");
        let hall: i64 = s.db.query_row("SELECT id FROM visits WHERE world_id = 'wrld_y'", [], |r| r.get(0)).unwrap();
        assert_eq!(hall, 1, "the stay already here keeps its number");
        // The new entries still go in by time, though they came from two tables.
        let feed = s.feed(50, None).unwrap();
        assert!(feed.windows(2).all(|w| w[0].at >= w[1].at && w[0].id > w[1].id));
        assert_eq!(feed[0].kind, "location");
        assert_eq!(s.setting(RENUMBER_FEED).unwrap(), None, "a list left in order loses its mark");
        assert_eq!(s.setting(RENUMBER_VISITS).unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_import_cut_off_before_the_renumber_is_put_in_order_later() {
        let dir = std::env::temp_dir().join(format!("anonymous-vrx-cut-off-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let other = dir.join("other.sqlite3");
        {
            let old = Connection::open(&other).unwrap();
            old.execute_batch(
                "CREATE TABLE usrme0000_feed_gps (id INTEGER PRIMARY KEY, created_at TEXT, user_id TEXT, display_name TEXT, location TEXT, world_name TEXT, previous_location TEXT, time INTEGER, group_name TEXT);
                 INSERT INTO usrme0000_feed_gps (created_at, user_id, display_name, location, previous_location) VALUES ('2025-01-02T10:00:00.000Z', 'usr_a', 'Ana', 'wrld_x:1', 'private');
                 CREATE TABLE gamelog_location (id INTEGER PRIMARY KEY, created_at TEXT, location TEXT, world_id TEXT, world_name TEXT, time INTEGER, group_name TEXT);
                 INSERT INTO gamelog_location (created_at, location, world_id, world_name, time) VALUES ('2025-01-02T12:00:00.000Z', 'wrld_x:1', 'wrld_x', 'Pier', 3600000);
                 CREATE TABLE gamelog_join_leave (id INTEGER PRIMARY KEY, created_at TEXT, type TEXT, display_name TEXT, location TEXT, user_id TEXT, time INTEGER);
                 INSERT INTO gamelog_join_leave (created_at, type, display_name, location, user_id, time) VALUES ('2025-01-02T12:00:05.000Z', 'OnPlayerJoined', 'Ana', 'wrld_x:1', 'usr_a', 0);",
            )
            .unwrap();
        }
        // The job left undone is finished by the next import, and also by opening the history again.
        for reopen in [false, true] {
            let path = dir.join(format!("history-{reopen}.sqlite3"));
            let s = Store::open(&path).unwrap();
            // Rows the app recorded itself, newer than everything coming over.
            s.db.execute_batch(
                "INSERT INTO feed (at, user_id, display_name, kind) VALUES ('2026-06-01T10:00:00.000Z', 'usr_b', 'Bo', 'online');
                 INSERT INTO visits (location, world_id, world_name, joined_at, left_at) VALUES ('wrld_y:1', 'wrld_y', 'Hall', '2026-06-01T10:00:00.000Z', '2026-06-01T11:00:00.000Z');",
            )
            .unwrap();

            // The run stops after its pieces are written and before the renumber.
            STOP_BEFORE_RENUMBER.with(|stop| stop.set(true));
            let cut = s.import_vrcx(&other, "usr_me-0000", true, None);
            STOP_BEFORE_RENUMBER.with(|stop| stop.set(false));
            assert!(cut.is_err());
            // The older rows sit after the newer ones, and the marks that say so are on the file.
            assert!(out_of_order(&s.db, "feed", "at", 0).unwrap());
            assert!(out_of_order(&s.db, "visits", "joined_at", 0).unwrap());
            assert_eq!(s.setting(RENUMBER_FEED).unwrap().as_deref(), Some("1"));
            assert_eq!(s.setting(RENUMBER_VISITS).unwrap().as_deref(), Some("1"));

            let s = if reopen {
                drop(s);
                Store::open(&path).unwrap()
            } else {
                assert_eq!(s.import_vrcx(&other, "usr_me-0000", true, None).unwrap(), BroughtOver::default(), "nothing comes over twice");
                s
            };
            assert!(!out_of_order(&s.db, "feed", "at", 0).unwrap(), "the feed is in order again (reopened: {reopen})");
            assert!(!out_of_order(&s.db, "visits", "joined_at", 0).unwrap(), "the stays are in order again (reopened: {reopen})");
            assert_eq!(s.setting(RENUMBER_FEED).unwrap(), None);
            assert_eq!(s.setting(RENUMBER_VISITS).unwrap(), None);
            let feed = s.feed(10, None).unwrap();
            assert_eq!(feed.len(), 2);
            assert_eq!(feed[0].user_id, "usr_b", "the newest entry leads");
            assert!(feed.windows(2).all(|w| w[0].at >= w[1].at && w[0].id > w[1].id));
            let visits = s.visits(10, None).unwrap();
            assert_eq!(visits.iter().map(|v| v.world_name.as_str()).collect::<Vec<_>>(), ["Hall", "Pier"]);
            let pier = visits.iter().find(|v| v.world_name == "Pier").unwrap();
            let people = s.visit_people(pier.id).unwrap();
            assert_eq!(people.len(), 1, "the people seen still point at their stay");
            assert_eq!(people[0].display_name, "Ana");
            drop(s);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_from_vrcx_comes_over_once_and_in_order() {
        let dir = std::env::temp_dir().join(format!("anonymous-vrx-bring-over-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let other = dir.join("other.sqlite3");
        {
            let old = Connection::open(&other).unwrap();
            old.execute_batch(
                "CREATE TABLE usrme0000_feed_gps (id INTEGER PRIMARY KEY, created_at TEXT, user_id TEXT, display_name TEXT, location TEXT, world_name TEXT, previous_location TEXT, time INTEGER, group_name TEXT);
                 INSERT INTO usrme0000_feed_gps (created_at, user_id, display_name, location, previous_location) VALUES ('2025-01-02T10:00:00.000Z', 'usr_a', 'Ana', 'wrld_x:1', 'private');
                 CREATE TABLE usrme0000_feed_online_offline (id INTEGER PRIMARY KEY, created_at TEXT, user_id TEXT, display_name TEXT, type TEXT, location TEXT, world_name TEXT, time INTEGER, group_name TEXT);
                 INSERT INTO usrme0000_feed_online_offline (created_at, user_id, display_name, type, location) VALUES ('2025-01-02T09:00:00.000Z', 'usr_a', 'Ana', 'Online', 'private');
                 CREATE TABLE usrme0000_feed_status (id INTEGER PRIMARY KEY, created_at TEXT, user_id TEXT, display_name TEXT, status TEXT, status_description TEXT, previous_status TEXT, previous_status_description TEXT);
                 INSERT INTO usrme0000_feed_status (created_at, user_id, display_name, status, status_description, previous_status, previous_status_description) VALUES ('2025-01-02T11:00:00.000Z', 'usr_a', 'Ana', 'busy', 'afk', 'active', 'afk');
                 CREATE TABLE usrme0000_friend_log_history (id INTEGER PRIMARY KEY, created_at TEXT, type TEXT, user_id TEXT, display_name TEXT, previous_display_name TEXT, trust_level TEXT, previous_trust_level TEXT, friend_number INTEGER);
                 INSERT INTO usrme0000_friend_log_history (created_at, type, user_id, display_name, previous_display_name) VALUES ('2025-01-01T08:00:00.000Z', 'Friend', 'usr_a', 'Ana', ''), ('2025-01-03T08:00:00.000Z', 'DisplayName', 'usr_a', 'Anna', 'Ana'), ('2025-01-03T09:00:00.000Z', 'TrustLevel', 'usr_a', 'Anna', '');
                 CREATE TABLE usrother_feed_gps (id INTEGER PRIMARY KEY, created_at TEXT, user_id TEXT, display_name TEXT, location TEXT, world_name TEXT, previous_location TEXT, time INTEGER, group_name TEXT);
                 INSERT INTO usrother_feed_gps (created_at, user_id, display_name, location, previous_location) VALUES ('2025-01-02T10:00:00.000Z', 'usr_z', 'Not mine', 'wrld_y:1', '');
                 CREATE TABLE gamelog_location (id INTEGER PRIMARY KEY, created_at TEXT, location TEXT, world_id TEXT, world_name TEXT, time INTEGER, group_name TEXT);
                 INSERT INTO gamelog_location (created_at, location, world_id, world_name, time) VALUES ('2025-01-02T12:00:00.000Z', 'wrld_x:1', 'wrld_x', 'Pier', 3600000);
                 CREATE TABLE gamelog_join_leave (id INTEGER PRIMARY KEY, created_at TEXT, type TEXT, display_name TEXT, location TEXT, user_id TEXT, time INTEGER);
                 INSERT INTO gamelog_join_leave (created_at, type, display_name, location, user_id, time) VALUES
                    ('2025-01-02T12:00:05.000Z', 'OnPlayerJoined', 'Ana', 'wrld_x:1', 'usr_a', 0),
                    ('2025-01-02T12:30:00.000Z', 'OnPlayerLeft', 'Ana', 'wrld_x:1', 'usr_a', 1795000),
                    ('2025-01-02T12:10:00.000Z', 'OnPlayerJoined', 'Bo', 'wrld_x:1', '', 0),
                    ('2025-01-05T12:10:00.000Z', 'OnPlayerJoined', 'Later', 'wrld_x:1', '', 0);
                 CREATE TABLE memos (user_id TEXT PRIMARY KEY, edited_at TEXT, memo TEXT);
                 INSERT INTO memos VALUES ('usr_a', '2025-01-02T00:00:00.000Z', 'met at the pier'), ('usr_b', '2025-01-02T00:00:00.000Z', 'from the other app');
                 CREATE TABLE favorite_world (id INTEGER PRIMARY KEY, created_at TEXT, world_id TEXT, group_name TEXT);
                 INSERT INTO favorite_world (created_at, world_id, group_name) VALUES ('2025-01-02T00:00:00.000Z', 'wrld_x', 'Chill');
                 CREATE TABLE cache_world (id TEXT PRIMARY KEY, name TEXT, author_name TEXT, thumbnail_image_url TEXT);
                 INSERT INTO cache_world VALUES ('wrld_x', 'Pier', 'Rook', 'https://example.test/pier.png');",
            )
            .unwrap();
        }

        let s = Store::in_memory().unwrap();
        // Something this app recorded itself, newer than everything coming over.
        s.add_feed(&FeedDraft { user_id: "usr_a".into(), display_name: "Anna".into(), kind: "online", detail: String::new(), previous: String::new(), location: String::new() }).unwrap();
        s.set_note("usr_b", "my own note stays").unwrap();

        let got = s.import_vrcx(&other, "usr_me-0000", false, None).unwrap();
        assert_eq!(got, BroughtOver { feed: 3, friend_log: 2, visits: 1, people: 2, notes: 1, favourites: 1, ..BroughtOver::default() });
        // The status text did not change, so only the status itself came over; another account's table was not read.
        let feed = s.feed(50, None).unwrap();
        assert_eq!(feed.len(), 6);
        assert!(feed.iter().all(|f| f.user_id != "usr_z"));
        // Newest first: my own entry from today leads, the oldest import is last.
        assert_eq!((feed[0].kind.as_str(), feed[0].display_name.as_str()), ("online", "Anna"));
        assert_eq!(feed.last().unwrap().kind, "friendAdded");
        assert!(feed.windows(2).all(|w| w[0].at >= w[1].at));

        let visits = s.visits(10, None).unwrap();
        assert_eq!(visits.len(), 1);
        assert_eq!(visits[0].left_at.as_deref(), Some("2025-01-02T13:00:00.000Z"));
        let people = s.visit_people(visits[0].id).unwrap();
        assert_eq!(people.len(), 2, "someone who joined days later is not part of this stay");
        assert_eq!(people.iter().find(|p| p.display_name == "Ana").unwrap().left_at.as_deref(), Some("2025-01-02T12:30:00.000Z"));
        assert_eq!(people.iter().find(|p| p.display_name == "Bo").unwrap().left_at.as_deref(), Some("2025-01-02T13:00:00.000Z"));
        assert!(s.open_visit().unwrap().is_none(), "a stay from the past is never left open");
        assert_eq!(s.note("usr_a").unwrap(), "met at the pier");
        assert_eq!(s.note("usr_b").unwrap(), "my own note stays");

        // A second run finds nothing new.
        assert_eq!(s.import_vrcx(&other, "usr_me-0000", false, None).unwrap(), BroughtOver::default());
        assert_eq!(s.feed(50, None).unwrap().len(), 6);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Reads a real VRCX file into an empty history in memory and prints the counts. The file
    /// is named in `ANON_VRX_OTHER`; nothing on disk is changed.
    #[test]
    #[ignore]
    fn real_vrcx_file_comes_over() {
        let other = std::path::PathBuf::from(std::env::var("ANON_VRX_OTHER").expect("ANON_VRX_OTHER"));
        let account: String = {
            let old = Connection::open_with_flags(&other, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            let table: String = old.query_row("SELECT name FROM sqlite_master WHERE name LIKE 'usr%_feed_gps' LIMIT 1", [], |r| r.get(0)).unwrap();
            table.trim_end_matches("_feed_gps").to_string()
        };
        let s = Store::in_memory().unwrap();
        let got = s.import_vrcx(&other, &account, false, None).unwrap();
        println!("{got:?}");
        let feed = s.feed(1000, None).unwrap();
        assert_eq!(feed.len(), got.feed + got.friend_log);
        assert!(feed.windows(2).all(|w| w[0].at >= w[1].at), "newest first");
        assert_eq!(s.import_vrcx(&other, &account, false, None).unwrap(), BroughtOver::default(), "a second run adds nothing");
    }
}
