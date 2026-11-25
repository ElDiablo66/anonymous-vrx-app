//! Local history in SQLite: cached friends, the activity feed and world names.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::AppResult;
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
}
