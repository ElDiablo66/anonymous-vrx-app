//! Holds the signed-in session: friends in memory, the history database,
//! the live websocket and the queue that looks up world names.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

use crate::error::{AppError, AppResult};
use crate::secrets;
use crate::backup::{self, BackupInfo};
use crate::gamelog;
use crate::store::{AvatarWorn, LocalFavorite, LostAvatar, Sighting, Store, Visit};
use crate::vrchat::client::{user_agent, ApiClient, PIPELINE_URL};
use crate::vrchat::events::{self, Effect};
use crate::vrchat::model::{now_utc, parse_location, Favorites, FeedDraft, FeedItem, Friend, Instance, Me, World};
use crate::vrchat::notices::{self, Handled, Notice};
use crate::vrchat::people::{AvatarCard, Person, Together, WorldCard};

pub type Emit = Arc<dyn Fn(&str, Value) + Send + Sync>;

/// A full friend list is fetched at most this often after the first one.
const RESYNC_AFTER: Duration = Duration::from_secs(300);
/// Pause between two world-name lookups.
const WORLD_GAP: Duration = Duration::from_millis(300);
/// How long a head count is trusted before it is asked for again.
const COUNT_FRESH: Duration = Duration::from_secs(300);
/// How long to leave an instance alone after VRChat refused to describe it.
const COUNT_RETRY: Duration = Duration::from_secs(1800);
/// How often occupied instances are checked for a stale head count.
const COUNT_EVERY: Duration = Duration::from_secs(60);

/// Slow background lookups, done one at a time.
enum Job {
    World(String),
    Instance(String),
    /// Look up an avatar I switched to, for the avatar history.
    Avatar(String),
}
const QUIET_LIMIT: Duration = Duration::from_secs(150);
/// How often the app pings the live connection, so a dead one is noticed within minutes.
const PING_EVERY: Duration = Duration::from_secs(45);
/// How often the game log folder is checked for new lines.
const LOG_EVERY: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Inner {
    me: Option<Me>,
    store: Option<Store>,
    friends: HashMap<String, Friend>,
    worlds: HashMap<String, World>,
    favorites: Favorites,
    notices: Vec<Notice>,
    wanted: HashSet<String>,
    instances: HashMap<String, (Instance, Instant)>,
    refused: HashMap<String, Instant>,
    pipeline: &'static str,
    last_sync: Option<Instant>,
    world_tx: Option<mpsc::UnboundedSender<Job>>,
    tasks: Vec<JoinHandle<()>>,
}

pub struct Hub {
    pub api: Arc<ApiClient>,
    emit: Emit,
    data_dir: PathBuf,
    /// Off in tests so they never touch the real credential store.
    use_secrets: bool,
    /// Folder with VRChat's output logs. `None` turns game-log reading off.
    log_dir: Option<PathBuf>,
    inner: Mutex<Inner>,
    alerts: Mutex<Alerts>,
    /// The last favourites answer per kind, so screens can show at once and refresh after.
    fav_cache: Mutex<HashMap<String, Value>>,
    /// The people I blocked or muted, with the account the list belongs to.
    blocked: Mutex<Option<(String, Value)>>,
    /// Picture, rank and status of people looked up for a list, kept while the app runs.
    briefs: Mutex<HashMap<String, (Instant, Value)>>,
    /// Calendar pages already asked for, by list and month, with when.
    calendars: Mutex<HashMap<String, (Instant, Value)>>,
    /// Put my own world, avatar and status changes into the feed too.
    own_feed: std::sync::atomic::AtomicBool,
}

/// Which notifications get a Windows pop-up. The window sends its switches here,
/// so pop-ups still appear while the window is hidden in the tray.
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Alerts {
    pub invites: bool,
    pub friend_requests: bool,
    pub boops: bool,
    pub everything_else: bool,
}

impl Default for Alerts {
    fn default() -> Self {
        Alerts {
            invites: true,
            friend_requests: true,
            boops: true,
            everything_else: true,
        }
    }
}

/// Everything the profile panel shows about one person.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub person: Person,
    /// My own note, kept on this PC.
    pub note: String,
    /// The profile exactly as VRChat sent it.
    pub raw: Value,
    pub together: Together,
    /// Their recent entries in my feed.
    pub feed: Vec<FeedItem>,
    /// Names this PC saw them use, each with when it was last seen.
    pub past_names: Vec<(String, String)>,
}

enum Ended {
    Dropped,
    SessionOver,
}

/// Makes a value safe to put in a request address.
fn utf8(text: &str) -> String {
    percent_encoding::utf8_percent_encode(text, percent_encoding::NON_ALPHANUMERIC)
        .to_string()
        .replace("%5F", "_")
        .replace("%2D", "-")
}

fn safe_file_part(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect()
}

