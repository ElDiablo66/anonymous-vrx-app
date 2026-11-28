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
const COUNT_FRESH: Duration = Duration::from_secs(150);
const COUNT_STALE: Duration = Duration::from_secs(30);
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
/// A live connection that has sent nothing back for this long, not even a pong, is taken as dead.
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

/// The picture backgrounds VRChat offers for profiles, by the id a profile names them by. The
/// pictures are on VRChat's own server; VRChat publishes no list of them, so it is kept here.
pub const PROFILE_TEXTURES: &[&str] = &[
    "grid", "filigree", "bit-mountain", "approach", "planet-fall", "jungle", "light-streams", "koi", "moonlight", "machine", "topology",
    "circuit", "kawaii-cupcakes", "bokeh", "hologram", "flow", "jelly", "cheese", "alchemy", "sunset-mountain", "magic-rocks",
    "city-stillness", "music", "wolf-running", "dragon-smoke", "night-rain", "butterfly-zero", "i-am-speed", "ronin", "abduction",
    "cat-dream", "virtual-waves", "bolt-and-snail", "escape",
];

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
    /// The last favourites answer per kind, so screens can show at once and refresh after,
    /// with the account it belongs to. Another account's answer is never handed out.
    fav_cache: Mutex<HashMap<String, (String, Value)>>,
    /// The people I blocked or muted, with the account the list belongs to.
    blocked: Mutex<Option<(String, Value)>>,
    /// Picture, rank and status of people looked up for a list, kept while the app runs.
    briefs: Mutex<HashMap<String, (Instant, Value)>>,
    /// Calendar pages already asked for, by list and month, with when.
    calendars: Mutex<HashMap<String, (Instant, Value)>>,
    /// Every icon frame and profile effect VRChat offers, by id, with the time it was asked for.
    cosmetics: Mutex<Option<(Instant, Value)>>,
    /// Group id to its name, for lists where VRChat sends only the id.
    group_names: Mutex<HashMap<String, String>>,
    /// Full instances I am waiting to get into: location to the world's name and when the wait began.
    waiting: Mutex<HashMap<String, (String, Instant)>>,
    waiting_watched: std::sync::atomic::AtomicBool,
    /// A look at who of my friends are friends with each other is under way, or was told to stop.
    mutuals_running: std::sync::atomic::AtomicBool,
    /// VRChat's whole list of my choices about other people, kept a minute.
    moderation_raw: Mutex<Option<(String, Instant, Value)>>,
    /// When the app last set my status itself, what it was before and what it was set to.
    status_set: Mutex<Option<StatusSet>>,
    mutuals_stop: std::sync::atomic::AtomicBool,
    /// Put my own world, avatar and status changes into the feed too.
    own_feed: std::sync::atomic::AtomicBool,
    /// Notifications the app is answering or dismissing right now, by id.
    acting: Mutex<HashSet<String>>,
    /// A VRCX import is under way. It runs without the hub's main lock, so this is what keeps
    /// a second one from starting meanwhile and writing the same rows again.
    importing: std::sync::atomic::AtomicBool,
}

/// A status change the app made itself, kept for a few seconds so a late word from VRChat
/// about the status before it can be told apart from a real change made elsewhere.
struct StatusSet {
    at: Instant,
    old_status: String,
    old_text: String,
    new_status: String,
    new_text: String,
}

/// How long after setting my status a word carrying the old one is taken as late.
const STATUS_ECHO: Duration = Duration::from_secs(8);

/// Marks a notification as being acted on for as long as it is kept. Dropping it clears the
/// mark, on every way out.
struct Acting<'a> {
    set: &'a Mutex<HashSet<String>>,
    id: String,
}

impl<'a> Acting<'a> {
    fn start(set: &'a Mutex<HashSet<String>>, id: &str) -> Acting<'a> {
        set.lock().unwrap().insert(id.to_string());
        Acting { set, id: id.to_string() }
    }
}

impl Drop for Acting<'_> {
    fn drop(&mut self) {
        self.set.lock().unwrap().remove(&self.id);
    }
}

/// Marks a VRCX import as running for as long as it is kept. Dropping it clears the mark on
/// every way out, a run that failed or stopped half way included.
struct Importing<'a>(&'a std::sync::atomic::AtomicBool);

impl<'a> Importing<'a> {
    /// Takes the mark, or gives back nothing when another import already holds it.
    fn start(flag: &'a std::sync::atomic::AtomicBool) -> Option<Importing<'a>> {
        if flag.swap(true, std::sync::atomic::Ordering::AcqRel) {
            None
        } else {
            Some(Importing(flag))
        }
    }
}

impl Drop for Importing<'_> {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
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

/// A friend whose friendship day is coming up.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendDay {
    pub user_id: String,
    pub name: String,
    pub image: String,
    /// When the friend log first shows us as friends (UTC).
    pub since: String,
    pub years: i32,
    /// The day it falls on this year, YYYY-MM-DD in the PC's own calendar.
    pub on: String,
    /// 0 for today.
    pub in_days: i64,
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
    /// The first time the friend log shows us becoming friends; empty when it has no record
    /// or when we are not friends.
    pub friends_since: Option<String>,
    /// Their recent entries in my feed.
    pub feed: Vec<FeedItem>,
    /// Names this PC saw them use, each with when it was last seen.
    pub past_names: Vec<(String, String)>,
    /// The newer profile record arrived, so the bio and its links are the real ones. Without
    /// it nothing about them may be saved over.
    pub record_read: bool,
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

/// VRChat limits boops on its own: a boop to the same person can be held back until they answer
/// the last one, and many in a row are turned down for a while. Said plainly, with the wait when
/// VRChat gave one. The app does not send it again by itself.
fn boop_turned_down(e: AppError) -> AppError {
    match e {
        AppError::RateLimited(seconds) => AppError::CannotDo(format!(
            "VRChat did not take the boop. {}. It can also hold back a new boop to someone until they answer your last one.",
            AppError::RateLimited(seconds)
        )),
        other => other,
    }
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
            cosmetics: Mutex::new(None),
            group_names: Mutex::new(HashMap::new()),
            waiting: Mutex::new(HashMap::new()),
            waiting_watched: std::sync::atomic::AtomicBool::new(false),
            mutuals_running: std::sync::atomic::AtomicBool::new(false),
            status_set: Mutex::new(None),
            moderation_raw: Mutex::new(None),
            mutuals_stop: std::sync::atomic::AtomicBool::new(false),
            own_feed: std::sync::atomic::AtomicBool::new(false),
            acting: Mutex::new(HashSet::new()),
            importing: std::sync::atomic::AtomicBool::new(false),
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
            // Only my own account counts.
            if user.get("id").and_then(Value::as_str).is_some_and(|id| id != me.id) {
                return;
            }
            // Right after the app set my status, VRChat can still send word of the one
            // before it. That late word must not put the old status back in the window. Only
            // word that carries the old values is held back; anything else is a real change,
            // made in VRChat or on the website, and goes through.
            {
                let mut set = self.status_set.lock().unwrap();
                if let Some(s) = set.as_ref() {
                    let status = user.get("status").and_then(Value::as_str);
                    let text = user.get("statusDescription").and_then(Value::as_str).map(str::trim);
                    let carries = |want_status: &str, want_text: &str| {
                        (status.is_some() || text.is_some())
                            && status.is_none_or(|v| v == want_status)
                            && text.is_none_or(|v| v == want_text.trim())
                    };
                    if carries(&s.new_status, &s.new_text) {
                        // VRChat confirmed the change, so later word is news, whatever it says.
                        *set = None;
                    } else if carries(&s.old_status, &s.old_text) && s.at.elapsed() < STATUS_ECHO {
                        return;
                    }
                }
            }
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
        let told = status.is_some() || text.is_some();
        if let Some((now, before)) = status {
            self.own_entry("status", &now, &before, "");
        }
        if let Some((now, before)) = text {
            self.own_entry("statusText", &now, &before, "");
        }
        // The window keeps my status in step, whether I changed it here, in VRChat or on the website.
        if let Some(me) = self.me() {
            if told {
                self.send("me:status", json!({ "status": me.status, "statusDescription": me.status_description }));
            }
        }
    }

    pub fn last_seen(&self) -> AppResult<HashMap<String, String>> {
        self.with_store(|s| Ok(s.last_seen()?.into_iter().collect()))
    }

    pub fn ratings(&self) -> AppResult<HashMap<String, i64>> {
        self.with_store(|s| Ok(s.ratings()?.into_iter().collect()))
    }

    /// The full instances I am waiting for, longest wait first.
    pub fn waiting(&self) -> Value {
        let mut list: Vec<(String, String, Instant)> = self.waiting.lock().unwrap().iter().map(|(l, (w, at))| (l.clone(), w.clone(), *at)).collect();
        list.sort_by_key(|(_, _, at)| *at);
        Value::Array(list.into_iter().map(|(location, world, at)| json!({ "location": location, "worldName": world, "minutes": at.elapsed().as_secs() / 60 })).collect())
    }

