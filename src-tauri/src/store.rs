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
    ended_at       TEXT
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
";

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
    pub seconds: i64,
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
fn close_visits(db: &Connection, at: Option<&str>) -> rusqlite::Result<()> {
    match at {
        Some(at) => {
            db.execute(
                "UPDATE sightings SET left_at = ?1 WHERE left_at IS NULL
                 AND visit_id IN (SELECT id FROM visits WHERE left_at IS NULL)",
                params![at],
            )?;
            db.execute("UPDATE visits SET left_at = ?1 WHERE left_at IS NULL", params![at])?;
        }
        None => {
            db.execute(
                "UPDATE visits SET left_at = COALESCE(
                    (SELECT MAX(COALESCE(s.left_at, s.joined_at)) FROM sightings s WHERE s.visit_id = visits.id),
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
        let db = Connection::open(path)?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.execute_batch(SCHEMA)?;
        Ok(Store { db })
    }

    /// Opens a history file that must already exist, without changing it.
    pub fn open_existing(path: &Path) -> AppResult<Store> {
        let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        db.query_row("SELECT COUNT(*) FROM feed", [], |r| r.get::<_, i64>(0))?;
        Ok(Store { db })
    }

    #[cfg(test)]
    pub fn in_memory() -> AppResult<Store> {
        let db = Connection::open_in_memory()?;
        db.execute_batch(SCHEMA)?;
        Ok(Store { db })
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

    /// Time spent in the same instance as someone, from the game-log history.
    pub fn together(&self, user_id: &str) -> AppResult<crate::vrchat::people::Together> {
        let at = now();
        Ok(self.db.query_row(
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
        )?)
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
            let pattern = like_pattern(q);
            sql.push_str(" AND (display_name LIKE ? ESCAPE '\\' OR detail LIKE ? ESCAPE '\\' OR previous LIKE ? ESCAPE '\\'");
            args.extend([pattern.clone(), pattern.clone(), pattern.clone()]);
            // World names live in their own table; match those worlds by id.
            let mut stmt = self.db.prepare("SELECT world_id FROM worlds WHERE name LIKE ?1 ESCAPE '\\' LIMIT 40")?;
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

    /// Avatars I wore, most recent first, one row per avatar.
    pub fn avatar_history(&self, limit: u32) -> AppResult<Vec<AvatarWorn>> {
        let mut stmt = self.db.prepare(
            "SELECT avatar_id, MAX(name), MAX(author_id), MAX(author_name), MAX(image), MAX(release_status),
                    MAX(worn_at), COUNT(*),
                    CAST(SUM(MIN(43200, MAX(0,
                        (julianday(COALESCE(ended_at, ?1)) - julianday(worn_at)) * 86400))) AS INTEGER)
             FROM avatar_history GROUP BY avatar_id ORDER BY MAX(id) DESC LIMIT ?2",
        )?;
        // For each avatar, the seconds its wearing times overlap my stays in worlds. A stay or
        // a wearing that was never closed runs until now, and each overlap counts 12 hours at most.
        let mut in_game = std::collections::HashMap::new();
        {
            let mut overlap = self.db.prepare(
                "SELECT h.avatar_id,
                        CAST(ROUND(SUM(MIN(43200, MAX(0,
                            (MIN(julianday(COALESCE(h.ended_at, ?1)), julianday(COALESCE(v.left_at, ?1)))
                             - MAX(julianday(h.worn_at), julianday(v.joined_at))) * 86400)))) AS INTEGER)
                        , COUNT(DISTINCT v.id),
                        strftime('%Y-%m-%dT%H:%M:%SZ', MAX(MIN(julianday(COALESCE(h.ended_at, ?1)), julianday(COALESCE(v.left_at, ?1)))))
                 FROM avatar_history h
                 JOIN visits v
                   ON julianday(v.joined_at) < julianday(COALESCE(h.ended_at, ?1))
                  AND julianday(COALESCE(v.left_at, ?1)) > julianday(h.worn_at)
                 GROUP BY h.avatar_id",
            )?;
            let found = overlap.query_map(params![now()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    (r.get::<_, Option<i64>>(1)?.unwrap_or(0), r.get::<_, i64>(2)?, r.get::<_, Option<String>>(3)?.unwrap_or_default()),
                ))
            })?;
            for row in found {
                let (id, facts) = row?;
                in_game.insert(id, facts);
            }
        }
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
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
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
            // A new log file means the game was started again.
            close_visits(&tx, None)?;
        }
        let mut room_name = String::new();
        for (at, event) in events {
            match event {
                LogEvent::Room(name) => room_name = name.clone(),
                LogEvent::Joining(location) => {
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
                    close_visits(&tx, Some(at))?;
                    changed = true;
                }
                LogEvent::PlayerJoined { name, id } => {
                    let done = tx.execute(
                        "INSERT INTO sightings (visit_id, user_id, display_name, joined_at)
                         SELECT id, ?1, ?2, ?3 FROM visits WHERE left_at IS NULL ORDER BY id DESC LIMIT 1",
                        params![id, name, at],
                    )?;
                    changed |= done > 0;
                }
                LogEvent::PlayerLeft { name, id } => {
                    let done = tx.execute(
                        "UPDATE sightings SET left_at = ?1 WHERE left_at IS NULL
                         AND visit_id = (SELECT id FROM visits WHERE left_at IS NULL ORDER BY id DESC LIMIT 1)
                         AND ((?2 <> '' AND user_id = ?2) OR (?2 = '' AND display_name = ?3))",
                        params![at, id, name],
                    )?;
                    changed |= done > 0;
                }
            }
        }
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

    pub fn worlds(&self) -> AppResult<Vec<World>> {
        let mut stmt = self.db.prepare("SELECT world_id, name, image FROM worlds")?;
        let rows = stmt.query_map([], world_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
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
    pub fn import_vrcx(&self, other: &Path, my_id: &str, history_only: bool, since: Option<&str>) -> AppResult<BroughtOver> {
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
        let mut done = BroughtOver::default();
        let tx = self.db.unchecked_transaction()?;

        // One feed entry, unless this history already has it.
        let add = |at: &str, user: &str, name: &str, kind: &str, detail: &str, previous: &str, location: &str| -> AppResult<bool> {
            if at.is_empty() || user.is_empty() {
                return Ok(false);
            }
            let known: i64 = tx.query_row(
                "SELECT COUNT(*) FROM feed WHERE user_id = ?1 AND kind = ?2
                 AND ABS(CAST(strftime('%s', at) AS INTEGER) - CAST(strftime('%s', ?3) AS INTEGER)) <= 5",
                params![user, kind, at],
                |r| r.get(0),
            )?;
            if known > 0 {
                return Ok(false);
            }
            tx.execute(
                "INSERT INTO feed (at, user_id, display_name, kind, detail, previous, location) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![at, user, name, kind, detail, previous, location],
            )?;
            Ok(true)
        };

        let table = format!("{prefix}_feed_gps");
        if has(&table) {
            let mut rows = old.prepare(&format!("SELECT created_at, user_id, display_name, location, previous_location FROM \"{table}\" {from} ORDER BY created_at"))?;
            let mut it = rows.query([])?;
            while let Some(r) = it.next()? {
                if add(&text(r, 0), &text(r, 1), &text(r, 2), "location", "", &text(r, 4), &text(r, 3))? {
                    done.feed += 1;
                }
            }
        }
        let table = format!("{prefix}_feed_online_offline");
        if has(&table) {
            let mut rows = old.prepare(&format!("SELECT created_at, user_id, display_name, type, location FROM \"{table}\" {from} ORDER BY created_at"))?;
            let mut it = rows.query([])?;
            while let Some(r) = it.next()? {
                let kind = if text(r, 3).eq_ignore_ascii_case("online") { "online" } else { "offline" };
                if add(&text(r, 0), &text(r, 1), &text(r, 2), kind, "", "", &text(r, 4))? {
                    done.feed += 1;
                }
            }
        }
        let table = format!("{prefix}_feed_status");
        if has(&table) {
            let mut rows = old.prepare(&format!(
                "SELECT created_at, user_id, display_name, status, status_description, previous_status, previous_status_description FROM \"{table}\" {from} ORDER BY created_at"
            ))?;
            let mut it = rows.query([])?;
            while let Some(r) = it.next()? {
                let (at, user, name) = (text(r, 0), text(r, 1), text(r, 2));
                let (status, words, was, were) = (text(r, 3), text(r, 4), text(r, 5), text(r, 6));
                if !status.is_empty() && status != was && add(&at, &user, &name, "status", &status, &was, "")? {
                    done.feed += 1;
                }
                if words != were && add(&at, &user, &name, "statusText", &words, &were, "")? {
                    done.feed += 1;
                }
            }
        }
        let table = format!("{prefix}_feed_avatar");
        if has(&table) {
            let mut rows = old.prepare(&format!("SELECT created_at, user_id, display_name FROM \"{table}\" {from} ORDER BY created_at"))?;
            let mut it = rows.query([])?;
            while let Some(r) = it.next()? {
                if add(&text(r, 0), &text(r, 1), &text(r, 2), "avatar", "", "", "")? {
                    done.feed += 1;
                }
            }
        }
        let table = format!("{prefix}_friend_log_history");
        if has(&table) {
            let mut rows = old.prepare(&format!("SELECT created_at, type, user_id, display_name, previous_display_name FROM \"{table}\" {from} ORDER BY created_at"))?;
            let mut it = rows.query([])?;
            while let Some(r) = it.next()? {
                let (at, kind, user, name, was) = (text(r, 0), text(r, 1), text(r, 2), text(r, 3), text(r, 4));
                let added = match kind.as_str() {
                    "Friend" => add(&at, &user, &name, "friendAdded", "", "", "")?,
                    "Unfriend" => add(&at, &user, &name, "friendRemoved", "", "", "")?,
                    "DisplayName" => add(&at, &user, &name, "name", &name, &was, "")?,
                    _ => false,
                };
                if added {
                    done.friend_log += 1;
                }
            }
        }

        // Visits, each with the people the game log saw there.
        if has("gamelog_location") {
            let mut rows = old.prepare(&format!("SELECT created_at, location, world_id, world_name, time FROM gamelog_location {from} ORDER BY created_at"))?;
            let mut it = rows.query([])?;
            while let Some(r) = it.next()? {
                let (joined, location, world_id, world_name) = (text(r, 0), text(r, 1), text(r, 2), text(r, 3));
                let stayed_ms: i64 = r.get::<_, Option<i64>>(4).ok().flatten().unwrap_or(0).max(0);
                if joined.is_empty() || location.is_empty() {
                    continue;
                }
                let known: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM visits WHERE location = ?1
                     AND ABS(CAST(strftime('%s', joined_at) AS INTEGER) - CAST(strftime('%s', ?2) AS INTEGER)) <= 10",
                    params![location, joined],
                    |r| r.get(0),
                )?;
                if known > 0 {
                    continue;
                }
                // VRCX keeps how long the stay lasted; a stay it never closed ends where it began.
                let left: String = tx.query_row(
                    "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', ?1, ?2)",
                    params![joined, format!("+{:.3} seconds", stayed_ms as f64 / 1000.0)],
                    |r| r.get(0),
                )?;
                tx.execute(
                    "INSERT INTO visits (location, world_id, world_name, joined_at, left_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![location, world_id, world_name, joined, left],
                )?;
                let visit = tx.last_insert_rowid();
                done.visits += 1;
                if !has("gamelog_join_leave") {
                    continue;
                }
                let mut seen = old.prepare(
                    "SELECT created_at, type, display_name, user_id FROM gamelog_join_leave
                     WHERE location = ?1 AND created_at >= ?2 AND created_at <= ?3 ORDER BY created_at",
                )?;
                let mut people = seen.query(params![location, joined, left])?;
                while let Some(p) = people.next()? {
                    let (at, kind, name, user) = (text(p, 0), text(p, 1), text(p, 2), text(p, 3));
                    if name.is_empty() {
                        continue;
                    }
                    if kind == "OnPlayerJoined" {
                        tx.execute(
                            "INSERT INTO sightings (visit_id, user_id, display_name, joined_at) VALUES (?1, ?2, ?3, ?4)",
                            params![visit, user, name, at],
                        )?;
                        done.people += 1;
                    } else if kind == "OnPlayerLeft" {
                        tx.execute(
                            "UPDATE sightings SET left_at = ?1 WHERE id = (
                                SELECT id FROM sightings WHERE visit_id = ?2 AND left_at IS NULL AND display_name = ?3 ORDER BY id DESC LIMIT 1)",
                            params![at, visit, name],
                        )?;
                    }
                }
                tx.execute("UPDATE sightings SET left_at = ?1 WHERE visit_id = ?2 AND left_at IS NULL", params![left, visit])?;
            }
        }

        // The avatars I wore. VRCX keeps the time it noted and how long the avatar was on; the
        // name, maker and picture come from what it remembered about the avatar.
        let worn = format!("{prefix}_avatar_history");
        if has(&worn) {
            let known = has("cache_avatar");
            let mut rows = old.prepare(&format!("SELECT avatar_id, created_at, time FROM \"{worn}\" {from} ORDER BY created_at"))?;
            let mut it = rows.query([])?;
            while let Some(r) = it.next()? {
                let (id, at) = (text(r, 0), text(r, 1));
                let millis = r.get::<_, Option<i64>>(2).ok().flatten().unwrap_or(0).max(0);
                if !id.starts_with("avtr_") || at.is_empty() {
                    continue;
                }
                let already: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM avatar_history WHERE avatar_id = ?1 AND ABS(strftime('%s', worn_at) - strftime('%s', ?2)) <= 10",
                    params![id, at],
                    |c| c.get(0),
                )?;
                if already > 0 {
                    continue;
                }
                let ended = chrono::DateTime::parse_from_rfc3339(&at)
                    .ok()
                    .map(|t| (t + chrono::Duration::milliseconds(millis)).with_timezone(&chrono::Utc).format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
                    .unwrap_or_else(|| at.clone());
                let (name, author_id, author, image, status) = if known {
                    old.query_row("SELECT name, author_id, author_name, thumbnail_image_url, release_status FROM cache_avatar WHERE id = ?1", params![id], |c| {
                        let get = |i: usize| c.get::<_, Option<String>>(i).map(Option::unwrap_or_default);
                        Ok((get(0)?, get(1)?, get(2)?, get(3)?, get(4)?))
                    })
                    .unwrap_or_default()
                } else {
                    Default::default()
                };
                done.avatars += tx.execute(
                    "INSERT INTO avatar_history (avatar_id, name, author_id, author_name, image, release_status, worn_at, ended_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![id, name, author_id, author, image, status, at, ended],
                )?;
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

        // Older entries were just added with the newest ids. Both lists are read newest id
        // first, so they are numbered again in order of time. People keep pointing at their visit.
        if done.feed + done.friend_log > 0 {
            tx.execute_batch(
                "CREATE TABLE feed_sorted AS SELECT at, user_id, display_name, kind, detail, previous, location FROM feed ORDER BY at, id;
                 DELETE FROM feed;
                 DELETE FROM sqlite_sequence WHERE name = 'feed';
                 INSERT INTO feed (at, user_id, display_name, kind, detail, previous, location)
                     SELECT at, user_id, display_name, kind, detail, previous, location FROM feed_sorted ORDER BY rowid;
                 DROP TABLE feed_sorted;",
            )?;
        }
        if done.visits > 0 {
            tx.execute_batch(
                "CREATE TABLE visit_order AS SELECT id AS was, ROW_NUMBER() OVER (ORDER BY joined_at, id) AS now FROM visits;
                 CREATE TABLE visits_sorted AS SELECT o.now AS id, v.location, v.world_id, v.world_name, v.joined_at, v.left_at
                     FROM visits v JOIN visit_order o ON o.was = v.id;
                 UPDATE sightings SET visit_id = (SELECT now FROM visit_order WHERE was = sightings.visit_id)
                     WHERE visit_id IN (SELECT was FROM visit_order);
                 DELETE FROM visits;
                 DELETE FROM sqlite_sequence WHERE name = 'visits';
                 INSERT INTO visits (id, location, world_id, world_name, joined_at, left_at)
                     SELECT id, location, world_id, world_name, joined_at, left_at FROM visits_sorted ORDER BY id;
                 DROP TABLE visits_sorted;
                 DROP TABLE visit_order;",
            )?;
        }
        tx.commit()?;
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
                "CREATE TABLE usrme0000_avatar_history (avatar_id TEXT, created_at TEXT, time INTEGER);
                 INSERT INTO usrme0000_avatar_history VALUES ('avtr_fox', '2025-03-01T20:00:00.000Z', 5400000), ('avtr_owl', '2025-03-02T20:00:00.000Z', NULL), ('not an avatar', '2025-03-02T20:00:00.000Z', 5);
                 CREATE TABLE cache_avatar (id TEXT, name TEXT, author_id TEXT, author_name TEXT, thumbnail_image_url TEXT, release_status TEXT);
                 INSERT INTO cache_avatar VALUES ('avtr_fox', 'Sunset Fox', 'usr_r', 'Rook', 'https://example.test/fox.png', 'public');
                 CREATE TABLE avatar_tags (avatar_id TEXT, tag TEXT, color TEXT);
                 INSERT INTO avatar_tags VALUES ('avtr_fox', 'dance', '#fff'), ('avtr_fox', '  ', '#fff');",
            )
            .unwrap();
        }
        let s = Store::in_memory().unwrap();
        let got = s.import_vrcx(&other, "usr_me-0000", false, None).unwrap();
        assert_eq!((got.avatars, got.tags), (2, 1));
        let worn = s.avatar_history(10).unwrap();
        let fox = worn.iter().find(|a| a.avatar_id == "avtr_fox").unwrap();
        assert_eq!((fox.name.as_str(), fox.author_name.as_str()), ("Sunset Fox", "Rook"));
        let (start, end): (String, String) = s.db.query_row("SELECT worn_at, ended_at FROM avatar_history WHERE avatar_id = 'avtr_fox'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((start.as_str(), end.as_str()), ("2025-03-01T20:00:00.000Z", "2025-03-01T21:30:00.000Z"));
        assert_eq!(s.tags().unwrap().len(), 1);
        // A second run adds nothing, and the quiet run leaves tags alone.
        assert_eq!(s.import_vrcx(&other, "usr_me-0000", false, None).unwrap(), BroughtOver::default());
        let quiet = Store::in_memory().unwrap();
        let got = quiet.import_vrcx(&other, "usr_me-0000", true, None).unwrap();
        assert_eq!((got.avatars, got.tags), (2, 0));
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