impl Hub {
    pub fn new(
        api: Arc<ApiClient>,
        data_dir: PathBuf,
        emit: Emit,
        use_secrets: bool,
        log_dir: Option<PathBuf>,
    ) -> Arc<Hub> {
        Arc::new(Hub {
            api,
            emit,
            data_dir,
            use_secrets,
            log_dir,
            inner: Mutex::new(Inner {
                pipeline: "off",
                ..Default::default()
            }),
            alerts: Mutex::new(Alerts::default()),
            fav_cache: Mutex::new(HashMap::new()),
            blocked: Mutex::new(None),
            briefs: Mutex::new(HashMap::new()),
            calendars: Mutex::new(HashMap::new()),
            own_feed: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn with_store<T>(&self, job: impl FnOnce(&Store) -> AppResult<T>) -> AppResult<T> {
        let inner = self.inner.lock().unwrap();
        match &inner.store {
            Some(store) => job(store),
            None => Err(AppError::NotSignedIn),
        }
    }

    /// Notes that I now wear this avatar and asks VRChat for its details.
    /// Whether my own changes go into the feed. Off unless the window says otherwise.
    pub fn set_own_feed(&self, on: bool) {
        self.own_feed.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// Writes one of my own changes into the feed, when that is switched on.
    fn own_entry(&self, kind: &'static str, detail: &str, previous: &str, location: &str) {
        if !self.own_feed.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let Some(me) = self.me() else { return };
        let draft = FeedDraft {
            user_id: me.id,
            display_name: "You".into(),
            kind,
            detail: detail.to_string(),
            previous: previous.to_string(),
            location: location.to_string(),
        };
        if let Ok(item) = self.with_store(|s| s.add_feed(&draft)) {
            self.send("feed:new", json!(item));
        }
    }

    /// My status or status text changed, as VRChat reports it for my own account.
    fn own_status_changed(&self, user: &Value) {
        let (mut status, mut text) = (None, None);
        {
            let mut inner = self.inner.lock().unwrap();
            let Some(me) = inner.me.as_mut() else { return };
            if let Some(now) = user.get("status").and_then(Value::as_str) {
                if !now.is_empty() && now != me.status {
                    status = Some((now.to_string(), std::mem::replace(&mut me.status, now.to_string())));
                }
            }
            if let Some(now) = user.get("statusDescription").and_then(Value::as_str) {
                if now != me.status_description {
                    text = Some((now.to_string(), std::mem::replace(&mut me.status_description, now.to_string())));
                }
            }
        }
        if let Some((now, before)) = status {
            self.own_entry("status", &now, &before, "");
        }
        if let Some((now, before)) = text {
            self.own_entry("statusText", &now, &before, "");
        }
    }

    pub fn last_seen(&self) -> AppResult<HashMap<String, String>> {
        self.with_store(|s| Ok(s.last_seen()?.into_iter().collect()))
    }

    pub fn ratings(&self) -> AppResult<HashMap<String, i64>> {
        self.with_store(|s| Ok(s.ratings()?.into_iter().collect()))
    }

    /// My stars for a world or an avatar, 0 to take them away.
    pub fn set_rating(&self, item_id: &str, stars: i64) -> AppResult<()> {
        if !(item_id.starts_with("wrld_") || item_id.starts_with("wld_") || item_id.starts_with("avtr_")) || !(0..=5).contains(&stars) {
            return Err(AppError::CannotDo("Stars go from 1 to 5, on a world or an avatar.".into()));
        }
        self.with_store(|s| s.set_rating(item_id, stars, &now_utc()))
    }

    /// Item id to the tags I gave it.
    pub fn tags(&self) -> AppResult<HashMap<String, Vec<String>>> {
        self.with_store(|s| {
            let mut out: HashMap<String, Vec<String>> = HashMap::new();
            for (id, tag) in s.tags()? {
                out.entry(id).or_default().push(tag);
            }
            Ok(out)
        })
    }

    /// Sets my tags on an avatar or a world. Tags are trimmed, at most 24 characters each and
    /// 12 per item, and the same word in different capitals counts once.
    pub fn set_tags(&self, item_id: &str, tags: &[String]) -> AppResult<Vec<String>> {
        if !(item_id.starts_with("avtr_") || item_id.starts_with("wrld_") || item_id.starts_with("wld_")) {
            return Err(AppError::CannotDo("Tags go on an avatar or a world.".into()));
        }
        let mut clean: Vec<String> = Vec::new();
        for tag in tags {
            let tag: String = tag.split_whitespace().collect::<Vec<_>>().join(" ");
            if tag.is_empty() || clean.iter().any(|t| t.to_lowercase() == tag.to_lowercase()) {
                continue;
            }
            if tag.chars().count() > 24 {
                return Err(AppError::CannotDo("A tag can be at most 24 characters.".into()));
            }
            clean.push(tag);
        }
        if clean.len() > 12 {
            return Err(AppError::CannotDo("At most 12 tags on one thing.".into()));
        }
        self.with_store(|s| s.set_tags(item_id, &clean))?;
        Ok(clean)
    }

    /// World id to how often I was there and when last.
    pub fn world_visits(&self) -> AppResult<HashMap<String, (i64, String)>> {
        self.with_store(|s| Ok(s.world_visits()?.into_iter().map(|(id, n, at)| (id, (n, at))).collect()))
    }

    fn avatar_changed(&self, avatar_id: &str) {
        if !avatar_id.starts_with("avtr_") {
            return;
        }
        if let Ok(true) = self.with_store(|s| s.wear_avatar(avatar_id, &now_utc())) {
            self.own_entry("avatar", "", "", "");
            if let Some(tx) = &self.inner.lock().unwrap().world_tx {
                let _ = tx.send(Job::Avatar(avatar_id.to_string()));
            }
            self.send("avatars:changed", Value::Null);
        }
    }

    /// My own note on anything: a person, an avatar or a world.
    pub fn note(&self, id: &str) -> AppResult<String> {
        self.with_store(|s| s.note(id))
    }

    /// Avatars of mine this PC saw on my account before that are not on it now.
    pub fn lost_avatars(&self) -> AppResult<Vec<LostAvatar>> {
        self.with_store(|s| s.lost_avatars())
    }

    pub fn avatar_history(&self, limit: u32) -> AppResult<Vec<AvatarWorn>> {
        self.with_store(|s| s.avatar_history(limit))
    }

    pub fn feed_search(
        &self,
        query: &str,
        kinds: &[String],
        before: Option<i64>,
        until: Option<&str>,
        limit: u32,
    ) -> AppResult<Vec<FeedItem>> {
        self.with_store(|s| s.feed_search(query, kinds, before, until, limit))
    }

    pub fn together_counts(&self) -> AppResult<HashMap<String, i64>> {
        self.with_store(|s| Ok(s.together_counts()?.into_iter().collect()))
    }

    pub fn local_favorites(&self, kind: &str) -> AppResult<Vec<LocalFavorite>> {
        self.with_store(|s| s.local_favorites(kind))
    }

    pub fn add_local_favorite(&self, f: &LocalFavorite) -> AppResult<()> {
        let fits = match f.kind.as_str() {
            "avatar" => f.item_id.starts_with("avtr_"),
            "world" => f.item_id.starts_with("wrld_") || f.item_id.starts_with("wld_"),
            _ => false,
        };
        if !fits {
            return Err(AppError::CannotDo("Only avatars and worlds can be kept on this PC.".into()));
        }
        self.with_store(|s| s.add_local_favorite(f))
    }

    pub fn remove_local_favorite(&self, kind: &str, item_id: &str) -> AppResult<()> {
        self.with_store(|s| s.remove_local_favorite(kind, item_id))
    }

    /// Sets my VRChat status (`join me`, `active`, `ask me`, `busy`) and status text.
    pub async fn set_status(&self, status: &str, text: &str) -> AppResult<()> {
        if !matches!(status, "join me" | "active" | "ask me" | "busy") {
            return Err(AppError::CannotDo("That is not a status VRChat knows.".into()));
        }
        if text.chars().count() > 32 {
            return Err(AppError::CannotDo("Status text can be at most 32 characters.".into()));
        }
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        self.api
            .update_me(&me.id, json!({ "status": status, "statusDescription": text }))
            .await?;
        // VRChat's own update for my account may land before or after this line. Whichever
        // comes first finds the old value and writes the feed entry; the other finds nothing changed.
        let (mut was_status, mut was_text) = (None, None);
        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(m) = inner.me.as_mut() {
                if m.status != status {
                    was_status = Some(std::mem::replace(&mut m.status, status.to_string()));
                }
                if m.status_description != text {
                    was_text = Some(std::mem::replace(&mut m.status_description, text.to_string()));
                }
            }
        }
        if let Some(before) = was_status {
            self.own_entry("status", status, &before, "");
        }
        if let Some(before) = was_text {
            self.own_entry("statusText", text, &before, "");
        }
        Ok(())
    }

    /// VRChat's own word on whether its servers are fine.
    pub async fn service_status(&self) -> AppResult<Value> {
        let page = self.api.service_status().await?;
        Ok(json!({
            "indicator": page.pointer("/status/indicator").and_then(Value::as_str).unwrap_or("none"),
            "description": page.pointer("/status/description").and_then(Value::as_str).unwrap_or(""),
        }))
    }

    /// Sends an event to the window. For work that lives outside this file.
    pub fn emit_event(&self, name: &str, payload: Value) {
        self.send(name, payload);
    }

    pub fn set_alerts(&self, alerts: Alerts) {
        *self.alerts.lock().unwrap() = alerts;
    }

    /// Tells the window about a new notification and asks for a Windows pop-up when it is switched on.
    fn announce(&self, notice: &Notice) {
        self.send("notice:new", json!(notice));
        let mine = self.me().is_some_and(|me| me.id == notice.sender_id);
        let alerts = *self.alerts.lock().unwrap();
        let wanted = match notices::alert_group(notice) {
            "friendRequests" => alerts.friend_requests,
            "invites" => alerts.invites,
            "boops" => alerts.boops,
            _ => alerts.everything_else,
        };
        if wanted && !mine {
            let (title, body) = notices::popup_text(notice);
            self.send("popup", json!({ "title": title, "body": body }));
        }
    }

    /// Keeps notifications VRChat cleared on its own side in the Done list, so none vanish unseen.
    fn log_cleared(&self, cleared: &[Notice]) {
        if cleared.is_empty() {
            return;
        }
        let inner = self.inner.lock().unwrap();
        if let Some(store) = &inner.store {
            for notice in cleared {
                let _ = store.log_handled(notice, "elsewhere", "");
            }
        }
    }

    fn history_path(&self, user_id: &str) -> PathBuf {
        self.data_dir
            .join(format!("history {}.sqlite", safe_file_part(user_id)))
    }

    /// Copies the history and the given window settings into a new folder under `root`.
    pub fn backup(&self, root: &std::path::Path, settings_json: &str, label: &str) -> AppResult<BackupInfo> {
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        // The copy is made through a second, read-only door to the same file, so the rest of
        // the app keeps working while a large history is copied.
        let copy = Store::open_existing(&self.history_path(&me.id))?;
        backup::create(root, &copy, settings_json, label)
    }

    /// Makes an automatic backup when the last one is older than a day, then keeps only the
    /// newest `keep`. Gives back the new backup, or nothing when none was due.
    pub fn auto_backup(&self, root: &std::path::Path, settings_json: &str, keep: usize) -> AppResult<Option<BackupInfo>> {
        if !backup::auto_due(root, std::time::Duration::from_secs(24 * 3600)) {
            return Ok(None);
        }
        let made = self.backup(root, settings_json, backup::AUTO_LABEL)?;
        backup::prune_auto(root, keep.clamp(1, 60));
        Ok(Some(made))
    }

    /// Saves the current data first, then lines the chosen backup up for the next start.
    /// Returns the backup's window settings.
    pub fn restore_backup(&self, root: &std::path::Path, name: &str, current_settings: &str) -> AppResult<String> {
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        self.backup(root, current_settings, "before restore")?;
        backup::stage_restore(root, name, &self.history_path(&me.id))
    }

    fn send(&self, name: &str, payload: Value) {
        (self.emit)(name, payload);
    }

    pub fn me(&self) -> Option<Me> {
        self.inner.lock().unwrap().me.clone()
    }

    pub fn friends(&self) -> Vec<Friend> {
        self.inner.lock().unwrap().friends.values().cloned().collect()
    }

    pub fn worlds(&self) -> Vec<World> {
        self.inner.lock().unwrap().worlds.values().cloned().collect()
    }

    pub fn favorites(&self) -> Favorites {
        self.inner.lock().unwrap().favorites.clone()
    }

    /// Fetches the favourite friends and their groups. Two or three requests.
    pub async fn sync_favorites(&self) -> AppResult<()> {
        let list = self.api.favorite_friends().await?;
        let groups = if list.is_empty() {
            Vec::new()
        } else {
            self.api.favorite_groups().await?
        };
        let favorites = Favorites::from_api(&list, &groups);
        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(store) = &inner.store {
                let _ = store.set_setting(
                    "favorites",
                    &serde_json::to_string(&favorites).unwrap_or_default(),
                );
            }
            inner.favorites = favorites;
        }
        self.send("favorites:reset", Value::Null);
        Ok(())
    }

    /// One person's profile from VRChat, with what this PC knows about them.
    pub async fn profile(&self, user_id: &str) -> AppResult<Profile> {
        // My own account has more to show (my settings, my avatar), and only I can read it.
        let mine = self.me().is_some_and(|me| me.id == user_id);
        let profile_path = format!("/profile/{}", utf8(user_id));
        let (user, public, own) = tokio::join!(self.api.user(user_id), self.api.read(&profile_path), async {
            if mine {
                self.api.current_user().await.ok()
            } else {
                None
            }
        });
        let mut json = user?;
        if mine {
            if let (Some(Value::Object(own)), Some(target)) = (own, json.as_object_mut()) {
                for (k, v) in own {
                    // The id lists of all my friends are long and belong on no profile.
                    if !matches!(k.as_str(), "friends" | "onlineFriends" | "activeFriends" | "offlineFriends" | "friendGroupNames") {
                        target.entry(k).or_insert(v);
                    }
                }
            }
        }
        // Bio, badges, banner, icon and the avatar being worn live in the newer profile record.
        if let Ok(Value::Object(public)) = public {
            if let Some(target) = json.as_object_mut() {
                for (k, v) in &public {
                    let empty = match target.get(k) {
                        None | Some(Value::Null) => true,
                        Some(Value::String(s)) => s.is_empty(),
                        Some(Value::Array(a)) => v.as_array().is_some_and(|b| b.len() > a.len()),
                        _ => false,
                    };
                    let has_value = !matches!(v, Value::Null) && v.as_str() != Some("");
                    if empty && has_value {
                        target.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        // Profiles no longer carry the worn avatar's picture; the friend list still does.
        let worn = self
            .inner
            .lock()
            .unwrap()
            .friends
            .get(user_id)
            .map(|f| f.avatar_image.clone())
            .unwrap_or_default();
        if let Some(target) = json.as_object_mut() {
            let has = |k: &str| target.get(k).and_then(Value::as_str).is_some_and(|s| !s.is_empty());
            if !worn.is_empty() && !has("currentAvatarThumbnailImageUrl") && !has("currentAvatarImageUrl") {
                target.insert("currentAvatarThumbnailImageUrl".into(), Value::String(worn));
            }
        }
        let person = Person::from_json(&json).ok_or_else(|| AppError::Parse("that user had no id".into()))?;
        let inner = self.inner.lock().unwrap();
        let store = inner.store.as_ref().ok_or(AppError::NotSignedIn)?;
        Ok(Profile {
            note: store.note(user_id)?,
            together: store.together(user_id)?,
            feed: store.feed_for(user_id, 40)?,
            past_names: store.past_names(user_id).unwrap_or_default(),
            person,
            raw: json,
        })
    }

    /// One tab of a profile, fetched only when it is opened.
    pub async fn profile_part(&self, user_id: &str, part: &str, extra: &str, provider: &str) -> AppResult<Value> {
        let id = utf8(user_id);
        let mine = self.me().is_some_and(|me| me.id == user_id);
        match part {
            "mutualFriends" => self.api.read(&format!("/users/{id}/mutuals/friends?n=100")).await,
            "mutualGroups" => self.api.read(&format!("/users/{id}/mutuals/groups?n=100")).await,
            "groups" => self.api.read(&format!("/users/{id}/groups")).await,
            "represented" => self.api.read(&format!("/users/{id}/groups/represented")).await,
            "worlds" if mine => {
                self.api
                    .read("/worlds?user=me&releaseStatus=all&n=100&sort=updated&order=descending")
                    .await
            }
            "worlds" => {
                self.api
                    .read(&format!("/worlds?userId={id}&releaseStatus=public&n=60&sort=updated&order=descending"))
                    .await
            }
            "favoriteWorlds" => {
                if mine {
                    return self.api.read("/worlds/favorites?n=100").await;
                }
                // Other people's favourites come group by group, and only the ones they made public.
                let groups = self.api.read(&format!("/favorite/groups?ownerId={id}&n=50")).await?;
                let mut all = Vec::new();
                for g in groups.as_array().into_iter().flatten() {
                    let world = g.get("type").and_then(Value::as_str) == Some("world");
                    let Some(tag) = g.get("name").and_then(Value::as_str) else { continue };
                    if !world {
                        continue;
                    }
                    let page = self
                        .api
                        .read(&format!("/worlds/favorites?n=100&userId={id}&tag={}", utf8(tag)))
                        .await
                        .unwrap_or(Value::Null);
                    all.extend(page.as_array().into_iter().flatten().cloned());
                }
                Ok(Value::Array(all))
            }
            "avatars" => {
                if mine {
                    // Every page, so the local record of my avatars is complete.
                    let all = self
                        .api
                        .read_all("/avatars?user=me&releaseStatus=all&sort=updated&order=descending")
                        .await?;
                    let seen: Vec<(String, String, String)> = all
                        .iter()
                        .filter_map(|a| {
                            let t = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                            let id = t("id");
                            let image = if t("thumbnailImageUrl").is_empty() { t("imageUrl") } else { t("thumbnailImageUrl") };
                            (!id.is_empty()).then(|| (id, t("name"), image))
                        })
                        .collect();
                    let _ = self.with_store(|s| s.record_own_avatars(&seen));
                    Ok(Value::Array(all))
                } else {
                    self.api.ask_avatar_database(provider, "authorId", user_id).await
                }
            }
            // `extra` is my own avatar's id, or the file id of the picture someone else's avatar uses.
            "wearing" => {
                if mine && extra.starts_with("avtr_") {
                    self.api.read(&format!("/avatars/{}", utf8(extra))).await
                } else if extra.starts_with("file_") {
                    // A public avatar is in the community database. A private one is not, but
                    // VRChat still names the picture file after the avatar, so its name can be shown.
                    let found = self.api.ask_avatar_database(provider, "fileId", extra).await.unwrap_or(Value::Null);
                    let empty = found.is_null() || found.as_array().is_some_and(|a| a.is_empty());
                    if !empty {
                        return Ok(found);
                    }
                    let file = self.api.read(&format!("/file/{}", utf8(extra))).await?;
                    match avatar_name_from_file(file["name"].as_str().unwrap_or_default()) {
                        Some(name) => Ok(json!({ "name": name, "authorId": file["ownerId"], "isPrivate": true })),
                        // A picture that was swapped for a custom one is named after its own file id, with no avatar name in it.
                        None => Ok(Value::Null),
                    }
                } else {
                    Ok(Value::Null)
                }
            }
            _ => Err(AppError::CannotDo("There is no such part of a profile.".into())),
        }
    }

    /// A world, avatar or group, exactly as VRChat describes it.
    pub async fn thing(&self, kind: &str, id: &str) -> AppResult<Value> {
        let raw_id = id;
        let id = utf8(id);
        match kind {
            "world" => self.api.read(&format!("/worlds/{id}")).await,
            "avatar" => self.api.read(&format!("/avatars/{id}")).await,
            "group" => self.api.read(&format!("/groups/{id}?includeRoles=true")).await,
            "groupMembers" => self.api.read(&format!("/groups/{id}/members?n=100")).await,
            "groupPosts" => self.api.read(&format!("/groups/{id}/posts?n=10")).await,
            "groupInstances" => self.api.read(&format!("/groups/{id}/instances")).await,
            // `grp_x/ggal_y`: the pictures of one gallery of a group.
            "groupPhotos" => match raw_id.split_once('/') {
                Some((group, gallery)) if group.starts_with("grp_") && gallery.starts_with("ggal_") => {
                    self.api
                        .read(&format!("/groups/{}/galleries/{}?n=60", utf8(group), utf8(gallery)))
                        .await
                }
                _ => Err(AppError::CannotDo("That is not a group gallery.".into())),
            },
            "instance" if raw_id.starts_with("wrld_") || raw_id.starts_with("wld_") => self.api.instance(raw_id).await,
            _ => Err(AppError::CannotDo("There is no such thing to open.".into())),
        }
    }

    /// My favourites of one kind: the groups, every entry, and the items VRChat can still show.
    /// Only the names of my favourite groups: one quick question to VRChat.
    pub async fn favorite_group_list(&self) -> AppResult<Value> {
        Ok(json!({ "groups": self.api.favorite_groups().await?, "counts": {} }))
    }

    /// Every favourite group of mine with how many favourites it holds. Much lighter than
    /// `favorites_of`: it never asks for the avatars, worlds or people themselves.
    pub async fn favorite_groups_overview(&self) -> AppResult<Value> {
        let (groups, friends, avatars, worlds) = tokio::join!(
            self.api.favorite_groups(),
            self.api.read_all("/favorites?type=friend"),
            self.api.read_all("/favorites?type=avatar"),
            self.api.read_all("/favorites?type=world"),
        );
        let mut counts = serde_json::Map::new();
        for (kind, entries) in [("friend", friends), ("avatar", avatars), ("world", worlds)] {
            for entry in entries.unwrap_or_default() {
                let Some(group) = entry.pointer("/tags/0").and_then(Value::as_str) else { continue };
                let key = format!("{kind}:{group}");
                let had = counts.get(&key).and_then(Value::as_u64).unwrap_or(0);
                counts.insert(key, json!(had + 1));
            }
        }
        Ok(json!({ "groups": groups?, "counts": counts }))
    }

    pub async fn favorites_of(&self, kind: &str) -> AppResult<Value> {
        if !matches!(kind, "world" | "avatar" | "friend") {
            return Err(AppError::CannotDo("Favourites are worlds, avatars or friends.".into()));
        }
        let entries_path = format!("/favorites?type={kind}");
        let (groups, entries, limits) = tokio::join!(
            self.api.favorite_groups(),
            self.api.read_all(&entries_path),
            self.api.read("/auth/user/favoritelimits"),
        );
        let groups: Vec<Value> = groups?
            .into_iter()
            .filter(|g| g.get("type").and_then(Value::as_str) == Some(kind))
            .collect();
        let entries = entries?;
        let fallback = match kind {
            "world" => 100,
            "friend" => 150,
            _ => 50,
        };
        let capacity = limits
            .ok()
            .and_then(|l| l.pointer(&format!("/maxFavoritesPerGroup/{kind}")).and_then(Value::as_u64))
            .unwrap_or(fallback);
        let mut items: Vec<Value> = Vec::new();
        let mut complete = true;
        if kind != "friend" {
            // VRChat hands these out one favourite group at a time; all groups in use are asked at once.
            let base = if kind == "world" { "/worlds/favorites" } else { "/avatars/favorites" };
            let mut tags: Vec<String> = entries
                .iter()
                .filter_map(|e| e.pointer("/tags/0").and_then(Value::as_str))
                .map(str::to_string)
                .collect();
            tags.sort();
            tags.dedup();
            let paths: Vec<String> = tags.iter().map(|tag| format!("{base}?tag={}", utf8(tag))).collect();
            let answers = futures_util::future::join_all(paths.iter().map(|p| self.api.read_all(p))).await;
            let mut seen = HashSet::new();
            for answer in answers {
                match answer {
                    Ok(list) => {
                        for item in list {
                            let id = item.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                            if !id.is_empty() && seen.insert(id) {
                                items.push(item);
                            }
                        }
                    }
                    Err(_) => complete = false,
                }
            }
        }
        let all = json!({ "groups": groups, "entries": entries, "items": items, "complete": complete, "capacity": capacity });
        self.fav_cache.lock().unwrap().insert(kind.to_string(), all.clone());
        Ok(all)
    }

    /// Looks for favourites that are really gone. Anything VRChat did not list is asked
    /// about one by one, and only a clear "not found" or "private" answer counts.
    pub async fn check_favorites(&self, kind: &str) -> AppResult<Value> {
        const MOST: usize = 150;
        if !matches!(kind, "world" | "avatar") {
            return Err(AppError::CannotDo("Only avatars and worlds can be checked.".into()));
        }
        let me = self.me().map(|m| m.id).unwrap_or_default();
        let all = self.favorites_of(kind).await?;
        let none = Vec::new();
        let listed: HashMap<&str, &Value> = all["items"]
            .as_array()
            .unwrap_or(&none)
            .iter()
            .filter_map(|i| Some((i.get("id")?.as_str()?, i)))
            .collect();
        let hidden = |v: &Value| {
            let status = v.get("releaseStatus").and_then(Value::as_str).unwrap_or("public");
            let author = v.get("authorId").and_then(Value::as_str).unwrap_or("");
            status != "public" && author != me
        };
        let suspects: Vec<&Value> = all["entries"]
            .as_array()
            .unwrap_or(&none)
            .iter()
            .filter(|e| {
                let id = e.get("favoriteId").and_then(Value::as_str).unwrap_or("");
                listed.get(id).is_none_or(|item| hidden(item))
            })
            .collect();
        let mut dead = Vec::new();
        let mut checked = 0usize;
        let mut stopped = false;
        for entry in suspects.iter().take(MOST) {
            let id = entry.get("favoriteId").and_then(Value::as_str).unwrap_or("");
            let path = if kind == "world" { format!("/worlds/{}", utf8(id)) } else { format!("/avatars/{}", utf8(id)) };
            let verdict = match self.api.read(&path).await {
                Ok(v) if hidden(&v) => Some(("private", v.get("name").and_then(Value::as_str).unwrap_or("").to_string())),
                Ok(_) => None,
                Err(AppError::Api { status: 404, .. }) => Some(("deleted", String::new())),
                Err(AppError::RateLimited) => {
                    stopped = true;
                    break;
                }
                Err(_) => None,
            };
            checked += 1;
            if let Some((why, name)) = verdict {
                dead.push(json!({ "entryId": entry.get("id"), "favoriteId": id, "why": why, "name": name }));
            }
        }
        Ok(json!({
            "dead": dead,
            "checked": checked,
            "left": suspects.len().saturating_sub(checked),
            "stopped": stopped,
        }))
    }

    /// `join` or `leave` a group.
    pub async fn group_action(&self, group_id: &str, action: &str) -> AppResult<()> {
        if !group_id.starts_with("grp_") {
            return Err(AppError::CannotDo("That is not a group.".into()));
        }
        match action {
            "join" => self.api.join_group(group_id).await,
            "leave" => self.api.leave_group(group_id).await,
            _ => Err(AppError::CannotDo("Groups can be joined or left.".into())),
        }
    }

    /// The favourites answer from last time, if there is one.
    pub fn favorites_cached(&self, kind: &str) -> Option<Value> {
        self.fav_cache.lock().unwrap().get(kind).cloned()
    }

    pub async fn boop(&self, user_id: &str) -> AppResult<()> {
        if !user_id.starts_with("usr_") {
            return Err(AppError::CannotDo("Only a person can be booped.".into()));
        }
        self.api.boop(user_id).await
    }

    pub async fn picture(&self, url: &str) -> AppResult<(String, Vec<u8>)> {
        self.api.picture_as(url, true).await
    }

    /// Changes one of my own uploads: `public`, `private` or `delete`. `kind` is `avatar` or `world`.
    /// VRChat is asked first whose upload it is; anything not mine is refused.
    pub async fn upload_action(&self, kind: &str, id: &str, action: &str) -> AppResult<()> {
        let fits = match kind {
            "avatar" => id.starts_with("avtr_"),
            "world" => id.starts_with("wrld_") || id.starts_with("wld_"),
            _ => false,
        };
        if !fits || !matches!(action, "public" | "private" | "delete") {
            return Err(AppError::CannotDo("That cannot be done to an upload.".into()));
        }
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        let upload = self.thing(kind, id).await?;
        if upload.get("authorId").and_then(Value::as_str) != Some(me.id.as_str()) {
            return Err(AppError::CannotDo("Only your own uploads can be changed or deleted.".into()));
        }
        match (kind, action) {
            ("avatar", "delete") => self.api.delete_avatar(id).await,
            ("avatar", status) => self.api.set_avatar_release(id, status).await,
            ("world", "delete") => self.api.delete_world(id).await,
            ("world", status) => self.api.set_world_public(id, status == "public").await,
            _ => unreachable!("kind was checked above"),
        }
    }

    /// `add` sends a friend request, `cancel` takes it back, `remove` unfriends.
    pub async fn friend_action(&self, user_id: &str, action: &str) -> AppResult<()> {
        if !user_id.starts_with("usr_") || self.me().is_some_and(|me| me.id == user_id) {
            return Err(AppError::CannotDo("That only works on another person.".into()));
        }
        match action {
            "add" => self.api.friend_request(user_id).await,
            "cancel" => self.api.cancel_friend_request(user_id).await,
            "remove" => self.api.unfriend(user_id).await,
            _ => Err(AppError::CannotDo("That is not something to do with a friend.".into())),
        }
    }

    /// The people I blocked or muted, newest first. VRChat is asked once; after that the list
    /// is kept until something changes or `fresh` asks again.
    pub async fn moderations(&self, fresh: bool) -> AppResult<Value> {
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        if !fresh {
            if let Some((owner, list)) = self.blocked.lock().unwrap().as_ref() {
                if *owner == me.id {
                    return Ok(list.clone());
                }
            }
        }
        let all = self.api.player_moderations().await?;
        let mut rows: Vec<Value> = all
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or_default()
            .iter()
            .filter(|m| matches!(m["type"].as_str(), Some("block" | "mute")))
            .map(|m| json!({ "kind": m["type"], "userId": m["targetUserId"], "name": m["targetDisplayName"], "at": m["created"] }))
            .collect();
        rows.sort_by(|a, b| b["at"].as_str().unwrap_or_default().cmp(a["at"].as_str().unwrap_or_default()));
        let list = Value::Array(rows);
        *self.blocked.lock().unwrap() = Some((me.id, list.clone()));
        Ok(list)
    }

    /// One month of events: `mine` (my groups), `following` or `featured`. `month` is `YYYY-MM`.
    /// Each list and month is asked for at most once in ten minutes; `fresh` asks again now.
    pub async fn calendar(&self, which: &str, month: &str, fresh: bool) -> AppResult<Value> {
        let list = match which {
            "mine" => "",
            "following" => "following",
            "featured" => "featured",
            _ => return Err(AppError::CannotDo("That is not a calendar this app knows.".into())),
        };
        let day = chrono::NaiveDate::parse_from_str(&format!("{month}-15"), "%Y-%m-%d")
            .map_err(|_| AppError::CannotDo("That is not a month.".into()))?;
        if self.me().is_none() {
            return Err(AppError::NotSignedIn);
        }
        let key = format!("{which}:{month}");
        if !fresh {
            if let Some((at, known)) = self.calendars.lock().unwrap().get(&key) {
                if at.elapsed() < Duration::from_secs(600) {
                    return Ok(known.clone());
                }
            }
        }
        let answer = self.api.calendar(list, &format!("{}T12:00:00.000Z", day.format("%Y-%m-%d"))).await?;
        let events: Vec<Value> = answer["results"]
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or_default()
            .iter()
            .filter(|e| e["deletedAt"].is_null() && e["isDraft"].as_bool() != Some(true))
            .map(|e| {
                json!({
                    "id": e["id"],
                    "groupId": e["ownerId"],
                    "groupName": e["ownerName"],
                    "title": e["title"],
                    "description": e["description"],
                    "category": e["category"],
                    "access": e["accessType"],
                    "startsAt": e["startsAt"],
                    "endsAt": e["endsAt"],
                    "image": e["imageUrl"],
                    "interested": e["interestedUserCount"],
                    "friends": e["interestedFriendCount"],
                    "following": e["userInterest"]["isFollowing"].as_bool().unwrap_or(false),
                    "featured": e["featured"].as_bool().unwrap_or(false),
                })
            })
            .collect();
        let out = json!({ "events": events, "more": answer["hasNext"].as_bool().unwrap_or(false) });
        self.calendars.lock().unwrap().insert(key, (Instant::now(), out.clone()));
        Ok(out)
    }

    /// Follows an event of a group, or stops following it. What was kept of the calendar is
    /// dropped, so the lists are read again.
    pub async fn follow_event(&self, group_id: &str, event_id: &str, on: bool) -> AppResult<()> {
        if !group_id.starts_with("grp_") || !event_id.starts_with("cal_") || group_id.len() > 80 || event_id.len() > 80 {
            return Err(AppError::CannotDo("That is not a group event.".into()));
        }
        if self.me().is_none() {
            return Err(AppError::NotSignedIn);
        }
        self.api.follow_event(group_id, event_id, on).await?;
        self.calendars.lock().unwrap().clear();
        Ok(())
    }

    /// The little a list row shows about someone who is not a friend: picture, rank, marks and
    /// status. One request the first time, none after that.
    pub async fn person_brief(&self, user_id: &str) -> AppResult<Value> {
        if !user_id.starts_with("usr_") {
            return Err(AppError::CannotDo("That is not a person.".into()));
        }
        if self.me().is_none() {
            return Err(AppError::NotSignedIn);
        }
        // Kept for ten minutes, so a status shown in a list is never much older than that.
        if let Some((at, known)) = self.briefs.lock().unwrap().get(user_id) {
            if at.elapsed() < Duration::from_secs(600) {
                return Ok(known.clone());
            }
        }
        let user = self.api.user(user_id).await?;
        let state = user["state"].as_str().unwrap_or("offline").to_string();
        let brief = Friend::from_user(&user, &state).map(|f| json!(f)).ok_or_else(|| AppError::Parse("that user had no id".into()))?;
        self.briefs.lock().unwrap().insert(user_id.to_string(), (Instant::now(), brief.clone()));
        Ok(brief)
    }

    /// Blocks or mutes someone, or takes that back. Nothing else can be set from here.
    pub async fn set_moderation(&self, user_id: &str, kind: &str, on: bool) -> AppResult<()> {
        if !user_id.starts_with("usr_") || self.me().is_some_and(|me| me.id == user_id) {
            return Err(AppError::CannotDo("That only works on another person.".into()));
        }
        if !matches!(kind, "block" | "mute") {
            return Err(AppError::CannotDo("Only a block or a mute can be set from here.".into()));
        }
        if self.me().is_none() {
            return Err(AppError::NotSignedIn);
        }
        if on {
            self.api.moderate(user_id, kind).await?;
        } else {
            self.api.unmoderate(user_id, kind).await?;
        }
        *self.blocked.lock().unwrap() = None;
        Ok(())
    }

    /// Whether we are friends, and whether a request is waiting either way.
    pub async fn friend_status(&self, user_id: &str) -> AppResult<Value> {
        self.api.read(&format!("/user/{}/friendStatus", utf8(user_id))).await
    }

    /// Renames one of my favourite groups. VRChat fixes how many groups there are; only the names are mine.
    pub async fn rename_favorite_group(&self, kind: &str, group: &str, name: &str) -> AppResult<()> {
        let name = name.trim();
        if !matches!(kind, "world" | "avatar" | "friend") || group.is_empty() || name.is_empty() || name.chars().count() > 24 {
            return Err(AppError::CannotDo("A group name needs 1 to 24 characters.".into()));
        }
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        self.api.rename_favorite_group(kind, group, &me.id, name).await?;
        self.fav_cache.lock().unwrap().remove(kind);
        if kind == "friend" {
            let _ = self.sync_favorites().await;
        }
        Ok(())
    }

    pub async fn add_favorite(&self, kind: &str, id: &str, group: &str) -> AppResult<()> {
        let fits = match kind {
            "world" => id.starts_with("wrld_") || id.starts_with("wld_"),
            "avatar" => id.starts_with("avtr_"),
            "friend" => id.starts_with("usr_"),
            _ => false,
        };
        if !fits || group.is_empty() {
            return Err(AppError::CannotDo("That cannot be added to favourites.".into()));
        }
        self.api.add_favorite(kind, id, group).await?;
        self.fav_cache.lock().unwrap().remove(kind);
        if kind == "friend" {
            let _ = self.sync_favorites().await;
        }
        Ok(())
    }

    /// Removes favourites one after another. Returns how many VRChat confirmed.
    pub async fn remove_favorites(&self, entry_ids: &[String]) -> AppResult<usize> {
        let mut done = 0;
        for id in entry_ids.iter().filter(|i| i.starts_with("fvrt_")) {
            match self.api.remove_favorite(id).await {
                Ok(()) => done += 1,
                Err(AppError::RateLimited) => break,
                Err(_) => {}
            }
        }
        self.fav_cache.lock().unwrap().clear();
        let _ = self.sync_favorites().await;
        Ok(done)
    }

    pub fn set_note(&self, user_id: &str, note: &str) -> AppResult<()> {
        let inner = self.inner.lock().unwrap();
        match &inner.store {
            Some(store) => store.set_note(user_id, note),
            None => Err(AppError::NotSignedIn),
        }
    }

    /// User ids I have written a note about.
    pub fn noted(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        inner.store.as_ref().and_then(|s| s.noted().ok()).unwrap_or_default()
    }

    pub fn friend_log(&self, limit: u32) -> AppResult<Vec<FeedItem>> {
        let inner = self.inner.lock().unwrap();
        match &inner.store {
            Some(store) => store.friend_log(limit),
            None => Err(AppError::NotSignedIn),
        }
    }

    pub async fn search_users(&self, query: &str) -> AppResult<Vec<Person>> {
        let found = self.api.search_users(query.trim()).await?;
        Ok(found.iter().filter_map(Person::from_json).collect())
    }

    pub async fn search_worlds(&self, query: &str) -> AppResult<Vec<WorldCard>> {
        let found = self.api.search_worlds(query.trim()).await?;
        Ok(found.iter().filter_map(WorldCard::from_json).collect())
    }

    pub async fn search_avatars(&self, provider: &str, query: &str) -> AppResult<Vec<AvatarCard>> {
        let found = self.api.search_avatars(provider.trim(), query.trim()).await?;
        Ok(found.iter().filter_map(AvatarCard::from_json).collect())
    }

    /// Invites a friend to the instance I am in.
    pub async fn invite_here(&self, user_id: &str) -> AppResult<()> {
        let is_friend = self.inner.lock().unwrap().friends.contains_key(user_id);
        if !is_friend {
            return Err(AppError::CannotDo("Only friends can be invited.".into()));
        }
        let here = self.here()?.map(|(visit, _)| visit.location);
        let Some(location) = here else {
            return Err(AppError::CannotDo(
                "You are not in a world right now, so there is nowhere to invite them to.".into(),
            ));
        };
        self.api.invite_user(user_id, &location).await
    }

    /// What the user did with earlier notifications, newest first.
    pub fn handled(&self, limit: u32) -> AppResult<Vec<Handled>> {
        let inner = self.inner.lock().unwrap();
        match &inner.store {
            Some(store) => store.handled(limit),
            None => Err(AppError::NotSignedIn),
        }
    }

    pub fn notices(&self) -> Vec<Notice> {
        self.inner.lock().unwrap().notices.clone()
    }

    /// Fetches every open notification. Two requests. With `announce`, anything that came in
    /// while the live connection was down is announced like a live one.
    pub async fn sync_notices(&self, announce: bool) -> AppResult<()> {
        let v1 = self.api.notifications().await?;
        let v2 = self.api.notifications_v2().await.unwrap_or_default();
        let fresh = notices::from_api(&v1, &v2);
        let new_ones: Vec<Notice> = {
            let mut inner = self.inner.lock().unwrap();
            let known: HashSet<String> = inner.notices.iter().map(|n| n.id.clone()).collect();
            let new_ones = fresh.iter().filter(|n| !known.contains(&n.id) && !n.seen).cloned().collect();
            inner.notices = fresh;
            new_ones
        };
        self.send("notices:changed", Value::Null);
        if announce {
            for notice in &new_ones {
                self.announce(notice);
            }
        }
        Ok(())
    }

    /// Carries out what the user clicked on a notification.
    /// `action` is `accept`, `dismiss`, `reply` or `inviteBack`.
    pub async fn notice_action(&self, id: &str, action: &str, reply_kind: &str, reply_data: &str) -> AppResult<()> {
        let notice = self
            .notices()
            .into_iter()
            .find(|n| n.id == id)
            .ok_or_else(|| AppError::CannotDo("That notification is already gone.".into()))?;
        match (action, notice.version) {
            ("accept", 1) if notice.kind == "friendRequest" => {
                self.api.accept_friend_request(id).await?;
            }
            ("dismiss", 1) => self.api.hide_notification(id).await?,
            ("dismiss", _) => self.api.delete_notification_v2(id).await?,
            ("reply", 2) => {
                if !notice.replies.iter().any(|r| r.kind == reply_kind && r.data == reply_data) {
                    return Err(AppError::CannotDo("That reply is not on offer.".into()));
                }
                self.api.respond_notification_v2(id, reply_kind, reply_data).await?;
            }
            ("boopBack", _) if notice.kind == "boop" => {
                self.api.boop(&notice.sender_id).await?;
                let _ = if notice.version == 2 {
                    self.api.delete_notification_v2(id).await
                } else {
                    self.api.hide_notification(id).await
                };
            }
            ("inviteBack", 1) if notice.kind == "requestInvite" => {
                let here = self.here()?.map(|(visit, _)| visit.location);
                let Some(location) = here else {
                    return Err(AppError::CannotDo(
                        "You are not in a world right now, so there is nowhere to invite them to.".into(),
                    ));
                };
                self.api.invite_user(&notice.sender_id, &location).await?;
                self.api.hide_notification(id).await?;
            }
            _ => return Err(AppError::CannotDo("That does not work on this notification.".into())),
        }
        let reply_text = notice
            .replies
            .iter()
            .find(|r| action == "reply" && r.kind == reply_kind && r.data == reply_data)
            .map(|r| if r.text.is_empty() { r.kind.clone() } else { r.text.clone() })
            .unwrap_or_default();
        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(store) = &inner.store {
                let _ = store.log_handled(&notice, action, &reply_text);
            }
            notices::remove(&mut inner.notices, id);
        }
        self.send("notices:changed", Value::Null);
        Ok(())
    }

    pub fn pipeline_status(&self) -> &'static str {
        self.inner.lock().unwrap().pipeline
    }

    pub fn feed(&self, limit: u32, before: Option<i64>) -> AppResult<Vec<FeedItem>> {
        let inner = self.inner.lock().unwrap();
        match &inner.store {
            Some(store) => store.feed(limit, before),
            None => Err(AppError::NotSignedIn),
        }
    }

    /// The instance the game is in now, with everyone seen there.
    /// The game closed without a goodbye: the stay it left open ends now, so nothing later
    /// acts on an instance I am no longer in.
    pub fn game_gone(&self) {
        if self.with_store(|s| s.close_open_visits(&now_utc())).is_ok() {
            self.send("here:changed", Value::Null);
        }
    }

    pub fn here(&self) -> AppResult<Option<(Visit, Vec<Sighting>)>> {
        let inner = self.inner.lock().unwrap();
        let Some(store) = &inner.store else {
            return Err(AppError::NotSignedIn);
        };
        match store.open_visit()? {
            Some(visit) => {
                let people = store.visit_people(visit.id)?;
                Ok(Some((visit, people)))
            }
            None => Ok(None),
        }
    }

    pub fn visits(&self, limit: u32, before: Option<i64>) -> AppResult<Vec<Visit>> {
        let inner = self.inner.lock().unwrap();
        match &inner.store {
            Some(store) => store.visits(limit, before),
            None => Err(AppError::NotSignedIn),
        }
    }

    /// One page of VRChat's photos under `root`, newest first, each with the world I was in and
    /// who was in the instance at that moment, from the app's own history.
    ///
    /// `query` narrows it to photos whose world name or one of whose people's names contains
    /// the text. `until` (a day, `YYYY-MM-DD`, in the PC's own time) leaves out anything taken
    /// after that day. Both come from the history on this PC; nothing is asked of VRChat and
    /// no photo is opened or changed. "With someone" means they were in the instance when the
    /// photo was taken, not that they are in the picture.
    pub fn photos(&self, root: &std::path::Path, limit: usize, skip: usize, query: &str, until: &str) -> AppResult<Value> {
        let all = crate::photos::scan(root);
        let me = self.me().map(|m| m.id).unwrap_or_default();
        let want = query.trim().to_lowercase();
        // A photo's time is kept in UTC; the day the user picks is a day on their own clock.
        let last = chrono::NaiveDate::parse_from_str(until.trim(), "%Y-%m-%d")
            .ok()
            .and_then(|d| d.succ_opt())
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .and_then(|t| chrono::TimeZone::from_local_datetime(&chrono::Local, &t).earliest())
            .map(|t| t.with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        let inner = self.inner.lock().unwrap();
        let store = inner.store.as_ref().ok_or(AppError::NotSignedIn)?;
        let describe = |photo: &crate::photos::Photo| -> AppResult<(Value, bool)> {
            let visit = store.visit_at(&photo.taken_at)?;
            let people: Vec<(String, String)> = match &visit {
                Some(v) => store
                    .people_at(v.id, &photo.taken_at)?
                    .into_iter()
                    .filter(|(id, _)| id.is_empty() || *id != me)
                    .collect(),
                None => Vec::new(),
            };
            let world_name = visit.as_ref().map(|v| v.world_name.clone()).unwrap_or_default();
            let hit = want.is_empty()
                || world_name.to_lowercase().contains(&want)
                || people.iter().any(|(_, name)| name.to_lowercase().contains(&want));
            let row = serde_json::json!({
                "path": photo.path,
                "file": photo.file,
                "takenAt": photo.taken_at,
                "width": photo.width,
                "height": photo.height,
                "worldId": visit.as_ref().map(|v| v.world_id.clone()).unwrap_or_default(),
                "worldName": world_name,
                "location": visit.as_ref().map(|v| v.location.clone()).unwrap_or_default(),
                "people": people.iter().map(|(id, name)| serde_json::json!({ "userId": id, "displayName": name })).collect::<Vec<_>>(),
            });
            Ok((row, hit))
        };
        let in_range = |photo: &&crate::photos::Photo| last.as_ref().is_none_or(|end| photo.taken_at.as_str() < end.as_str());
        let limit = limit.min(200);
        let mut page = Vec::new();
        if want.is_empty() {
            // Nothing to look up for photos that are not on this page.
            let dated: Vec<&crate::photos::Photo> = all.iter().filter(in_range).collect();
            for photo in dated.iter().skip(skip).take(limit) {
                page.push(describe(photo)?.0);
            }
            return Ok(serde_json::json!({ "total": dated.len(), "everything": all.len(), "photos": page }));
        }
        let mut found = 0usize;
        for photo in all.iter().filter(in_range) {
            let (row, hit) = describe(photo)?;
            if !hit {
                continue;
            }
            if found >= skip && page.len() < limit {
                page.push(row);
            }
            found += 1;
        }
        Ok(serde_json::json!({ "total": found, "everything": all.len(), "photos": page }))
    }

    pub fn visit_people(&self, visit_id: i64) -> AppResult<Vec<Sighting>> {
        let inner = self.inner.lock().unwrap();
        match &inner.store {
            Some(store) => store.visit_people(visit_id),
            None => Err(AppError::NotSignedIn),
        }
    }

    /// Reads new game-log lines every couple of seconds. Local files only.
    /// Brings the history up to date with the game log. The files are read with the history
    /// free, and it is only held for the moment each batch of lines is written, so a big
    /// backlog does not freeze the window.
    fn read_log(&self, dir: &std::path::Path) -> bool {
        let plan = {
            let inner = self.inner.lock().unwrap();
            match &inner.store {
                Some(store) => gamelog::plan(dir, store).ok().flatten(),
                None => None,
            }
        };
        let mut changed = false;
        for mut pending in plan.unwrap_or_default() {
            while let Ok(Some(chunk)) = gamelog::read_next(&mut pending) {
                let inner = self.inner.lock().unwrap();
                let Some(store) = &inner.store else { return changed };
                match store.record_log(&chunk.events, chunk.fresh_file, &pending.key, chunk.offset) {
                    Ok(true) => changed = true,
                    Ok(false) => {}
                    Err(_) => break,
                }
            }
        }
        changed
    }

    async fn log_worker(self: Arc<Self>, dir: PathBuf) {
        // The stay I was in when the last pass ended; a different one means I moved.
        let mut stay: Option<Option<i64>> = None;
        loop {
            let changed = self.read_log(&dir);
            if let Ok(here) = self.here() {
                let now = here.as_ref().map(|(visit, _)| visit.id);
                if let (Some(before), Some((visit, _))) = (stay, here.as_ref()) {
                    if before != now {
                        self.own_entry("location", "", "", &visit.location);
                    }
                }
                stay = Some(now);
            }
            if changed {
                if let Ok(Some((visit, _))) = self.here() {
                    if !visit.world_id.is_empty() {
                        self.want_world(&visit.world_id);
                    }
                    self.want_count(&visit.location);
                }
                self.send("here:changed", Value::Null);
            }
            tokio::time::sleep(LOG_EVERY).await;
        }
    }

    fn set_pipeline(&self, status: &'static str) {
        self.inner.lock().unwrap().pipeline = status;
        self.send("pipeline:status", json!(status));
    }

    /// Tries the saved cookies. Returns the user when they still work.
    pub async fn restore(self: &Arc<Self>) -> AppResult<Option<Me>> {
        if let Some(me) = self.me() {
            return Ok(Some(me));
        }
        if !self.use_secrets {
            return Ok(None);
        }
        let Some(session) = secrets::load()? else {
            return Ok(None);
        };
        if session.auth.is_none() {
            self.api.set_session(session);
            return Ok(None);
        }
        self.api.set_session(session);
        match self.api.current_user().await {
            Ok(user) => Ok(Some(self.begin(user)?)),
            Err(AppError::Unauthorized) | Err(AppError::TwoFactorRequired) => {
                self.api.clear_auth();
                let _ = secrets::save(&self.api.session());
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    /// Called once VRChat has returned the signed-in user.
    pub fn begin(self: &Arc<Self>, user: Value) -> AppResult<Me> {
        self.stop_tasks();
        let me = Me::from_user(&user);
        if me.id.is_empty() {
            return Err(AppError::Parse("signed-in user had no id".into()));
        }
        if self.use_secrets {
            secrets::save(&self.api.session())?;
        }
        std::fs::create_dir_all(&self.data_dir).map_err(|e| AppError::Storage(e.to_string()))?;
        let path = self.history_path(&me.id);
        backup::finish_restore(&path)?;
        let store = Store::open(&path)?;
        let (tx, rx) = mpsc::unbounded_channel();
        {
            let mut inner = self.inner.lock().unwrap();
            inner.friends = store
                .friends()?
                .into_iter()
                .map(|f| (f.id.clone(), f))
                .collect();
            inner.worlds = store
                .worlds()?
                .into_iter()
                .map(|w| (w.id.clone(), w))
                .collect();
            inner.wanted.clear();
            inner.favorites = store
                .setting("favorites")?
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default();
            inner.store = Some(store);
            inner.me = Some(me.clone());
            inner.world_tx = Some(tx);
            inner.last_sync = None;
        }
        let worlds = tokio::spawn(self.clone().world_worker(rx));
        let counts = tokio::spawn(self.clone().count_ticker());
        let live = tokio::spawn(self.clone().run(user));
        let mut tasks = vec![worlds, live, counts];
        if let Some(dir) = self.log_dir.clone() {
            tasks.push(tokio::spawn(self.clone().log_worker(dir)));
        }
        self.inner.lock().unwrap().tasks = tasks;
        Ok(me)
    }

    fn stop_tasks(&self) {
        let tasks = std::mem::take(&mut self.inner.lock().unwrap().tasks);
        for t in tasks {
            t.abort();
        }
    }

    pub async fn sign_out(&self) -> AppResult<()> {
        self.stop_tasks();
        let _ = self.api.logout().await;
        if self.use_secrets {
            // The remembered-device cookie stays so the next sign-in can skip the code.
            secrets::save(&self.api.session())?;
        }
        self.forget();
        Ok(())
    }

    fn forget(&self) {
        // What was kept for the account that is leaving must not greet the next one.
        self.fav_cache.lock().unwrap().clear();
        *self.blocked.lock().unwrap() = None;
        self.briefs.lock().unwrap().clear();
        self.calendars.lock().unwrap().clear();
        let mut inner = self.inner.lock().unwrap();
        inner.me = None;
        inner.store = None;
        inner.friends.clear();
        inner.favorites = Favorites::default();
        inner.notices.clear();
        inner.worlds.clear();
        inner.wanted.clear();
        inner.instances.clear();
        inner.refused.clear();
        inner.world_tx = None;
        inner.pipeline = "off";
    }

    async fn run(self: Arc<Self>, user: Value) {
        if let Some(id) = user.get("currentAvatar").and_then(Value::as_str) {
            self.avatar_changed(id);
        }
        let (friends, favorites, notices) =
            tokio::join!(self.sync_friends(Some(user)), self.sync_favorites(), self.sync_notices(false));
        for result in [friends, favorites, notices] {
            if let Err(e) = result {
                self.send("sync:error", json!(e));
            }
        }
        // Favourites are read ahead, so their screens open at once.
        let ahead = self.clone();
        tokio::spawn(async move {
            let _ = tokio::join!(ahead.favorites_of("avatar"), ahead.favorites_of("world"), ahead.favorites_of("friend"));
        });
        let mut delay = Duration::from_secs(5);
        loop {
            self.set_pipeline("connecting");
            let started = Instant::now();
            if let Ended::SessionOver = self.listen().await {
                self.api.clear_auth();
                if self.use_secrets {
                    let _ = secrets::save(&self.api.session());
                }
                self.forget();
                self.send("session:ended", Value::Null);
                return;
            }
            self.set_pipeline("reconnecting");
            if started.elapsed() > Duration::from_secs(60) {
                delay = Duration::from_secs(5);
            }
            let jitter = Duration::from_millis(u64::from(started.elapsed().subsec_millis()));
            tokio::time::sleep(delay + jitter).await;
            delay = (delay * 2).min(Duration::from_secs(300));

            let stale = self
                .inner
                .lock()
                .unwrap()
                .last_sync
                .map_or(true, |t| t.elapsed() > RESYNC_AFTER);
            if stale {
                match self.sync_friends(None).await {
                    Ok(()) => {
                        let _ = self.sync_favorites().await;
                    }
                    Err(AppError::Unauthorized) => {
                        self.api.clear_auth();
                        self.forget();
                        self.send("session:ended", Value::Null);
                        return;
                    }
                    Err(e) => self.send("sync:error", json!(e)),
                }
            }
            // Notifications sent while the connection was down are never replayed, so ask for them.
            let _ = self.sync_notices(true).await;
        }
    }

    /// Fetches every friend once and replaces what is in memory and on disk.
    pub async fn sync_friends(&self, user: Option<Value>) -> AppResult<()> {
        let user = match user {
            Some(u) => u,
            None => self.api.current_user().await?,
        };
        let ids = |key: &str| -> HashSet<String> {
            user.get(key)
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        };
        let online = ids("onlineFriends");
        let active = ids("activeFriends");

        let (here, away) = tokio::join!(self.api.friends(false), self.api.friends(true));
        let (here, away) = (here?, away?);
        let mut map: HashMap<String, Friend> = HashMap::new();
        for raw in &here {
            let Some(mut f) = Friend::from_user(raw, "active") else { continue };
            f.state = if online.contains(&f.id) {
                "online"
            } else if active.contains(&f.id) {
                "active"
            } else if f.place.kind != "none" {
                "online"
            } else {
                "active"
            }
            .into();
            map.insert(f.id.clone(), f);
        }
        for raw in &away {
            let Some(mut f) = Friend::from_user(raw, "offline") else { continue };
            if map.contains_key(&f.id) {
                continue;
            }
            f.set_location("offline");
            map.insert(f.id.clone(), f);
        }

        let mut want = Vec::new();
        let mut news = Vec::new();
        {
            let mut inner = self.inner.lock().unwrap();
            // What changed on the friend list since it was last saved, the app being closed or not.
            for draft in events::list_changes(&inner.friends, &map) {
                if let Some(store) = &inner.store {
                    if let Ok(item) = store.add_feed(&draft) {
                        news.push(item);
                    }
                }
            }
            // A friend still in the same place keeps the arrival time already noted.
            for f in map.values_mut() {
                if let Some(old) = inner.friends.get(&f.id) {
                    if old.location == f.location {
                        f.since = old.since.clone();
                        f.since_at_least = old.since_at_least;
                    }
                }
                if f.since.is_empty() && f.state == "online" && f.place.kind != "none" {
                    // Next best: the feed may have recorded the move. Failing that, count from now.
                    let recorded = inner
                        .store
                        .as_ref()
                        .and_then(|s| s.arrived_at(&f.id, &f.location).ok().flatten());
                    match recorded {
                        Some(at) => f.since = at,
                        None => {
                            f.since = now_utc();
                            f.since_at_least = true;
                        }
                    }
                }
            }
            let list: Vec<Friend> = map.values().cloned().collect();
            if let Some(store) = inner.store.as_mut() {
                store.replace_friends(&list)?;
            }
            for f in &list {
                want.extend(world_ids(f));
            }
            inner.friends = map;
            inner.last_sync = Some(Instant::now());
        }
        for id in want {
            self.want_world(&id);
        }
        for item in news {
            self.send("feed:new", json!(item));
        }
        self.send("friends:reset", Value::Null);
        Ok(())
    }

    /// Asks for a world's name and picture when they are not known yet. The answer arrives as
    /// `world:resolved`. Asked once per world; the request gate spaces the questions out.
    pub fn ask_world(&self, world_id: &str) -> AppResult<()> {
        if !(world_id.starts_with("wrld_") || world_id.starts_with("wld_")) || world_id.len() > 80 {
            return Err(AppError::CannotDo("That is not a world.".into()));
        }
        if self.me().is_none() {
            return Err(AppError::NotSignedIn);
        }
        self.want_world(world_id);
        Ok(())
    }

    fn want_world(&self, world_id: &str) {
        let mut inner = self.inner.lock().unwrap();
        if inner.worlds.contains_key(world_id) || inner.wanted.contains(world_id) {
            return;
        }
        inner.wanted.insert(world_id.to_string());
        if let Some(tx) = &inner.world_tx {
            let _ = tx.send(Job::World(world_id.to_string()));
        }
    }

    pub fn instances(&self) -> Vec<Instance> {
        let inner = self.inner.lock().unwrap();
        inner.instances.values().map(|(i, _)| i.clone()).collect()
    }

    /// Queues a head count for an instance unless a recent one is already known.
    fn want_count(&self, location: &str) {
        let mut inner = self.inner.lock().unwrap();
        let fresh = inner
            .instances
            .get(location)
            .is_some_and(|(_, at)| at.elapsed() < COUNT_FRESH);
        let refused = inner
            .refused
            .get(location)
            .is_some_and(|at| at.elapsed() < COUNT_RETRY);
        let key = format!("count:{location}");
        if fresh || refused || inner.wanted.contains(&key) {
            return;
        }
        inner.wanted.insert(key);
        if let Some(tx) = &inner.world_tx {
            let _ = tx.send(Job::Instance(location.to_string()));
        }
    }

    /// Every minute: ask for counts that have gone stale, forget instances nobody is in.
    async fn count_ticker(self: Arc<Self>) {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let occupied: HashSet<String> = {
                let inner = self.inner.lock().unwrap();
                inner
                    .friends
                    .values()
                    .filter(|f| f.state == "online" && f.place.kind == "world")
                    .map(|f| f.location.clone())
                    .collect()
            };
            {
                let mut inner = self.inner.lock().unwrap();
                inner.instances.retain(|loc, _| occupied.contains(loc));
                inner.refused.retain(|loc, _| occupied.contains(loc));
            }
            for location in &occupied {
                self.want_count(location);
            }
            tokio::time::sleep(COUNT_EVERY).await;
        }
    }

    /// Sends me an invite to an instance. It shows up inside VRChat.
    pub async fn invite_myself(&self, location: &str) -> AppResult<()> {
        if parse_location(location).kind != "world" {
            return Err(AppError::CannotDo("That is not an instance you can be invited to.".into()));
        }
        self.api.invite_myself(location).await
    }

    /// Makes a new instance of a world, owned by me. `access` is one of `public`, `friendsPlus`,
    /// `friends`, `invitePlus`, `invite`; `region` one of `us`, `use`, `eu`, `jp`.
    pub async fn create_instance(&self, world_id: &str, access: &str, region: &str) -> AppResult<String> {
        if !world_id.starts_with("wrld_") || world_id.contains(':') {
            return Err(AppError::CannotDo("That is not a world.".into()));
        }
        if !["us", "use", "eu", "jp"].contains(&region) {
            return Err(AppError::CannotDo("That is not a region VRChat has.".into()));
        }
        let (kind, mine, ask) = match access {
            "public" => ("public", false, false),
            "friendsPlus" => ("hidden", true, false),
            "friends" => ("friends", true, false),
            "invitePlus" => ("private", true, true),
            "invite" => ("private", true, false),
            _ => return Err(AppError::CannotDo("That is not a kind of instance.".into())),
        };
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        self.api.create_instance(world_id, kind, region, mine.then_some(me.id.as_str()), ask).await
    }

    /// Asks a friend to invite me to where they are.
    pub async fn request_invite(&self, user_id: &str) -> AppResult<()> {
        let is_friend = self.inner.lock().unwrap().friends.contains_key(user_id);
        if !is_friend {
            return Err(AppError::CannotDo("Only friends can be asked for an invite.".into()));
        }
        self.api.request_invite(user_id).await
    }

    fn keep_world(&self, world: World) {
        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(store) = &inner.store {
                let _ = store.save_world(&world);
            }
            inner.worlds.insert(world.id.clone(), world.clone());
        }
        self.send("world:resolved", json!(world));
    }

    /// Looks up world names one at a time, slowly. Each world is asked for once.
    async fn world_worker(self: Arc<Self>, mut rx: mpsc::UnboundedReceiver<Job>) {
        while let Some(job) = rx.recv().await {
            match job {
                Job::World(id) => match self.api.world(&id).await {
                    Ok(json) => {
                        if let Some(world) = World::from_json(&json) {
                            self.keep_world(world);
                        }
                    }
                    Err(AppError::RateLimited) => {
                        tokio::time::sleep(Duration::from_secs(120)).await;
                        self.inner.lock().unwrap().wanted.remove(&id);
                    }
                    Err(_) => {}
                },
                Job::Avatar(id) => match self.api.read(&format!("/avatars/{}", utf8(&id))).await {
                    Ok(a) => {
                        let t = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                        let image = if t("thumbnailImageUrl").is_empty() { t("imageUrl") } else { t("thumbnailImageUrl") };
                        let _ = self.with_store(|s| {
                            s.fill_avatar(&id, &t("name"), &t("authorId"), &t("authorName"), &image, &t("releaseStatus"))
                        });
                        self.send("avatars:changed", Value::Null);
                    }
                    Err(AppError::RateLimited) => tokio::time::sleep(Duration::from_secs(120)).await,
                    Err(_) => {}
                },
                Job::Instance(location) => {
                    let result = self.api.instance(&location).await;
                    let limited = matches!(result, Err(AppError::RateLimited));
                    let found = result.ok().and_then(|json| Instance::from_json(&location, &json));
                    {
                        let mut inner = self.inner.lock().unwrap();
                        inner.wanted.remove(&format!("count:{location}"));
                        match &found {
                            Some(instance) => {
                                inner
                                    .instances
                                    .insert(location.clone(), (instance.clone(), Instant::now()));
                            }
                            None if !limited => {
                                inner.refused.insert(location.clone(), Instant::now());
                            }
                            None => {}
                        }
                    }
                    if let Some(instance) = found {
                        self.send("instance:info", json!(instance));
                    }
                    if limited {
                        tokio::time::sleep(Duration::from_secs(120)).await;
                    }
                }
            }
            tokio::time::sleep(WORLD_GAP).await;
        }
    }

    async fn listen(&self) -> Ended {
        let Some(token) = self.api.auth_token() else {
            return Ended::SessionOver;
        };
        let Ok(mut request) = format!("{PIPELINE_URL}?authToken={token}").into_client_request()
        else {
            return Ended::Dropped;
        };
        if let Ok(agent) = HeaderValue::from_str(&user_agent()) {
            request.headers_mut().insert("User-Agent", agent);
        }
        let Ok((mut socket, _)) = tokio_tungstenite::connect_async(request).await else {
            return Ended::Dropped;
        };
        self.set_pipeline("live");
        let mut ping = tokio::time::interval(PING_EVERY);
        ping.tick().await;
        loop {
            let next = tokio::select! {
                next = tokio::time::timeout(QUIET_LIMIT, socket.next()) => next,
                _ = ping.tick() => {
                    // A connection that silently died fails here, or stays quiet until QUIET_LIMIT.
                    if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                        return Ended::Dropped;
                    }
                    continue;
                }
            };
            let message = match next {
                Ok(Some(Ok(m))) => m,
                _ => return Ended::Dropped,
            };
            match message {
                Message::Text(text) => {
                    if self.handle_frame(text.as_str()) {
                        // The server refused the socket. Find out if the sign-in is gone.
                        return match self.api.current_user().await {
                            Err(AppError::Unauthorized) | Err(AppError::TwoFactorRequired) => {
                                Ended::SessionOver
                            }
                            _ => Ended::Dropped,
                        };
                    }
                }
                Message::Close(_) => return Ended::Dropped,
                _ => {}
            }
        }
    }

    /// Applies one websocket frame. Returns true when the frame was a server error.
    pub fn handle_frame(&self, raw: &str) -> bool {
        let Some((kind, content)) = events::read_frame(raw) else {
            return false;
        };
        if kind == "error" {
            return true;
        }
        if kind == "user-update" {
            let user = content.get("user").unwrap_or(&content);
            if let Some(id) = user.get("currentAvatar").and_then(Value::as_str) {
                self.avatar_changed(id);
            }
            self.own_status_changed(user);
            return false;
        }
        if kind.contains("notification") {
            let outcome = notices::apply(&mut self.inner.lock().unwrap().notices, &kind, &content);
            if let Some(notice) = &outcome.arrived {
                self.announce(notice);
            }
            self.log_cleared(&outcome.removed);
            if outcome.changed {
                self.send("notices:changed", Value::Null);
            }
            return false;
        }
        let mut outgoing: Vec<(&'static str, Value)> = Vec::new();
        let mut want: Vec<String> = Vec::new();
        let mut counts: Vec<String> = Vec::new();
        let mut seen: Vec<World> = Vec::new();
        {
            let mut guard = self.inner.lock().unwrap();
            let inner = &mut *guard;
            let effects = events::apply(&mut inner.friends, &kind, &content);
            for effect in effects {
                match effect {
                    Effect::FriendChanged(f) => {
                        if let Some(store) = &inner.store {
                            let _ = store.save_friend(&f);
                        }
                        want.extend(world_ids(&f));
                        if f.state == "online" && f.place.kind == "world" {
                            counts.push(f.location.clone());
                        }
                        outgoing.push(("friend:changed", json!(f)));
                    }
                    Effect::FriendRemoved(id) => {
                        if let Some(store) = &inner.store {
                            let _ = store.remove_friend(&id);
                        }
                        outgoing.push(("friend:removed", json!(id)));
                    }
                    Effect::Feed(draft) => {
                        if let Some(store) = &inner.store {
                            if let Ok(item) = store.add_feed(&draft) {
                                outgoing.push(("feed:new", json!(item)));
                            }
                        }
                    }
                    Effect::WorldSeen(world) => seen.push(world),
                }
            }
        }
        for world in seen {
            self.keep_world(world);
        }
        for id in want {
            self.want_world(&id);
        }
        for location in counts {
            self.want_count(&location);
        }
        for (name, payload) in outgoing {
            self.send(name, payload);
        }
        false
    }
}

fn world_ids(f: &Friend) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(id) = &f.place.world_id {
        out.push(id.clone());
    }
    if let Some(id) = parse_location(&f.traveling_to).world_id {
        out.push(id);
    }
    out
}

/// VRChat names an avatar's picture file "Avatar - <name> - Image - <unity version>...".
/// Gives the name back, or nothing when the file is something else.
pub fn avatar_name_from_file(file_name: &str) -> Option<String> {
    let rest = file_name.strip_prefix("Avatar - ")?;
    // Most files end in " - Image - <version>"; a few older ones stop after the name.
    let end = rest.rfind(" - Image").unwrap_or(rest.len());
    let name = rest[..end].trim();
    (!name.is_empty()).then(|| name.to_string())
}

#[cfg(test)]
mod file_name_tests {
    use super::avatar_name_from_file;

    #[test]
    fn avatar_name_is_read_from_its_picture_file() {
        assert_eq!(avatar_name_from_file("Avatar - kaira - Image - 2022.3.22f1_4_2019.4.31f1_standalonewindows_Release").as_deref(), Some("kaira"));
        // A name with " - " in it keeps every part before the last " - Image".
        assert_eq!(avatar_name_from_file("Avatar - Sun - Set Fox - Image - 2022").as_deref(), Some("Sun - Set Fox"));
        assert!(avatar_name_from_file("World - Pier - Image - 2022").is_none());
        assert!(avatar_name_from_file("Avatar -  - Image - x").is_none());
    }
}