    /// Starts waiting for room in a full instance. Three at a time at most, so the app never
    /// asks VRChat about many instances over and over.
    pub fn wait_for_room(self: &Arc<Self>, location: &str, world_name: &str) -> AppResult<()> {
        if self.me().is_none() {
            return Err(AppError::NotSignedIn);
        }
        if crate::vrchat::model::launch_link(location).is_none() {
            return Err(AppError::CannotDo("That is not an instance VRChat can be started in.".into()));
        }
        {
            let mut waiting = self.waiting.lock().unwrap();
            if !waiting.contains_key(location) && waiting.len() >= 3 {
                return Err(AppError::CannotDo("You are already waiting for three instances. Stop one of those first.".into()));
            }
            waiting.entry(location.to_string()).or_insert_with(|| (world_name.to_string(), Instant::now()));
        }
        self.send("queue:changed", Value::Null);
        // One watcher looks at all of them, and goes away when nothing is waited for.
        if !self.waiting_watched.swap(true, std::sync::atomic::Ordering::Relaxed) {
            let hub = self.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(45)).await;
                    if hub.look_for_room().await == 0 {
                        break;
                    }
                }
                hub.waiting_watched.store(false, std::sync::atomic::Ordering::Relaxed);
                // Something added in the instant before the watcher left gets a watcher of its own.
                let again = hub.waiting.lock().unwrap().iter().next().map(|(l, (w, _))| (l.clone(), w.clone()));
                if let Some((location, world)) = again {
                    let _ = hub.wait_for_room(&location, &world);
                }
            });
        }
        Ok(())
    }

    /// Stops waiting for one instance.
    pub fn stop_waiting(&self, location: &str) {
        if self.waiting.lock().unwrap().remove(location).is_some() {
            self.send("queue:changed", Value::Null);
        }
    }

    /// Asks VRChat once about every instance waited for. One with room is announced and
    /// dropped from the list; so is one that has closed, or has been waited for an hour.
    /// Gives back how many are still waited for.
    pub async fn look_for_room(&self) -> usize {
        let list: Vec<(String, String, Instant)> = self.waiting.lock().unwrap().iter().map(|(l, (w, at))| (l.clone(), w.clone(), *at)).collect();
        if self.me().is_none() {
            self.waiting.lock().unwrap().clear();
            self.send("queue:changed", Value::Null);
            return 0;
        }
        // A wait ends after an hour, before VRChat is asked again, whatever VRChat answered in
        // the rounds before. A wait that old never announces room.
        let (old, list): (Vec<_>, Vec<_>) = list.into_iter().partition(|(_, _, began)| began.elapsed() >= Duration::from_secs(3600));
        for (location, world, began) in old {
            let ended = {
                let mut waiting = self.waiting.lock().unwrap();
                // Only the same wait: one stopped and asked for again since is newer.
                let same = waiting.get(&location).is_some_and(|(_, at)| *at == began);
                same && waiting.remove(&location).is_some()
            };
            if ended {
                self.send("queue:gone", json!({ "location": location, "worldName": world, "why": "hour" }));
                self.send("queue:changed", Value::Null);
            }
        }
        for (location, world, _) in list {
            // Stopped while this round was under way.
            if !self.waiting.lock().unwrap().contains_key(&location) {
                continue;
            }
            let reply = self.api.instance(&location).await;
            // Stopped, or stopped and asked for again, while VRChat answered: the wait as it
            // stands now decides, and one that is gone gets no word at all.
            let Some(since) = self.waiting.lock().unwrap().get(&location).map(|(_, at)| *at) else {
                continue;
            };
            let gone = |why: &str| ("queue:gone", json!({ "location": location, "worldName": world, "why": why }));
            let done = match reply {
                Ok(i) => {
                    // The head count just read is news for the cards too, so the instance's card
                    // shows the free places as soon as the alarm sees them.
                    if let Some(instance) = Instance::from_json(&location, &i) {
                        self.inner.lock().unwrap().instances.insert(location.clone(), (instance.clone(), Instant::now()));
                        self.send("instance:info", json!(instance));
                    }
                    let users = crate::vrchat::model::head_count(&i);
                    let capacity = i["capacity"].as_i64().unwrap_or(0);
                    let closed = i["closedAt"].as_str().is_some_and(|c| !c.is_empty());
                    // Without a head count or VRChat's own word, nothing is known this round.
                    let full = match (i["full"].as_bool(), users) {
                        (Some(true), _) => Some(true),
                        (_, Some(n)) if capacity > 0 => Some(n >= capacity),
                        (Some(false), _) => Some(false),
                        _ => None,
                    };
                    if closed {
                        Some(gone("closed"))
                    } else if full == Some(false) {
                        let free = users.filter(|_| capacity > 0).map_or(1, |n| (capacity - n).max(1));
                        self.send("popup", json!({ "title": format!("Room in {world}"), "body": format!("{free} {} free. Open Anonymous VRX to join.", if free == 1 { "place is" } else { "places are" }), "kind": "room", "label": "Room free" }));
                        Some(("queue:room", json!({ "location": location, "worldName": world, "free": free })))
                    } else {
                        None
                    }
                }
                Err(AppError::Api { status: 403 | 404, .. }) => Some(gone("closed")),
                // Told to slow down, no connection, or signed out: the rest of this round is
                // skipped, the waits go on.
                Err(AppError::RateLimited(_) | AppError::Network(_) | AppError::Unauthorized | AppError::NotSignedIn) => break,
                // VRChat could not answer about this one instance: the others are still asked.
                Err(_) => None,
            };
            let done = done.or_else(|| (since.elapsed() >= Duration::from_secs(3600)).then(|| gone("hour")));
            if let Some((event, payload)) = done {
                self.waiting.lock().unwrap().remove(&location);
                self.send(event, payload);
                self.send("queue:changed", Value::Null);
            }
        }
        self.waiting.lock().unwrap().len()
    }

    /// Puts a friend on the list without asking VRChat. For tests.
    #[doc(hidden)]
    pub fn put_friend_for_test(&self, id: &str) {
        self.inner.lock().unwrap().friends.insert(id.to_string(), Friend { id: id.to_string(), ..Friend::default() });
    }

    /// What is known about which of my friends are friends with each other.
    pub fn mutuals(&self) -> AppResult<Value> {
        let (pairs, mut asked) = self.with_store(|s| Ok((s.mutuals()?, s.mutuals_asked()?)))?;
        // Only people who are still friends count; someone unfriended since would make a
        // stopped run look finished.
        {
            let inner = self.inner.lock().unwrap();
            asked.retain(|(id, _, _)| inner.friends.contains_key(id));
        }
        let newest = asked.iter().map(|(_, at, _)| at.as_str()).max().unwrap_or_default().to_string();
        Ok(json!({
            "pairs": pairs.into_iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>(),
            "asked": asked.len(),
            "hidden": asked.iter().filter(|(_, _, hidden)| *hidden).map(|(id, _, _)| id.clone()).collect::<Vec<_>>(),
            "at": newest,
            "running": self.mutuals_running.load(std::sync::atomic::Ordering::Relaxed),
        }))
    }

    /// Tells a look that is under way to end after the friend it is on.
    pub fn stop_mutuals(&self) {
        self.mutuals_stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Asks VRChat, one friend at a time, which of my other friends they are friends with.
    /// One request a friend (more for someone with over a hundred shared friends), with a
    /// pause between friends, and only when I press the button. `gap` is that pause.
    /// Gives back how many friends were asked and why it ended.
    ///
    /// Unless `everyone` is set, friends who answered within the last twelve hours are not
    /// asked again, so a run that was stopped picks up where it left off.
    pub async fn fetch_mutuals(&self, gap: Duration, everyone: bool) -> AppResult<Value> {
        use std::sync::atomic::Ordering;
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        if self.mutuals_running.swap(true, Ordering::Relaxed) {
            return Err(AppError::CannotDo("That is already under way.".into()));
        }
        self.mutuals_stop.store(false, Ordering::Relaxed);
        let mut ids: Vec<String> = self.inner.lock().unwrap().friends.keys().cloned().collect();
        ids.sort();
        if !everyone {
            let recent = (chrono::Utc::now() - chrono::Duration::hours(12)).format("%Y-%m-%dT%H:%M:%SZ").to_string();
            let fresh: std::collections::HashSet<String> = self
                .with_store(|s| s.mutuals_asked())
                .unwrap_or_default()
                .into_iter()
                .filter(|(_, at, hidden)| !hidden && at.as_str() >= recent.as_str())
                .map(|(id, _, _)| id)
                .collect();
            ids.retain(|id| !fresh.contains(id));
        }
        let total = ids.len();
        let mut done = 0;
        let mut ended = "done";
        let mut problem = String::new();
        for id in &ids {
            if self.mutuals_stop.load(Ordering::Relaxed) || self.me().is_none_or(|now| now.id != me.id) {
                ended = "stopped";
                break;
            }
            let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
            // The page size and offset are added by `read_all`; naming one here as well made
            // VRChat turn the question down for every friend.
            let answer = self.api.read_all(&format!("/users/{}/mutuals/friends", utf8(id))).await;
            // Signed out, or into another account, while VRChat answered: nothing may be
            // written into whichever history is open now.
            if self.me().is_none_or(|now| now.id != me.id) {
                ended = "stopped";
                break;
            }
            match answer {
                Ok(list) => {
                    let others: Vec<String> = list.iter().filter_map(|u| u["id"].as_str().map(str::to_string)).collect();
                    let _ = self.with_store(|s| s.set_mutuals(id, &others, &now, false));
                }
                Err(AppError::RateLimited(_)) => {
                    ended = "slow";
                    break;
                }
                Err(AppError::Unauthorized) => {
                    ended = "signedOut";
                    break;
                }
                // Someone who keeps their friends to themselves, or an account that is gone.
                Err(AppError::Api { status: 403 | 404, .. }) => {
                    let _ = self.with_store(|s| s.set_mutuals(id, &[], &now, true));
                }
                // Anything else VRChat says no to is a fault in the question, not in the friend:
                // stop, keep what is known, and say what came back.
                Err(AppError::Api { status, message }) => {
                    problem = format!("VRChat answered {status}: {message}");
                    ended = "refused";
                    break;
                }
                Err(other) => {
                    problem = other.to_string();
                    ended = "network";
                    break;
                }
            }
            done += 1;
            self.send("mutuals:progress", json!({ "done": done, "total": total }));
            tokio::time::sleep(gap).await;
        }
        self.mutuals_running.store(false, Ordering::Relaxed);
        // Heard by the window even when the view that started the run was closed meanwhile.
        self.send("mutuals:ended", Value::Null);
        Ok(json!({ "done": done, "total": total, "ended": ended, "problem": problem }))
    }

    /// Which of these people I often saw in the same instance: at least three stays together.
    pub fn pairs(&self, ids: &[String]) -> AppResult<Value> {
        let rows = self.with_store(|s| s.pairs(ids, 3, 40))?;
        Ok(Value::Array(rows.into_iter().map(|(a, b, times)| json!({ "a": a, "b": b, "times": times })).collect()))
    }

    /// People and worlds in my own history by name.
    pub fn log_find(&self, query: &str) -> AppResult<Value> {
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        let (people, worlds) = self.with_store(|s| s.log_find(query, &me.id))?;
        Ok(json!({ "people": people, "worlds": worlds }))
    }

    /// Every stay of mine with one person (`kind` "person") or in one world ("world").
    pub fn log_stays(&self, kind: &str, id: &str, name: &str) -> AppResult<Value> {
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        let stays = self.with_store(|s| if kind == "world" { s.stays_in(id, name, &me.id, 200) } else { s.stays_with(id, name, &me.id, 200) })?;
        Ok(json!(stays))
    }

    /// Where and when I meet one person, and where they go by themselves, over the last 90 days of friend activity.
    pub fn habits(&self, user_id: &str) -> AppResult<Value> {
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        let since = (chrono::Utc::now() - chrono::Duration::days(90)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let habits = self.with_store(|s| s.habits(user_id, &me.id, &since))?;
        // A world the app has no name for is looked up once, in the slow queue; the window
        // gets the name when it arrives.
        for world in habits.own_worlds.iter().chain(&habits.worlds) {
            if world.name.is_empty() && world.world_id.starts_with("wrld_") {
                self.want_world(&world.world_id);
            }
        }
        Ok(json!(habits))
    }

    /// What was noted about my own worlds over the last `days` days, oldest first.
    pub fn world_stats(&self, days: i64) -> AppResult<Value> {
        let since = (chrono::Local::now() - chrono::Duration::days(days.clamp(1, 730))).format("%Y-%m-%d").to_string();
        let rows = self.with_store(|s| s.world_stats(&since))?;
        Ok(Value::Array(
            rows.into_iter()
                .map(|(world_id, day, visits, favorites, occupants)| json!({ "worldId": world_id, "day": day, "visits": visits, "favorites": favorites, "occupants": occupants }))
                .collect(),
        ))
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
        // `me` was read before the change went out, so it holds the status VRChat had before.
        *self.status_set.lock().unwrap() = Some(StatusSet {
            at: Instant::now(),
            old_status: me.status.clone(),
            old_text: me.status_description.clone(),
            new_status: status.to_string(),
            new_text: text.to_string(),
        });
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
            let (kind, label) = notices::popup_kind(notice);
            self.send("popup", json!({ "title": title, "body": body, "kind": kind, "label": label }));
        }
    }

    /// Keeps notifications VRChat cleared on its own side in the Done list, so none vanish unseen.
    /// A notice the app is acting on right now is left out: the app logs it itself, with what
    /// was really done, once VRChat answers. `acting` is what the app was acting on when the
    /// notices were taken off the list, read under the same lock.
    fn log_cleared(&self, cleared: &[Notice], acting: &HashSet<String>) {
        let cleared: Vec<&Notice> = cleared.iter().filter(|n| !acting.contains(&n.id)).collect();
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
        backup::create(root, &copy, settings_json, label, &me.id)
    }

    /// The backups the signed-in account may restore, newest first: its own and the older
    /// untagged ones. With nobody signed in, every backup on this PC.
    pub fn list_backups(&self, root: &std::path::Path) -> Vec<BackupInfo> {
        match self.me() {
            Some(me) => backup::list_for(root, &me.id),
            None => backup::list(root),
        }
    }

    /// Brings the history VRCX kept for this account into this one. A backup is made first,
    /// named so it can be told apart, and VRCX's own file is only read.
    ///
    /// `by_itself` is the run the app makes when VRCX's file has changed. It backs up at most
    /// once a day and keeps three of those, so a PC with both apps open does not fill up.
    /// The icon frames, profile effects and nameplate effects people can wear, by the id a
    /// profile names. Three small lists, asked for at most once a day.
    pub async fn cosmetics(&self) -> AppResult<Value> {
        if self.me().is_none() {
            return Err(AppError::NotSignedIn);
        }
        if let Some((at, kept)) = &*self.cosmetics.lock().unwrap() {
            if at.elapsed() < Duration::from_secs(24 * 3600) {
                return Ok(kept.clone());
            }
        }
        let mut all = serde_json::Map::new();
        let mut answered = false;
        for kind in ["iconFrame", "profileEffect", "nameplateEffect", "profileBackground"] {
            let list = match self.api.read(&format!("/cosmetics/index/{kind}")).await {
                Ok(list) => list,
                Err(e @ AppError::RateLimited(_)) => return Err(e),
                // One list failing does not hide the others.
                Err(_) => continue,
            };
            answered = true;
            for item in list.as_array().map(|a| a.as_slice()).unwrap_or_default() {
                let Some(id) = item["id"].as_str().filter(|id| !id.is_empty()) else {
                    continue;
                };
                let meta = &item["metadata"];
                let asset = |kind: &str| {
                    meta["assets"]
                        .as_array()
                        .and_then(|a| a.iter().find(|x| x["type"] == kind))
                        .and_then(|x| x["url"].as_str())
                        .filter(|u| u.starts_with("https://"))
                        .unwrap_or_default()
                        .to_string()
                };
                let plain = |v: &Value| v.as_str().filter(|u| u.starts_with("https://")).unwrap_or_default().to_string();
                let still = [asset("base"), plain(&meta["imageUrl"]), plain(&item["imageUrl"])].into_iter().find(|u| !u.is_empty()).unwrap_or_default();
                let moving = asset("mainAnimation");
                all.insert(
                    id.to_string(),
                    json!({
                        "picture": if moving.is_empty() { still.clone() } else { moving },
                        "still": still,
                        "from": meta["gradientStart"].as_str().unwrap_or_default(),
                        "to": meta["gradientEnd"].as_str().unwrap_or_default(),
                        "name": item["name"].as_str().unwrap_or_default(),
                        "kind": kind,
                    }),
                );
            }
        }
        let out = Value::Object(all);
        // Nothing is kept when VRChat answered none of the lists, so the next profile asks again.
        if answered {
            *self.cosmetics.lock().unwrap() = Some((Instant::now(), out.clone()));
        }
        Ok(out)
    }

    /// Brings VRCX's history for this account over. See `Store::import_vrcx`.
    pub fn import_vrcx(&self, other: &std::path::Path, backups: &std::path::Path, settings_json: &str, by_itself: bool, since: Option<&str>) -> AppResult<Value> {
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        if !other.is_file() {
            return Err(AppError::CannotDo("VRCX's history file was not found on this PC.".into()));
        }
        // Only one import runs at a time, from the backup to the last write. A run started from
        // Settings while the hourly look is going, or the other way round, is turned away here;
        // the window says nothing when its own hourly look is the one turned away.
        let _running = Importing::start(&self.importing)
            .ok_or_else(|| AppError::CannotDo("VRCX history is already being brought over. Wait for it to finish.".into()))?;
        const QUIET_LABEL: &str = "before taking over VRCX news";
        let saved = if !by_itself {
            self.backup(backups, settings_json, "before bringing over VRCX")?.name
        } else if backup::due_for(backups, QUIET_LABEL, &me.id, std::time::Duration::from_secs(24 * 3600)) {
            let made = self.backup(backups, settings_json, QUIET_LABEL)?;
            backup::prune_for(backups, QUIET_LABEL, &me.id, 3);
            made.name
        } else {
            String::new()
        };
        // The import gets its own connection to the history file instead of the one the hub
        // keeps, so the hub's lock is held only for this look and the rest of the app keeps
        // reading and writing while a long history comes over. The file keeps a write-ahead
        // log, so readers are never held up, and the import writes in short pieces.
        let path = {
            let inner = self.inner.lock().unwrap();
            if inner.store.is_none() || inner.me.as_ref().map(|m| m.id.as_str()) != Some(me.id.as_str()) {
                return Err(AppError::NotSignedIn);
            }
            self.history_path(&me.id)
        };
        let got = Store::open(&path)?.import_vrcx(other, &me.id, by_itself, since.filter(|_| by_itself))?;
        // Lists the window shows are read from the history again when it hears this.
        self.send("here:changed", Value::Null);
        Ok(json!({ "broughtOver": got, "backup": saved }))
    }

    /// Makes an automatic backup when the last one is older than a day, then keeps only the
    /// newest `keep`. Gives back the new backup, or nothing when none was due.
    pub fn auto_backup(&self, root: &std::path::Path, settings_json: &str, keep: usize) -> AppResult<Option<BackupInfo>> {
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        // Each account on this PC counts and keeps only its own automatic backups.
        if !backup::auto_due(root, &me.id, std::time::Duration::from_secs(24 * 3600)) {
            return Ok(None);
        }
        let made = self.backup(root, settings_json, backup::AUTO_LABEL)?;
        backup::prune_auto(root, &me.id, keep.clamp(1, 60));
        Ok(Some(made))
    }

    /// Saves the current data first, then lines the chosen backup up for the next start.
    /// Returns the backup's window settings. A backup made for another account is refused
    /// before anything is saved or replaced.
    pub fn restore_backup(&self, root: &std::path::Path, name: &str, current_settings: &str) -> AppResult<String> {
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        backup::check_restore(root, name, &me.id)?;
        self.backup(root, current_settings, "before restore")?;
        backup::stage_restore(root, name, &self.history_path(&me.id), &me.id)
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
        self.sync_favorites_read().await.map(|_| ())
    }

    /// The work of `sync_favorites`. Gives back the favourite friend entries and, when they
    /// were asked for, the favourite groups, so the read-ahead after signing in does not ask
    /// for them again.
    async fn sync_favorites_read(&self) -> AppResult<(Vec<Value>, Option<Vec<Value>>)> {
        let list = self.api.favorite_friends().await?;
        let groups = if list.is_empty() {
            None
        } else {
            Some(self.api.favorite_groups().await?)
        };
        let favorites = Favorites::from_api(&list, groups.as_deref().unwrap_or_default());
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
        Ok((list, groups))
    }

    /// One person's profile from VRChat, with what this PC knows about them.
    pub async fn profile(&self, user_id: &str) -> AppResult<Profile> {
        // My own account has more to show (my settings, my avatar), and only I can read it.
        let mine = self.me().is_some_and(|me| me.id == user_id);
        // My own record is asked for as myself, so a bio I just saved is read back fresh.
        let profile_path = if mine { format!("/profile/{}?asSelf=true", utf8(user_id)) } else { format!("/profile/{}", utf8(user_id)) };
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
        let mut record_read = false;
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
                // The record is the only place the bio and its links are kept now, so it wins
                // over any older copy, also when it is empty or has fewer links.
                target.insert("bio".into(), public.get("bio").filter(|v| v.is_string()).cloned().unwrap_or_else(|| json!("")));
                target.insert("bioLinks".into(), public.get("bioLinks").filter(|v| v.is_array()).cloned().unwrap_or_else(|| json!([])));
                record_read = true;
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
            friends_since: if inner.friends.contains_key(user_id) { store.friends_since(user_id)? } else { None },
            feed: store.feed_for(user_id, 40)?,
            past_names: store.past_names(user_id).unwrap_or_default(),
            person,
            raw: json,
            record_read,
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
                let list = self.api.read("/worlds?user=me&releaseStatus=all&n=100&sort=updated&order=descending").await?;
                // Each look at my worlds is also a note of how they stand today.
                let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                let rows: Vec<(String, i64, i64, i64)> = list
                    .as_array()
                    .map(|a| a.as_slice())
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|w| {
                        let n = |k: &str| w[k].as_i64().unwrap_or(0);
                        Some((w["id"].as_str()?.to_string(), n("visits"), n("favorites"), n("occupants")))
                    })
                    .collect();
                if !rows.is_empty() {
                    let _ = self.with_store(|s| s.record_world_stats(&today, &rows));
                }
                Ok(list)
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
            "instance" if raw_id.starts_with("wrld_") || raw_id.starts_with("wld_") => {
                let json = self.api.instance(raw_id).await?;
                // The head count just read is news for the cards too.
                if let Some(instance) = Instance::from_json(raw_id, &json) {
                    self.inner.lock().unwrap().instances.insert(raw_id.to_string(), (instance.clone(), Instant::now()));
                    self.send("instance:info", json!(instance));
                }
                Ok(json)
            }
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
        let groups = groups?;
        let mut counts = serde_json::Map::new();
        for (kind, entries) in [("friend", friends), ("avatar", avatars), ("world", worlds)] {
            // A list that could not be read is not taken as empty: every group of its kind
            // would then show no favourites at all. The answer fails instead, and the window
            // keeps the counts it showed last.
            for entry in entries? {
                let Some(group) = entry.pointer("/tags/0").and_then(Value::as_str) else { continue };
                let key = format!("{kind}:{group}");
                let had = counts.get(&key).and_then(Value::as_u64).unwrap_or(0);
                counts.insert(key, json!(had + 1));
            }
        }
        Ok(json!({ "groups": groups, "counts": counts }))
    }

    pub async fn favorites_of(&self, kind: &str) -> AppResult<Value> {
        if !matches!(kind, "world" | "avatar" | "friend") {
            return Err(AppError::CannotDo("Favourites are worlds, avatars or friends.".into()));
        }
        // The answer is kept only for the account it was read for.
        let owner = self.me().map(|m| m.id).unwrap_or_default();
        let entries_path = format!("/favorites?type={kind}");
        let (groups, entries, limits) = tokio::join!(
            self.api.favorite_groups(),
            self.api.read_all(&entries_path),
            self.api.read("/auth/user/favoritelimits"),
        );
        let (groups, entries) = (groups?, entries?);
        Ok(self.build_favorites(kind, &owner, &groups, entries, limits.ok().as_ref()).await)
    }

    /// Reads my avatar and world favourites ahead after signing in, so their screens open at
    /// once. The favourite friends and the group list the sign-in sync just read are used
    /// again, and the groups and limits are asked for once for all three kinds, so no list is
    /// asked for twice.
    async fn read_ahead_favorites(&self, read: Option<(Vec<Value>, Option<Vec<Value>>)>) {
        let Some(owner) = self.me().map(|m| m.id) else { return };
        let known_groups = read.as_ref().and_then(|(_, groups)| groups.clone());
        let (groups, limits, avatars, worlds) = tokio::join!(
            async {
                match known_groups {
                    Some(groups) => Ok(groups),
                    None => self.api.favorite_groups().await,
                }
            },
            self.api.read("/auth/user/favoritelimits"),
            self.api.read_all("/favorites?type=avatar"),
            self.api.read_all("/favorites?type=world"),
        );
        let Ok(groups) = groups else { return };
        let limits = limits.ok();
        let avatar = async {
            if let Ok(entries) = avatars {
                self.build_favorites("avatar", &owner, &groups, entries, limits.as_ref()).await;
            }
        };
        let world = async {
            if let Ok(entries) = worlds {
                self.build_favorites("world", &owner, &groups, entries, limits.as_ref()).await;
            }
        };
        tokio::join!(avatar, world);
        if let Some((friends, _)) = read {
            self.build_favorites("friend", &owner, &groups, friends, limits.as_ref()).await;
        }
    }

    /// Puts one kind of favourites together from the groups, entries and limits already read,
    /// asks VRChat only for the items in each group in use, and keeps the answer for `owner`.
    async fn build_favorites(&self, kind: &str, owner: &str, all_groups: &[Value], entries: Vec<Value>, limits: Option<&Value>) -> Value {
        let groups: Vec<Value> = all_groups
            .iter()
            .filter(|g| g.get("type").and_then(Value::as_str) == Some(kind))
            .cloned()
            .collect();
        let fallback = match kind {
            "world" => 100,
            "friend" => 150,
            _ => 50,
        };
        let capacity = limits
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
        self.cache_favorites(kind, owner, &all);
        all
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
                Err(AppError::RateLimited(_)) => {
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

    /// The favourites answer from last time, if there is one and it belongs to the account
    /// signed in now.
    pub fn favorites_cached(&self, kind: &str) -> Option<Value> {
        let current = self.me().map(|m| m.id).unwrap_or_default();
        self.fav_cache.lock().unwrap().get(kind).filter(|(owner, _)| *owner == current).map(|(_, v)| v.clone())
    }

    /// Keeps a favourites answer for later, but only while the account it was read for is
    /// still the one signed in. A read that finished after signing out, or after another
    /// account signed in, is dropped.
    fn cache_favorites(&self, kind: &str, owner: &str, all: &Value) {
        let current = self.me().map(|m| m.id).unwrap_or_default();
        if current == owner {
            self.fav_cache.lock().unwrap().insert(kind.to_string(), (owner.to_string(), all.clone()));
        }
    }

    pub async fn boop(&self, user_id: &str, emoji: Option<&str>) -> AppResult<()> {
        if !user_id.starts_with("usr_") {
            return Err(AppError::CannotDo("Only a person can be booped.".into()));
        }
        // One of VRChat's own emoji, or a file of mine there. Nothing else is passed on.
        let plain = |id: &str| id.len() <= 64 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
        let emoji = emoji.map(str::trim).filter(|e| !e.is_empty());
        if emoji.is_some_and(|e| !((e.starts_with("default_") || e.starts_with("file_")) && plain(e))) {
            return Err(AppError::CannotDo("That is not an emoji VRChat knows.".into()));
        }
        self.api.boop(user_id, emoji).await.map_err(boop_turned_down)
    }

    /// My saved messages for invites (`message`) or for asking to be invited (`request`), twelve of each.
    pub async fn invite_messages(&self, kind: &str) -> AppResult<Value> {
        if !matches!(kind, "message" | "request") {
            return Err(AppError::CannotDo("That is not a kind of saved message.".into()));
        }
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        self.api.invite_messages(&me.id, kind).await
    }

    /// Changes one saved message. VRChat allows that once an hour for each slot.
    pub async fn set_invite_message(&self, kind: &str, slot: u8, text: &str) -> AppResult<Value> {
        if !matches!(kind, "message" | "request") {
            return Err(AppError::CannotDo("That is not a kind of saved message.".into()));
        }
        let text = text.trim();
        if slot > 11 || text.is_empty() || text.chars().count() > 64 || text.chars().any(char::is_control) {
            return Err(AppError::CannotDo("A saved message is one line of 1 to 64 characters, in one of twelve slots.".into()));
        }
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        self.api.set_invite_message(&me.id, kind, slot, text).await
    }

    /// Changes what my profile says about me. `about` is the bio with up to three links, sent
    /// together; `pronouns` goes only when it changed. Either can be left out.
    ///
    /// VRChat keeps the bio and its links in the newer profile record, so they go to
    /// `PUT /profile/{me}`. Since September 2026 `PUT /users/{me}` answers yes to a bio and keeps
    /// nothing, which is how a bio could look saved and stay the same. Pronouns still live on the
    /// account, `PUT /users/{me}`. VRChat's answer is checked, and when it does not show the bio
    /// the record is read once more: a bio VRChat did not keep is never reported as saved.
    pub async fn update_profile(&self, about: Option<(&str, &[String])>, pronouns: Option<&str>) -> AppResult<()> {
        let web = |l: &str| (l.starts_with("https://") || l.starts_with("http://")) && l.len() <= 200 && !l.chars().any(|c| c.is_control() || c.is_whitespace());
        let about = match about {
            None => None,
            Some((bio, links)) => {
                let links: Vec<&str> = links.iter().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
                if bio.chars().count() > 512 {
                    return Err(AppError::CannotDo("A bio can be at most 512 characters.".into()));
                }
                if bio.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
                    return Err(AppError::CannotDo("The bio has a character in it that cannot be shown. Type it again without pasting.".into()));
                }
                if links.len() > 3 || links.iter().any(|l| !web(l)) {
                    return Err(AppError::CannotDo("A profile holds up to three links, each a web address starting with https://.".into()));
                }
                Some((bio, links))
            }
        };
        if let Some(p) = pronouns {
            if p.chars().count() > 32 || p.chars().any(char::is_control) {
                return Err(AppError::CannotDo("Pronouns can be at most 32 characters.".into()));
            }
        }
        if about.is_none() && pronouns.is_none() {
            return Ok(());
        }
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        if let Some((bio, links)) = about {
            let answer = self.api.update_profile_record(&me.id, json!({ "bio": bio, "bioLinks": links })).await?;
            // Line ends and the spaces at either end may come back tidied.
            let tidy = |s: &str| s.replace("\r\n", "\n").trim().to_string();
            let kept_in = |record: &Value| record.as_object().and_then(|o| o.get("bio")).map(|b| tidy(b.as_str().unwrap_or("")) == tidy(bio));
            let kept = match kept_in(&answer) {
                Some(kept) => kept,
                None => {
                    let again = self.api.read(&format!("/profile/{}?asSelf=true", utf8(&me.id))).await?;
                    // A record without a bio has an empty one.
                    again.is_object() && tidy(again.get("bio").and_then(Value::as_str).unwrap_or("")) == tidy(bio)
                }
            };
            if !kept {
                return Err(AppError::CannotDo("VRChat answered but did not keep the new bio. Try again in a minute.".into()));
            }
        }
        if let Some(p) = pronouns {
            self.api.update_me(&me.id, json!({ "pronouns": p.trim() })).await?;
        }
        Ok(())
    }

    pub async fn picture(&self, url: &str) -> AppResult<(String, Vec<u8>)> {
        self.api.picture_as(url, true).await
    }

    /// The frames, effects and backgrounds I own, by kind, as the ids profiles name them by.
    pub async fn my_cosmetics(&self) -> AppResult<Value> {
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        let list = self.api.read(&format!("/user/{}/cosmetics", utf8(&me.id))).await?;
        let mut out = serde_json::Map::new();
        for item in list.as_array().map(|a| a.as_slice()).unwrap_or_default() {
            let (Some(kind), Some(id)) = (item["itemType"].as_str(), item["templateId"].as_str()) else { continue };
            let entry = out.entry(kind.to_string()).or_insert_with(|| Value::Array(Vec::new()));
            if let Some(ids) = entry.as_array_mut() {
                if !ids.iter().any(|x| x == id) {
                    ids.push(json!(id));
                }
            }
        }
        Ok(Value::Object(out))
    }

    /// Changes how my profile looks: my languages, the frame round my picture, the effect over my
    /// banner, the plate behind my name and the background. Only the keys given change, and each
    /// is checked before it goes to VRChat.
    pub async fn update_look(&self, changes: &Value) -> AppResult<()> {
        let Some(changes) = changes.as_object() else {
            return Err(AppError::CannotDo("Nothing to change.".into()));
        };
        let template = |v: &Value| match v.as_str() {
            Some("") => true,
            Some(id) => id.len() == 41 && id.starts_with("invt_") && id[5..].chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
            None => false,
        };
        let colour = |v: &Value| v.as_str().is_some_and(|c| c.len() == 6 && c.chars().all(|x| x.is_ascii_hexdigit()));
        let mut body = serde_json::Map::new();
        for (key, value) in changes {
            let fine = match key.as_str() {
                "iconFrame" | "profileEffect" | "nameplateEffect" => template(value),
                "backgroundType" => matches!(value.as_str(), Some("default" | "gradient" | "texture")),
                "backgroundGradientTop" | "backgroundGradientBottom" => colour(value),
                "backgroundTextureId" => value.as_str().is_some_and(|id| PROFILE_TEXTURES.contains(&id)),
                _ => false,
            };
            if !fine {
                return Err(AppError::CannotDo(format!("The {key} given is not one VRChat takes.")));
            }
            body.insert(key.clone(), value.clone());
        }
        if body.is_empty() {
            return Ok(());
        }
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        self.api.update_profile_record(&me.id, Value::Object(body)).await.map(|_| ())
    }

    /// Changes the languages on my profile. VRChat keeps them as tags on the account
    /// (`language_eng`): the new ones go on first, then the ones dropped come off. In that order
    /// a held or refused add leaves the profile as it was, never with fewer languages than before.
    pub async fn update_languages(&self, add: &[String], remove: &[String]) -> AppResult<()> {
        let code = |c: &String| c.len() == 3 && c.chars().all(|x| x.is_ascii_lowercase());
        if add.len() > 3 || remove.len() > 10 || !add.iter().chain(remove).all(code) {
            return Err(AppError::CannotDo("A profile names up to three languages.".into()));
        }
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        let tags = |list: &[String]| list.iter().map(|c| format!("language_{c}")).collect::<Vec<_>>();
        // A language both added and dropped stays on.
        let remove: Vec<String> = remove.iter().filter(|c| !add.contains(c)).cloned().collect();
        if !add.is_empty() {
            self.api.change_tags(&me.id, true, &tags(add)).await?;
        }
        if !remove.is_empty() {
            if let Err(e) = self.api.change_tags(&me.id, false, &tags(&remove)).await {
                if add.is_empty() {
                    return Err(e);
                }
                // Nothing more is sent here, after a wait or not: the profile has every language
                // it had plus the new ones, and the next save takes the old ones off.
                return Err(AppError::CannotDo(format!(
                    "The new languages were added, but the old ones could not be taken off ({e}). Save again to take them off."
                )));
            }
        }
        Ok(())
    }

    /// Everything I set about one other person: block, mute, chatbox mute, their avatar shown or
    /// hidden, interaction off. Read from VRChat's list, kept a minute.
    pub async fn moderations_for(&self, user_id: &str) -> AppResult<Value> {
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        let kept = self.moderation_raw.lock().unwrap().as_ref().filter(|(owner, at, _)| *owner == me.id && at.elapsed() < Duration::from_secs(60)).map(|(_, _, v)| v.clone());
        let all = match kept {
            Some(v) => v,
            None => {
                let v = self.api.player_moderations().await?;
                *self.moderation_raw.lock().unwrap() = Some((me.id.clone(), Instant::now(), v.clone()));
                v
            }
        };
        let kinds: Vec<Value> = all
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or_default()
            .iter()
            .filter(|m| m["targetUserId"].as_str() == Some(user_id))
            .filter_map(|m| m["type"].as_str().map(|k| json!(k)))
            .collect();
        Ok(Value::Array(kinds))
    }

    /// My groups, to pick one to invite someone into. VRChat decides whether my role may invite.
    pub async fn my_groups(&self) -> AppResult<Value> {
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        self.api.read(&format!("/users/{}/groups", utf8(&me.id))).await
    }

    pub async fn invite_to_group(&self, group_id: &str, user_id: &str) -> AppResult<()> {
        let plain = |id: &str, head: &str| id.starts_with(head) && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !plain(group_id, "grp_") || !plain(user_id, "usr_") {
            return Err(AppError::CannotDo("That is not a group and a person VRChat knows.".into()));
        }
        if self.me().is_some_and(|me| me.id == user_id) {
            return Err(AppError::CannotDo("You cannot invite yourself.".into()));
        }
        self.api.invite_to_group(group_id, user_id).await
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
            // On now and coming up across VRChat. It has no months.
            "discover" => "discover",
            _ => return Err(AppError::CannotDo("That is not a calendar this app knows.".into())),
        };
        let everywhere = which == "discover";
        let day = if everywhere {
            chrono::Utc::now().date_naive()
        } else {
            chrono::NaiveDate::parse_from_str(&format!("{month}-15"), "%Y-%m-%d").map_err(|_| AppError::CannotDo("That is not a month.".into()))?
        };
        if self.me().is_none() {
            return Err(AppError::NotSignedIn);
        }
        let key = if everywhere { "discover".to_string() } else { format!("{which}:{month}") };
        if !fresh {
            if let Some((at, known)) = self.calendars.lock().unwrap().get(&key) {
                if at.elapsed() < Duration::from_secs(600) {
                    return Ok(known.clone());
                }
            }
        }
        let answer = if everywhere {
            self.api.discover_events().await?
        } else {
            self.api.calendar(list, &format!("{}T12:00:00.000Z", day.format("%Y-%m-%d"))).await?
        };
        // The calendar names an event's group only by id. Each group is asked for its name
        // once, a few per look at the calendar; the rest follow on the next look.
        let mut unnamed: Vec<String> = Vec::new();
        for e in answer["results"].as_array().map(|a| a.as_slice()).unwrap_or_default() {
            let id = e["ownerId"].as_str().unwrap_or_default();
            let named = e["ownerName"].as_str().is_some_and(|n| !n.is_empty());
            if !named && id.starts_with("grp_") && !unnamed.iter().any(|u| u == id) && !self.group_names.lock().unwrap().contains_key(id) {
                unnamed.push(id.to_string());
            }
        }
        for id in unnamed.into_iter().take(8) {
            match self.api.read(&format!("/groups/{}", utf8(&id))).await {
                Ok(group) => {
                    let name = group["name"].as_str().unwrap_or_default().to_string();
                    self.group_names.lock().unwrap().insert(id, name);
                }
                Err(AppError::RateLimited(_)) => break,
                Err(_) => {
                    // A group that cannot be read is not asked about again this session.
                    self.group_names.lock().unwrap().insert(id, String::new());
                }
            }
        }
        let names = self.group_names.lock().unwrap().clone();
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
                    "groupName": e["ownerName"].as_str().filter(|n| !n.is_empty()).map(str::to_string).or_else(|| names.get(e["ownerId"].as_str().unwrap_or_default()).cloned()).unwrap_or_default(),
                    "title": e["title"],
                    "description": e["description"],
                    "category": e["category"],
                    "access": e["accessType"],
                    "startsAt": e["startsAt"],
                    "endsAt": e["endsAt"],
                    "image": e["imageUrl"],
                    "interested": e["interestedUserCount"],
                    "friends": e["interestedFriendCount"],
                    // Everything on the list of events I follow is followed, whatever else the answer says.
                    "following": which == "following" || e["userInterest"]["isFollowing"].as_bool().unwrap_or(false),
                    "featured": e["featured"].as_bool().unwrap_or(false),
                })
            })
            .collect();
        let more = answer["hasNext"].as_bool().unwrap_or(false) || answer["nextCursor"].as_str().is_some_and(|c| !c.is_empty());
        let out = json!({ "events": events, "more": more });
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
        if !matches!(kind, "block" | "mute" | "muteChat" | "hideAvatar" | "showAvatar" | "interactOff") {
            return Err(AppError::CannotDo("That is not a choice VRChat has.".into()));
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
        *self.moderation_raw.lock().unwrap() = None;
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
                Err(AppError::RateLimited(_)) => break,
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
    pub async fn invite_here(&self, user_id: &str, slot: Option<u8>) -> AppResult<()> {
        if slot.is_some_and(|s| s > 11) {
            return Err(AppError::CannotDo("There are twelve saved messages, 1 to 12.".into()));
        }
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
        self.api.invite_user(user_id, &location, slot).await
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
        let v2 = self.api.notifications_v2().await;
        let mut fresh = notices::from_api(&v1, v2.as_deref().unwrap_or_default());
        let new_ones: Vec<Notice> = {
            let mut inner = self.inner.lock().unwrap();
            // The newer kind could not be read: the ones already listed stay, so they neither
            // vanish from the Inbox nor come back later as if they were new.
            if v2.is_err() {
                notices::keep_newer_kind(&mut fresh, &inner.notices);
            }
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
        // VRChat's live word that the notice was cleared often arrives before the answer to
        // this request. While the request is under way, that word must not log it a second
        // time as handled elsewhere.
        //
        // The guard ends only inside the lock that also logs and removes the notice. A frame
        // takes the notice off the list and reads the guards under that same lock, so it either
        // finds the guard still there or finds the notice already gone, never neither.
        // The guard is set while the list is held, so the notice is either still there and
        // guarded, or already cleared and logged by the frame that cleared it.
        let acting = {
            let inner = self.inner.lock().unwrap();
            if inner.notices.iter().all(|n| n.id != id) {
                return Err(AppError::CannotDo("That notification is already gone.".into()));
            }
            Acting::start(&self.acting, id)
        };
        let done = self.carry_out(&notice, id, action, reply_kind, reply_data).await;
        if let Err(e) = done {
            // VRChat may still have cleared it meanwhile. The frame that did so left it alone
            // because of the guard, so it is logged here as handled elsewhere.
            let inner = self.inner.lock().unwrap();
            if inner.notices.iter().all(|n| n.id != id) {
                if let Some(store) = &inner.store {
                    let _ = store.log_handled(&notice, "elsewhere", "");
                }
            }
            drop(acting);
            drop(inner);
            return Err(e);
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
            drop(acting);
        }
        self.send("notices:changed", Value::Null);
        Ok(())
    }

    /// Sends what the user clicked on a notification to VRChat.
    async fn carry_out(&self, notice: &Notice, id: &str, action: &str, reply_kind: &str, reply_data: &str) -> AppResult<()> {
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
                self.api.boop(&notice.sender_id, None).await.map_err(boop_turned_down)?;
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
                self.api.invite_user(&notice.sender_id, &location, None).await?;
                // The invite went out. Hiding the request is tidying up, like after a boop
                // back: if it fails, the invite still counts as sent, so the button is not
                // pressed again for a second invite. The next sync shows the request again.
                let _ = self.api.hide_notification(id).await;
            }
            _ => return Err(AppError::CannotDo("That does not work on this notification.".into())),
        }
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
        let want = query.trim().to_lowercase();
        // A photo's time is kept in UTC; the day the user picks is a day on their own clock.
        let last = chrono::NaiveDate::parse_from_str(until.trim(), "%Y-%m-%d")
            .ok()
            .and_then(|d| d.succ_opt())
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .and_then(|t| chrono::TimeZone::from_local_datetime(&chrono::Local, &t).earliest())
            .map(|t| t.with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        let me = self.me().ok_or(AppError::NotSignedIn)?.id;
        let in_range = |photo: &&crate::photos::Photo| last.as_ref().is_none_or(|end| photo.taken_at.as_str() < end.as_str());
        let dated: Vec<&crate::photos::Photo> = all.iter().filter(in_range).collect();
        // A search can look at thousands of photos. It reads through a second, read-only door
        // to the history, so the live events and every other part of the app keep working
        // meanwhile instead of waiting for the whole search.
        let store = Store::open_existing(&self.history_path(&me))?;
        let store = &store;
        // A search over many photos reads every stay once, and each photo finds its own in
        // memory, so the history is not searched again for every photo.
        let stays: Option<Vec<Visit>> = if !want.is_empty() && dated.len() > 200 { Some(all_stays(store)?) } else { None };
        let find_stay = |at: &str| -> AppResult<Option<Visit>> {
            match &stays {
                Some(sorted) => Ok(stay_at(sorted, at)),
                None => store.visit_at(at),
            }
        };
        let describe = |photo: &crate::photos::Photo| -> AppResult<(Value, bool)> {
            let visit = find_stay(&photo.taken_at)?;
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
        let limit = limit.min(200);
        let mut page = Vec::new();
        if want.is_empty() {
            // Nothing to look up for photos that are not on this page.
            for photo in dated.iter().skip(skip).take(limit) {
                page.push(describe(photo)?.0);
            }
            return Ok(serde_json::json!({ "total": dated.len(), "everything": all.len(), "photos": page }));
        }
        let mut found = 0usize;
        for photo in &dated {
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

    pub fn videos_page(&self, query: &str, skip_arrival: bool, before: Option<(String, i64)>, limit: u32) -> AppResult<(Vec<crate::store::VideoPlay>, i64)> {
        let inner = self.inner.lock().unwrap();
        match &inner.store {
            Some(store) => store.videos_page(query, skip_arrival, before, limit),
            None => Err(AppError::NotSignedIn),
        }
    }

    /// My own numbers over a range of days, from this PC's history. `running` says whether
    /// VRChat runs now, so a stay it never closed is not counted up to now by mistake. The only
    /// thing VRChat may be asked is the name of a top world the app has not looked up yet.
    pub fn my_stats(&self, range: &str, running: bool) -> AppResult<crate::stats::MyStats> {
        use chrono::TimeZone;
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        let now = chrono::Utc::now();
        let to = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        // A range starts at midnight, in the PC's own time.
        let midnight = |days_back: i64| {
            let day = chrono::Local::now().date_naive() - chrono::Duration::days(days_back);
            let start = day.and_hms_opt(0, 0, 0).unwrap_or_default();
            // Where the clock jumps forward at midnight, the day starts at the first hour that exists.
            (0..4)
                .find_map(|h| chrono::Local.from_local_datetime(&(start + chrono::Duration::hours(h))).earliest())
                .map(|t| t.with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
                .unwrap_or_else(|| to.clone())
        };
        let from = match range {
            "7d" => midnight(6),
            "30d" => midnight(29),
            "year" => midnight(364),
            _ => match self.with_store(|s| s.first_stay())? {
                Some(first) => first,
                None => return Ok(crate::stats::MyStats { from: to.clone(), to, hours: vec![0; 24], weekdays: vec![0; 7], ..Default::default() }),
            },
        };
        let (stays, sights) = self.with_store(|s| s.stats_rows(&from, &to, &me.id, &me.display_name))?;
        let jd = |text: &str| chrono::DateTime::parse_from_rfc3339(text).map(|t| t.timestamp_millis() as f64 / 86_400_000.0 + 2_440_587.5).unwrap_or(0.0);
        let mut stats = crate::stats::work_out(jd(&from), jd(&to), &stays, &sights, running);
        stats.from = from;
        stats.to = to;
        // A world the app has no name for is looked up once, in the slow queue.
        for world in &stats.worlds {
            if world.name.is_empty() && world.world_id.starts_with("wrld_") {
                self.want_world(&world.world_id);
            }
        }
        Ok(stats)
    }

    /// Friends whose friendship day (the date the friend log first shows us as friends) falls
    /// today or in the next `ahead` days, in the PC's own calendar.
    pub fn friend_days(&self, ahead: u32) -> AppResult<Vec<FriendDay>> {
        use chrono::Datelike;
        let inner = self.inner.lock().unwrap();
        let store = inner.store.as_ref().ok_or(AppError::NotSignedIn)?;
        let today = chrono::Local::now().date_naive();
        let mut out = Vec::new();
        for (user_id, since) in store.friends_since_all()? {
            let Some(friend) = inner.friends.get(&user_id) else { continue };
            let Ok(started) = chrono::DateTime::parse_from_rfc3339(&since) else { continue };
            let start = started.with_timezone(&chrono::Local).date_naive();
            for offset in 0..=ahead.min(14) {
                let day = today + chrono::Duration::days(offset as i64);
                let years = day.year() - start.year();
                // A friendship from 29 February is kept on 28 February in other years.
                let same = (day.month(), day.day()) == (start.month(), start.day())
                    || (start.month() == 2 && start.day() == 29 && day.month() == 2 && day.day() == 28 && chrono::NaiveDate::from_ymd_opt(day.year(), 2, 29).is_none());
                if same && years >= 1 {
                    out.push(FriendDay {
                        user_id: user_id.clone(),
                        name: friend.display_name.clone(),
                        image: friend.image.clone(),
                        since: since.clone(),
                        years,
                        on: day.format("%Y-%m-%d").to_string(),
                        in_days: offset as i64,
                    });
                    break;
                }
            }
        }
        out.sort_by(|a, b| a.in_days.cmp(&b.in_days).then(b.years.cmp(&a.years)).then_with(|| a.name.cmp(&b.name)));
        Ok(out)
    }

    pub fn videos(&self, limit: u32) -> AppResult<Vec<crate::store::VideoPlay>> {
        let inner = self.inner.lock().unwrap();
        match &inner.store {
            Some(store) => store.videos(limit),
            None => Err(AppError::NotSignedIn),
        }
    }

    pub fn visit_videos(&self, visit_id: i64) -> AppResult<Vec<crate::store::VideoPlay>> {
        let inner = self.inner.lock().unwrap();
        match &inner.store {
            Some(store) => store.visit_videos(visit_id),
            None => Err(AppError::NotSignedIn),
        }
    }

    /// Titles for YouTube videos that played around me. YouTube is asked once per video, a few
    /// at a time with a pause, and the answer is kept; VRChat is not asked anything.
    pub async fn video_titles(&self, ids: Vec<String>) -> AppResult<Vec<crate::store::VideoTitle>> {
        let mut ids: Vec<String> = ids
            .into_iter()
            .filter(|id| id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
            .collect();
        ids.sort();
        ids.dedup();
        ids.truncate(60);
        let known = {
            let inner = self.inner.lock().unwrap();
            let Some(store) = &inner.store else { return Err(AppError::NotSignedIn) };
            store.video_titles(&ids)?
        };
        let missing: Vec<String> = ids.iter().filter(|id| !known.iter().any(|k| &k.id == *id)).take(8).cloned().collect();
        if missing.is_empty() {
            return Ok(known);
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .user_agent("Anonymous VRX")
            .build()
            .map_err(|e| AppError::Storage(e.to_string()))?;
        for (n, id) in missing.iter().enumerate() {
            if n > 0 {
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
            // The id holds only letters, digits, - and _, so the address needs no other escaping.
            let ask = format!("https://www.youtube.com/oembed?format=json&url=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3D{id}");
            let answer = client.get(ask).send().await;
            let (title, channel) = match answer {
                Ok(r) if r.status().is_success() => match r.json::<Value>().await {
                    Ok(v) => (
                        v.get("title").and_then(Value::as_str).unwrap_or_default().chars().take(300).collect::<String>(),
                        v.get("author_name").and_then(Value::as_str).unwrap_or_default().chars().take(120).collect::<String>(),
                    ),
                    Err(_) => continue,
                },
                // A removed or private video has no title to give; that answer is kept too,
                // so YouTube is not asked about it again.
                Ok(r) if matches!(r.status().as_u16(), 400 | 401 | 403 | 404) => (String::new(), String::new()),
                // No answer at all: asked again another time.
                _ => continue,
            };
            let inner = self.inner.lock().unwrap();
            if let Some(store) = &inner.store {
                let _ = store.save_video_title(id, &title, &channel);
            }
        }
        let inner = self.inner.lock().unwrap();
        let Some(store) = &inner.store else { return Err(AppError::NotSignedIn) };
        store.video_titles(&ids)
    }

    /// Once for each history: reads the videos out of the log that was read before videos were
    /// kept. Returns true when any was found.
    fn read_back_videos(&self, dir: &std::path::Path) -> bool {
        const DONE: &str = "videos:read-back";
        let todo = {
            let inner = self.inner.lock().unwrap();
            let Some(store) = &inner.store else { return false };
            if !matches!(store.setting(DONE), Ok(None)) {
                return false;
            }
            gamelog::read_so_far(dir, store).unwrap_or_default()
        };
        let mut found = false;
        for (path, upto) in todo {
            let Ok(events) = gamelog::videos_in(&path, upto) else { continue };
            if events.is_empty() {
                continue;
            }
            let inner = self.inner.lock().unwrap();
            let Some(store) = &inner.store else { return found };
            found |= store.record_old_videos(&events).unwrap_or(false);
        }
        let inner = self.inner.lock().unwrap();
        if let Some(store) = &inner.store {
            let _ = store.set_setting(DONE, "1");
        }
        found
    }

    /// Writes the history out as CSV files in a new folder under `root`. The files are written
    /// through a second, read-only door to the history, so the app keeps working meanwhile.
    pub fn export_csv(&self, root: &std::path::Path, parts: &[String]) -> AppResult<crate::export::ExportInfo> {
        let me = self.me().ok_or(AppError::NotSignedIn)?;
        let copy = Store::open_existing(&self.history_path(&me.id))?;
        crate::export::create(root, &copy, parts)
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
        // A stay is told apart by when it began and where, not by its row id, because a
        // VRCX import can renumber the visits and give the open one a new id.
        let mut stay: Option<Option<(String, String)>> = None;
        loop {
            let found = self.read_back_videos(&dir);
            let changed = self.read_log(&dir) || found;
            if let Ok(here) = self.here() {
                let now = here.as_ref().map(|(visit, _)| stay_key(visit));
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
        // Nothing read for an account signed in before may greet this one.
        self.fav_cache.lock().unwrap().clear();
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
        // Added to, not replaced: the live task may already have put its favourites read-ahead there.
        self.inner.lock().unwrap().tasks.extend(tasks);
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

    /// VRChat ended the sign-in on its side. Every background worker is stopped first, so
    /// lookups still queued are never sent without a sign-in, then the dead cookie is dropped
    /// from the saved session and everything kept for the account is forgotten.
    ///
    /// Called from the live connection's own task, whose handle is among those aborted. That
    /// is harmless: an abort only takes hold at the next wait, and nothing here waits.
    fn end_session(&self) {
        self.stop_tasks();
        self.api.clear_auth();
        if self.use_secrets {
            let _ = secrets::save(&self.api.session());
        }
        self.forget();
        self.send("session:ended", Value::Null);
    }

    fn forget(&self) {
        // What was kept for the account that is leaving must not greet the next one.
        self.fav_cache.lock().unwrap().clear();
        self.waiting.lock().unwrap().clear();
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
            tokio::join!(self.sync_friends(Some(user)), self.sync_favorites_read(), self.sync_notices(false));
        let (favorites, read) = match favorites {
            Ok(read) => (Ok(()), Some(read)),
            Err(e) => (Err(e), None),
        };
        for result in [friends, favorites, notices] {
            if let Err(e) = result {
                self.send("sync:error", json!(e));
            }
        }
        // Favourites are read ahead, so their screens open at once. The task is kept with the
        // others, so signing out stops it and it sends nothing more to VRChat.
        let ahead = self.clone();
        let reading = tokio::spawn(async move { ahead.read_ahead_favorites(read).await });
        self.inner.lock().unwrap().tasks.push(reading);
        let mut delay = Duration::from_secs(5);
        loop {
            self.set_pipeline("connecting");
            let started = Instant::now();
            if let Ended::SessionOver = self.listen().await {
                self.end_session();
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
                        self.end_session();
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
                        // Still on the way: where they came from is still worth knowing.
                        f.before_travel = old.before_travel.clone();
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
        // A count below the friends I know are there is out of date already: it was taken
        // while the instance was filling. That one is asked again after half a minute.
        let friends_there = inner.friends.values().filter(|f| f.state == "online" && f.location == location).count() as i64;
        let fresh = inner
            .instances
            .get(location)
            .is_some_and(|(i, at)| at.elapsed() < if i.users < friends_there { COUNT_STALE } else { COUNT_FRESH });
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
            let friends_at: HashSet<String> = {
                let inner = self.inner.lock().unwrap();
                inner
                    .friends
                    .values()
                    .filter(|f| f.state == "online" && f.place.kind == "world")
                    .map(|f| f.location.clone())
                    .collect()
            };
            // The instance I am in counts as occupied too, so its count is kept for its full
            // time and the game log does not ask for it again every round.
            let mut occupied = friends_at.clone();
            if let Ok(Some((visit, _))) = self.here() {
                occupied.insert(visit.location);
            }
            {
                let mut inner = self.inner.lock().unwrap();
                inner.instances.retain(|loc, _| occupied.contains(loc));
                // A refusal is honoured for its whole time, wherever anyone is.
                inner.refused.retain(|loc, at| occupied.contains(loc) || at.elapsed() < COUNT_RETRY);
            }
            // Only friends' instances are asked for here; my own is asked for by the game log.
            for location in &friends_at {
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
    pub async fn request_invite(&self, user_id: &str, slot: Option<u8>) -> AppResult<()> {
        if slot.is_some_and(|s| s > 11) {
            return Err(AppError::CannotDo("There are twelve saved messages, 1 to 12.".into()));
        }
        let is_friend = self.inner.lock().unwrap().friends.contains_key(user_id);
        if !is_friend {
            return Err(AppError::CannotDo("Only friends can be asked for an invite.".into()));
        }
        self.api.request_invite(user_id, slot).await
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
            // Without a sign-in the lookups still queued are dropped, never sent.
            if self.api.auth_token().is_none() {
                break;
            }
            match job {
                Job::World(id) => match self.api.world(&id).await {
                    Ok(json) => {
                        if let Some(world) = World::from_json(&json) {
                            self.keep_world(world);
                        }
                    }
                    Err(AppError::RateLimited(_)) => {
                        tokio::time::sleep(Duration::from_secs(120)).await;
                        self.inner.lock().unwrap().wanted.remove(&id);
                    }
                    // A dropped connection or a fault on VRChat's side says nothing about the
                    // world, so the next time it is seen or opened it is asked for again.
                    Err(AppError::Network(_)) => {
                        self.inner.lock().unwrap().wanted.remove(&id);
                    }
                    Err(AppError::Api { status, .. }) if status >= 500 => {
                        self.inner.lock().unwrap().wanted.remove(&id);
                    }
                    // Refused or not found: asked once this session, as before.
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
                    Err(AppError::RateLimited(_)) => tokio::time::sleep(Duration::from_secs(120)).await,
                    Err(_) => {}
                },
                Job::Instance(location) => {
                    let result = self.api.instance(&location).await;
                    let limited = matches!(result, Err(AppError::RateLimited(_)));
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
        // When anything last came in: a frame, a pong or a ping. Kept across the loop, so the
        // pings going out never restart the count.
        let mut heard = Instant::now();
        loop {
            let next = tokio::select! {
                next = socket.next() => next,
                _ = ping.tick() => {
                    // Something in between can keep the link up and take our pings while
                    // nothing comes back from VRChat. Quiet that long means the connection is dead.
                    if heard.elapsed() > QUIET_LIMIT {
                        return Ended::Dropped;
                    }
                    // A connection that silently died fails here.
                    if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                        return Ended::Dropped;
                    }
                    continue;
                }
            };
            let message = match next {
                Some(Ok(m)) => m,
                _ => return Ended::Dropped,
            };
            heard = Instant::now();
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
            let (outcome, acting) = {
                let mut inner = self.inner.lock().unwrap();
                let outcome = notices::apply(&mut inner.notices, &kind, &content);
                // Read while the notices are still locked, so a notice the app is acting on
                // cannot finish being logged by notice_action in between.
                let acting = self.acting.lock().unwrap().clone();
                (outcome, acting)
            };
            if let Some(notice) = &outcome.arrived {
                self.announce(notice);
            }
            self.log_cleared(&outcome.removed, &acting);
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

/// Every stay in the history, oldest first by when it began, ties by id.
fn all_stays(store: &Store) -> AppResult<Vec<Visit>> {
    let mut all: Vec<Visit> = Vec::new();
    let mut before = None;
    loop {
        let page = store.visits(500, before)?;
        let Some(last) = page.last() else { break };
        before = Some(last.id);
        all.extend(page);
    }
    all.sort_by(|a, b| a.joined_at.cmp(&b.joined_at).then(a.id.cmp(&b.id)));
    Ok(all)
}

/// The stay I was in at a moment, the same way `Store::visit_at` finds it: the one that began
/// last at or before the moment, newest id first on a tie, and only if it had not ended yet.
fn stay_at(sorted: &[Visit], at: &str) -> Option<Visit> {
    let after = sorted.partition_point(|v| v.joined_at.as_str() <= at);
    let latest = sorted[..after].last()?;
    latest.left_at.as_deref().is_none_or(|left| left >= at).then(|| latest.clone())
}

/// What tells one stay from another across passes over the log: when it began and where.
/// The row id is not used, because a VRCX import can renumber the visits.
fn stay_key(visit: &Visit) -> (String, String) {
    (visit.joined_at.clone(), visit.location.clone())
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

#[cfg(test)]
mod hub_tests {
    use super::*;

    fn hub() -> Arc<Hub> {
        let api = Arc::new(ApiClient::new().unwrap());
        Hub::new(api, std::env::temp_dir(), Arc::new(|_, _| {}), false, None)
    }

    fn sign_in_as(hub: &Hub, id: &str, status: &str) {
        hub.inner.lock().unwrap().me = Some(Me {
            id: id.into(),
            display_name: "Tester".into(),
            status: status.into(),
            status_description: String::new(),
            image: String::new(),
        });
    }

    #[test]
    fn a_late_word_is_held_back_but_a_real_change_goes_through() {
        let hub = hub();
        sign_in_as(&hub, "usr_me", "busy");
        let set = |hub: &Hub| {
            *hub.status_set.lock().unwrap() = Some(StatusSet {
                at: Instant::now(),
                old_status: "active".into(),
                old_text: String::new(),
                new_status: "busy".into(),
                new_text: String::new(),
            });
        };
        set(&hub);
        // The old status arriving late does not put it back.
        hub.own_status_changed(&json!({ "id": "usr_me", "status": "active" }));
        assert_eq!(hub.me().unwrap().status, "busy");
        // A change made in VRChat a few seconds later is not lost.
        hub.own_status_changed(&json!({ "id": "usr_me", "status": "join me" }));
        assert_eq!(hub.me().unwrap().status, "join me");

        // Once VRChat confirms the new status, later word is news whatever it says.
        sign_in_as(&hub, "usr_me", "busy");
        set(&hub);
        hub.own_status_changed(&json!({ "id": "usr_me", "status": "busy" }));
        assert!(hub.status_set.lock().unwrap().is_none());
        hub.own_status_changed(&json!({ "id": "usr_me", "status": "active" }));
        assert_eq!(hub.me().unwrap().status, "active");
    }

    #[test]
    fn favourites_kept_for_one_account_are_not_shown_to_another() {
        let hub = hub();
        let answer = json!({ "groups": [], "entries": [] });
        // Read for an account that has signed out since: dropped.
        hub.cache_favorites("avatar", "usr_a", &answer);
        assert!(hub.fav_cache.lock().unwrap().is_empty());
        sign_in_as(&hub, "usr_a", "active");
        hub.cache_favorites("avatar", "usr_a", &answer);
        assert_eq!(hub.favorites_cached("avatar"), Some(answer));
        sign_in_as(&hub, "usr_b", "active");
        assert!(hub.favorites_cached("avatar").is_none());
    }

    fn stay(id: i64, joined: &str, left: Option<&str>) -> Visit {
        Visit {
            id,
            location: format!("wrld_{id}:1"),
            world_id: format!("wrld_{id}"),
            world_name: String::new(),
            joined_at: joined.into(),
            left_at: left.map(str::to_string),
            place: Default::default(),
            people: 0,
        }
    }

    #[test]
    fn a_photo_finds_its_stay_in_memory_like_the_history_does() {
        let mut stays = vec![
            stay(3, "2026-01-01T12:00:00Z", None),
            stay(1, "2026-01-01T10:00:00Z", Some("2026-01-01T11:00:00Z")),
            stay(2, "2026-01-01T12:00:00Z", Some("2026-01-01T13:00:00Z")),
        ];
        stays.sort_by(|a, b| a.joined_at.cmp(&b.joined_at).then(a.id.cmp(&b.id)));
        let at = |t: &str| stay_at(&stays, t).map(|v| v.id);
        assert_eq!(at("2026-01-01T09:00:00Z"), None, "before any stay");
        assert_eq!(at("2026-01-01T10:30:00Z"), Some(1));
        assert_eq!(at("2026-01-01T11:30:00Z"), None, "between two stays");
        // Two stays that began at the same moment: the newer id wins, as in the history.
        assert_eq!(at("2026-01-01T12:00:00Z"), Some(3));
        assert_eq!(at("2026-01-01T15:00:00Z"), Some(3), "a stay never closed");
    }

    #[test]
    fn a_notice_being_acted_on_is_not_logged_by_the_frame_that_clears_it() {
        let hub = hub();
        hub.inner.lock().unwrap().store = Some(Store::in_memory().unwrap());
        let arrive = |id: &str| {
            let note = json!({ "id": id, "type": "invite", "senderUserId": "usr_a", "senderUsername": "Ana", "created_at": "2026-01-01T10:00:00Z" });
            hub.handle_frame(&json!({ "type": "notification", "content": note.to_string() }).to_string());
        };
        let hide = |id: &str| hub.handle_frame(&json!({ "type": "hide-notification", "content": id }).to_string());
        let logged = || hub.inner.lock().unwrap().store.as_ref().unwrap().handled(50).unwrap().len();

        arrive("not_1");
        let guard = Acting::start(&hub.acting, "not_1");
        hide("not_1");
        assert!(hub.notices().is_empty());
        assert_eq!(logged(), 0, "left for the action to log");
        drop(guard);

        arrive("not_2");
        hide("not_2");
        assert_eq!(logged(), 1, "cleared elsewhere, logged once");
    }

    #[test]
    fn a_renumbered_stay_is_still_the_same_stay() {
        let before = stay(7, "2026-01-01T12:00:00Z", None);
        // A VRCX import gave the open stay a new id; it is still where I am.
        let mut after = before.clone();
        after.id = 42;
        assert_eq!(stay_key(&before), stay_key(&after));
        // A new stay in the same world is a move, even with the old id.
        let mut moved = before.clone();
        moved.joined_at = "2026-01-01T13:00:00Z".into();
        assert_ne!(stay_key(&before), stay_key(&moved));
    }

    #[test]
    fn a_second_vrcx_import_waits_for_the_first() {
        let dir = std::env::temp_dir().join(format!("anonymous-vrx-one-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let api = Arc::new(ApiClient::new().unwrap());
        let hub = Hub::new(api, dir.clone(), Arc::new(|_, _| {}), false, None);
        let me = "usr_me-0000";
        sign_in_as(&hub, me, "active");
        hub.inner.lock().unwrap().store = Some(Store::open(&hub.history_path(me)).unwrap());
        let other = dir.join("VRCX.sqlite3");
        rusqlite::Connection::open(&other)
            .unwrap()
            .execute_batch(
                "CREATE TABLE usrme0000_feed_gps (id INTEGER PRIMARY KEY, created_at TEXT, user_id TEXT, display_name TEXT, location TEXT, world_name TEXT, previous_location TEXT, time INTEGER, group_name TEXT);
                 INSERT INTO usrme0000_feed_gps (created_at, user_id, display_name, location, previous_location) VALUES ('2025-01-02T10:00:00.000Z', 'usr_a', 'Ana', 'wrld_x:1', 'private');",
            )
            .unwrap();
        let backups = dir.join("backups");

        // While one import holds the mark, another is turned away before it makes a backup or
        // writes anything.
        let first = Importing::start(&hub.importing).unwrap();
        for by_itself in [false, true] {
            match hub.import_vrcx(&other, &backups, "{}", by_itself, None) {
                Err(AppError::CannotDo(words)) => assert_eq!(words, "VRCX history is already being brought over. Wait for it to finish."),
                got => panic!("a second import ran: {got:?}"),
            }
        }
        assert!(!backups.exists(), "no backup was made for the run that was turned away");
        assert_eq!(hub.feed(10, None).unwrap().len(), 0);
        drop(first);

        // Once the first is done the next one runs, and the mark is let go again afterwards,
        // also when the run fails.
        let got = hub.import_vrcx(&other, &backups, "{}", false, None).unwrap();
        assert_eq!(got["broughtOver"]["feed"], 1);
        assert!(!hub.importing.load(std::sync::atomic::Ordering::Acquire));
        assert!(hub.import_vrcx(&dir.join("missing.sqlite3"), &backups, "{}", false, None).is_err());
        // A backup folder that cannot be made stops the run after it took the mark.
        std::fs::write(dir.join("not a folder"), b"").unwrap();
        assert!(hub.import_vrcx(&other, &dir.join("not a folder"), "{}", false, None).is_err());
        assert!(!hub.importing.load(std::sync::atomic::Ordering::Acquire), "a failed run lets go of the mark");
        assert_eq!(hub.import_vrcx(&other, &backups, "{}", false, None).unwrap()["broughtOver"]["feed"], 0);
        hub.inner.lock().unwrap().store = None;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Builds a history of 50,000 feed entries and 10,000 stays, and a VRCX file with 50,000
    /// friend-activity lines (10,000 of them already here), 10,000 stays (half already here)
    /// with 50,000 join and leave lines, then brings VRCX over while the rest of the app keeps
    /// reading and writing. It takes a few seconds even in a debug build; `--nocapture` prints
    /// the times.
    #[test]
    fn a_big_vrcx_import_leaves_the_app_free() {
        let dir = std::env::temp_dir().join(format!("anonymous-vrx-big-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let api = Arc::new(ApiClient::new().unwrap());
        let hub = Hub::new(api, dir.clone(), Arc::new(|_, _| {}), false, None);
        // `ANON_VRX_SCALE` makes everything but the avatars, notes and favourites that many times bigger.
        let n: usize = std::env::var("ANON_VRX_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(1).max(1);
        let me = "usr_me-0000";
        sign_in_as(&hub, me, "active");
        let store = Store::open(&hub.history_path(me)).unwrap();
        // Entry j is at 2024-01-01 plus j minutes, for one of 500 people, of one of four kinds.
        store
            .run_sql(&format!(
                "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 50000 * {n} - 1)
                 INSERT INTO feed (at, user_id, display_name, kind)
                     SELECT strftime('%Y-%m-%dT%H:%M:%fZ', '2024-01-01', '+' || (i * 60) || ' seconds'), 'usr_' || (i % 500), 'Person ' || (i % 500),
                            CASE i % 4 WHEN 0 THEN 'online' WHEN 1 THEN 'offline' WHEN 2 THEN 'location' ELSE 'status' END
                     FROM n;
                 WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 10000 * {n} - 1)
                 INSERT INTO visits (location, world_id, world_name, joined_at, left_at)
                     SELECT 'wrld_' || (i % 200) || ':' || i, 'wrld_' || (i % 200), 'World ' || (i % 200),
                            strftime('%Y-%m-%dT%H:%M:%fZ', '2024-01-01', '+' || (i * 900) || ' seconds'),
                            strftime('%Y-%m-%dT%H:%M:%fZ', '2024-01-01', '+' || (i * 900 + 600) || ' seconds')
                     FROM n;"
            ))
            .unwrap();
        hub.inner.lock().unwrap().store = Some(store);

        let other = dir.join("VRCX.sqlite3");
        {
            let old = rusqlite::Connection::open(&other).unwrap();
            old.execute_batch(&format!(
                "CREATE TABLE usrme0000_feed_gps (id INTEGER PRIMARY KEY, created_at TEXT, user_id TEXT, display_name TEXT, location TEXT, world_name TEXT, previous_location TEXT, time INTEGER, group_name TEXT);
                 CREATE TABLE usrme0000_feed_online_offline (id INTEGER PRIMARY KEY, created_at TEXT, user_id TEXT, display_name TEXT, type TEXT, location TEXT, world_name TEXT, time INTEGER, group_name TEXT);
                 CREATE TABLE usrme0000_feed_status (id INTEGER PRIMARY KEY, created_at TEXT, user_id TEXT, display_name TEXT, status TEXT, status_description TEXT, previous_status TEXT, previous_status_description TEXT);
                 CREATE TABLE usrme0000_feed_avatar (id INTEGER PRIMARY KEY, created_at TEXT, user_id TEXT, display_name TEXT, owner_id TEXT, avatar_name TEXT, current_avatar_image_url TEXT, current_avatar_thumbnail_image_url TEXT, previous_current_avatar_image_url TEXT, previous_current_avatar_thumbnail_image_url TEXT);
                 CREATE TABLE usrme0000_friend_log_history (id INTEGER PRIMARY KEY, created_at TEXT, type TEXT, user_id TEXT, display_name TEXT, previous_display_name TEXT, trust_level TEXT, previous_trust_level TEXT, friend_number INTEGER);
                 CREATE TABLE gamelog_location (id INTEGER PRIMARY KEY, created_at TEXT, location TEXT, world_id TEXT, world_name TEXT, time INTEGER, group_name TEXT);
                 CREATE TABLE gamelog_join_leave (id INTEGER PRIMARY KEY, created_at TEXT, type TEXT, display_name TEXT, location TEXT, user_id TEXT, time INTEGER);
                 CREATE TABLE usrme0000_avatar_history (avatar_id TEXT PRIMARY KEY, created_at TEXT, time INTEGER);
                 CREATE TABLE memos (user_id TEXT PRIMARY KEY, edited_at TEXT, memo TEXT);
                 CREATE TABLE favorite_world (id INTEGER PRIMARY KEY, created_at TEXT, world_id TEXT, group_name TEXT);

                 -- Lines from before this history began, 47 seconds apart in each table.
                 WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 15000 * {n} - 1)
                 INSERT INTO usrme0000_feed_gps (created_at, user_id, display_name, location, previous_location)
                     SELECT strftime('%Y-%m-%dT%H:%M:%fZ', '2023-06-01', '+' || (i * 47) || ' seconds'), 'usr_' || (i % 500), 'Person', 'wrld_1:' || i, 'private' FROM n;
                 WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 5000 * {n} - 1)
                 INSERT INTO usrme0000_feed_online_offline (created_at, user_id, display_name, type, location)
                     SELECT strftime('%Y-%m-%dT%H:%M:%fZ', '2023-06-01', '+' || (i * 47 + 3) || ' seconds'), 'usr_' || (i % 500), 'Person', 'Offline', '' FROM n;
                 -- The same moments as the 'online' entries already here: all of them are skipped.
                 WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 10000 * {n} - 1)
                 INSERT INTO usrme0000_feed_online_offline (created_at, user_id, display_name, type, location)
                     SELECT strftime('%Y-%m-%dT%H:%M:%fZ', '2024-01-01', '+' || (i * 240 + 2) || ' seconds'), 'usr_' || ((i * 4) % 500), 'Person', 'Online', '' FROM n;
                 WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 10000 * {n} - 1)
                 INSERT INTO usrme0000_feed_status (created_at, user_id, display_name, status, status_description, previous_status, previous_status_description)
                     SELECT strftime('%Y-%m-%dT%H:%M:%fZ', '2023-06-01', '+' || (i * 47 + 5) || ' seconds'), 'usr_' || (i % 500), 'Person', 'busy', 'same', 'active', 'same' FROM n;
                 WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 5000 * {n} - 1)
                 INSERT INTO usrme0000_feed_avatar (created_at, user_id, display_name)
                     SELECT strftime('%Y-%m-%dT%H:%M:%fZ', '2023-06-01', '+' || (i * 47 + 7) || ' seconds'), 'usr_' || (i % 500), 'Person' FROM n;
                 WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 5000 * {n} - 1)
                 INSERT INTO usrme0000_friend_log_history (created_at, type, user_id, display_name, previous_display_name)
                     SELECT strftime('%Y-%m-%dT%H:%M:%fZ', '2023-06-01', '+' || (i * 47 + 9) || ' seconds'), 'Friend', 'usr_' || i, 'Person', '' FROM n;

                 -- Half the stays are ones already here, three seconds off; half are older.
                 WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 5000 * {n} - 1)
                 INSERT INTO gamelog_location (created_at, location, world_id, world_name, time)
                     SELECT strftime('%Y-%m-%dT%H:%M:%fZ', '2024-01-01', '+' || (i * 2 * 900 + 3) || ' seconds'), 'wrld_' || ((i * 2) % 200) || ':' || (i * 2), 'wrld_' || ((i * 2) % 200), 'World', 600000 FROM n;
                 WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 5000 * {n} - 1)
                 INSERT INTO gamelog_location (created_at, location, world_id, world_name, time)
                     SELECT strftime('%Y-%m-%dT%H:%M:%fZ', '2023-06-01', '+' || (i * 900) || ' seconds'), 'wrld_old' || (i % 200) || ':' || i, 'wrld_old' || (i % 200), 'Old world', 600000 FROM n;
                 -- Five lines in each stay: three people join, two of them leave.
                 INSERT INTO gamelog_join_leave (created_at, type, display_name, location, user_id)
                     SELECT strftime('%Y-%m-%dT%H:%M:%fZ', l.created_at, '+' || (k.n * 60 + 10) || ' seconds'),
                            CASE WHEN k.n < 3 THEN 'OnPlayerJoined' ELSE 'OnPlayerLeft' END,
                            'Guest ' || (k.n % 3), l.location, 'usr_g' || (k.n % 3)
                     FROM gamelog_location l, (SELECT 0 AS n UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4) k;

                 WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 1999)
                 INSERT INTO usrme0000_avatar_history (avatar_id, created_at, time)
                     SELECT 'avtr_' || i, strftime('%Y-%m-%dT%H:%M:%fZ', '2023-06-01', '+' || (i * 3600) || ' seconds'), 3600000 FROM n;
                 WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 999)
                 INSERT INTO memos (user_id, edited_at, memo) SELECT 'usr_' || i, '2023-06-01T00:00:00.000Z', 'note ' || i FROM n;
                 WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 499)
                 INSERT INTO favorite_world (created_at, world_id, group_name) SELECT '2023-06-01T00:00:00.000Z', 'wrld_' || i, 'Fav' FROM n;"
            ))
            .unwrap();
        }

        let backups = dir.join("backups");
        let started = Instant::now();
        let worker = {
            let hub = hub.clone();
            let (other, backups) = (other.clone(), backups.clone());
            std::thread::spawn(move || {
                let got = hub.import_vrcx(&other, &backups, "{}", false, None);
                (got, started.elapsed())
            })
        };
        // Meanwhile the app reads its feed and writes to it, as it does while it runs.
        let (mut calls, mut slowest) = (0u32, Duration::ZERO);
        while !worker.is_finished() {
            let asked = Instant::now();
            hub.feed(50, None).unwrap();
            hub.with_store(|s| {
                s.add_feed(&FeedDraft {
                    user_id: "usr_live".into(),
                    display_name: "Live".into(),
                    kind: "online",
                    detail: String::new(),
                    previous: String::new(),
                    location: String::new(),
                })
            })
            .unwrap();
            slowest = slowest.max(asked.elapsed());
            calls += 1;
            std::thread::sleep(Duration::from_millis(20));
        }
        let (got, took) = worker.join().unwrap();
        let got = got.unwrap();
        println!("import took {took:?}; the app made {calls} reads and writes meanwhile, the slowest took {slowest:?}");
        println!("{got}");
        let counts = &got["broughtOver"];
        assert_eq!(counts["feed"], 35000 * n);
        assert_eq!(counts["friendLog"], 5000 * n);
        assert_eq!(counts["visits"], 5000 * n);
        assert_eq!(counts["people"], 15000 * n);
        assert_eq!(counts["avatars"], 2000);
        assert_eq!(counts["notes"], 1000);
        assert_eq!(counts["favourites"], 500);
        assert!(!got["backup"].as_str().unwrap_or_default().is_empty(), "a backup was made first");
        assert!(calls > 0, "the app got its turn while the import ran");
        // The longest wait is the one write that puts the whole feed back in order of time.
        let limit = Duration::from_secs(3).max(Duration::from_millis(500 * n as u64));
        assert!(slowest < limit, "the app waited {slowest:?} for one call");
        // The feed stays in order of time, newest first.
        let feed = hub.feed(200, None).unwrap();
        assert!(feed.windows(2).all(|w| w[0].at >= w[1].at));

        let again = Instant::now();
        let second = hub.import_vrcx(&other, &backups, "{}", false, None).unwrap();
        println!("a second run took {:?}", again.elapsed());
        assert_eq!(second["broughtOver"], json!(crate::store::BroughtOver::default()));
        let wardrobe = Instant::now();
        hub.avatar_history(1000).unwrap();
        println!("the wardrobe list took {:?}", wardrobe.elapsed());
        hub.inner.lock().unwrap().store = None;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
