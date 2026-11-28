pub mod backup;
pub mod discord;
pub mod error;
pub mod export;
pub mod gamecache;
pub mod gamelog;
pub mod hub;
pub mod overlay;
pub mod photos;
pub mod secrets;
pub mod stats;
pub mod store;
pub mod vr;
pub mod vrchat;

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

use error::{AppError, AppResult};
use hub::Hub;
use store::{Sighting, Visit};
use vrchat::client::{ApiClient, LoginOutcome};
use vrchat::model::{Favorites, FeedItem, Friend, Instance, Me, World};
use vrchat::notices::{Handled, Notice};
use vrchat::people::{AvatarCard, Person, WorldCard};

/// Used in the User-Agent, the credential store entry and file names.
pub const APP_SLUG: &str = "anonymous-vrx";
/// Shown to VRChat in the User-Agent so they can tell what this client is.
pub const APP_CONTACT: &str = "https://github.com/ElDiablo66/anonymous-vrx-app";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SignIn {
    /// `signedIn`, `signedOut` or `needsCode`.
    status: &'static str,
    me: Option<Me>,
    methods: Vec<String>,
}

impl SignIn {
    fn of(me: Option<Me>) -> SignIn {
        SignIn {
            status: if me.is_some() { "signedIn" } else { "signedOut" },
            me,
            methods: Vec::new(),
        }
    }
}

#[tauri::command]
async fn session_restore(hub: State<'_, Arc<Hub>>) -> AppResult<SignIn> {
    Ok(SignIn::of(hub.restore().await?))
}

#[tauri::command]
async fn login(hub: State<'_, Arc<Hub>>, username: String, password: String) -> AppResult<SignIn> {
    match hub.api.login(username.trim(), &password).await? {
        LoginOutcome::SignedIn(user) => Ok(SignIn::of(Some(hub.begin(user)?))),
        LoginOutcome::NeedsCode(methods) => Ok(SignIn {
            status: "needsCode",
            me: None,
            methods,
        }),
    }
}

#[tauri::command]
async fn verify_code(hub: State<'_, Arc<Hub>>, method: String, code: String) -> AppResult<SignIn> {
    hub.api.verify_code(&method, &code).await?;
    let user = hub.api.current_user().await?;
    Ok(SignIn::of(Some(hub.begin(user)?)))
}

#[tauri::command]
async fn logout(hub: State<'_, Arc<Hub>>) -> AppResult<()> {
    hub.sign_out().await
}

// Commands that read the history or the hub's shared state are marked `async`, so they run
// on the background pool instead of the window's own thread. A command that has to wait for
// the history (a long import or another long job holding it) then never freezes the window.
#[tauri::command(async)]
fn get_friends(hub: State<'_, Arc<Hub>>) -> Vec<Friend> {
    hub.friends()
}

#[tauri::command(async)]
fn get_favorites(hub: State<'_, Arc<Hub>>) -> Favorites {
    hub.favorites()
}

#[tauri::command(async)]
fn get_notices(hub: State<'_, Arc<Hub>>) -> Vec<Notice> {
    hub.notices()
}

#[tauri::command]
async fn get_profile(hub: State<'_, Arc<Hub>>, user_id: String) -> AppResult<hub::Profile> {
    hub.profile(&user_id).await
}

#[tauri::command]
async fn get_profile_part(
    hub: State<'_, Arc<Hub>>,
    user_id: String,
    part: String,
    extra: Option<String>,
    provider: Option<String>,
) -> AppResult<serde_json::Value> {
    hub.profile_part(&user_id, &part, extra.as_deref().unwrap_or(""), provider.as_deref().unwrap_or(""))
        .await
}

#[tauri::command]
async fn get_thing(hub: State<'_, Arc<Hub>>, kind: String, id: String) -> AppResult<serde_json::Value> {
    hub.thing(&kind, &id).await
}

#[tauri::command]
async fn get_favorites_of(hub: State<'_, Arc<Hub>>, kind: String) -> AppResult<serde_json::Value> {
    hub.favorites_of(&kind).await
}

#[tauri::command]
async fn add_favorite(hub: State<'_, Arc<Hub>>, kind: String, id: String, group: String) -> AppResult<()> {
    hub.add_favorite(&kind, &id, &group).await
}

#[tauri::command]
async fn remove_favorites(hub: State<'_, Arc<Hub>>, entry_ids: Vec<String>) -> AppResult<usize> {
    hub.remove_favorites(&entry_ids).await
}

/// My favourite groups of every kind with how many favourites each holds.
#[tauri::command]
async fn get_favorite_groups(hub: State<'_, Arc<Hub>>, counts: bool) -> AppResult<serde_json::Value> {
    if counts {
        hub.favorite_groups_overview().await
    } else {
        hub.favorite_group_list().await
    }
}

#[tauri::command]
async fn check_favorites(hub: State<'_, Arc<Hub>>, kind: String) -> AppResult<serde_json::Value> {
    hub.check_favorites(&kind).await
}

#[tauri::command]
async fn boop(hub: State<'_, Arc<Hub>>, user_id: String, emoji: Option<String>) -> AppResult<()> {
    hub.boop(&user_id, emoji.as_deref()).await
}

/// My twelve saved messages for invites, or for asking to be invited.
#[tauri::command]
async fn get_invite_messages(hub: State<'_, Arc<Hub>>, kind: String) -> AppResult<serde_json::Value> {
    hub.invite_messages(&kind).await
}

/// Changes one saved message. VRChat allows it once an hour for each slot.
#[tauri::command]
async fn set_invite_message(hub: State<'_, Arc<Hub>>, kind: String, slot: u8, text: String) -> AppResult<serde_json::Value> {
    hub.set_invite_message(&kind, slot, &text).await
}

/// The frames, effects and backgrounds I own.
#[tauri::command]
async fn get_my_cosmetics(hub: State<'_, Arc<Hub>>) -> AppResult<serde_json::Value> {
    hub.my_cosmetics().await
}

/// Changes my languages, frame, effects and background. Only the keys given change.
#[tauri::command]
async fn update_look(hub: State<'_, Arc<Hub>>, changes: serde_json::Value) -> AppResult<()> {
    hub.update_look(&changes).await
}

/// Changes my languages: the ones in `remove` come off, the ones in `add` go on.
#[tauri::command]
async fn update_languages(hub: State<'_, Arc<Hub>>, add: Vec<String>, remove: Vec<String>) -> AppResult<()> {
    hub.update_languages(&add, &remove).await
}

/// Everything I set about one person: block, mutes, their avatar shown or hidden.
#[tauri::command]
async fn get_moderations_for(hub: State<'_, Arc<Hub>>, user_id: String) -> AppResult<serde_json::Value> {
    hub.moderations_for(&user_id).await
}

#[tauri::command]
async fn get_my_groups(hub: State<'_, Arc<Hub>>) -> AppResult<serde_json::Value> {
    hub.my_groups().await
}

#[tauri::command]
async fn invite_to_group(hub: State<'_, Arc<Hub>>, group_id: String, user_id: String) -> AppResult<()> {
    hub.invite_to_group(&group_id, &user_id).await
}

/// Changes my bio with its links, and my pronouns. What is left out stays as it is.
#[tauri::command]
async fn update_profile(hub: State<'_, Arc<Hub>>, bio: Option<String>, links: Option<Vec<String>>, pronouns: Option<String>) -> AppResult<()> {
    let about = match (&bio, &links) {
        (Some(bio), Some(links)) => Some((bio.as_str(), links.as_slice())),
        (None, None) => None,
        _ => return Err(AppError::CannotDo("The bio and its links go together.".into())),
    };
    hub.update_profile(about, pronouns.as_deref()).await
}

fn pictures_root(app: &AppHandle) -> AppResult<std::path::PathBuf> {
    app.path()
        .picture_dir()
        .map(|d| d.join("Anonymous VRX"))
        .map_err(|e| AppError::Storage(e.to_string()))
}

/// Saves a picture into Pictures, in this app's own folder. Resolves to the full path.
#[tauri::command]
async fn save_picture(app: AppHandle, hub: State<'_, Arc<Hub>>, url: String, name: String) -> AppResult<String> {
    let (kind, bytes) = hub.picture(&url).await?;
    let ending = match kind.as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => return Err(AppError::CannotDo("That kind of picture cannot be saved.".into())),
    };
    let mut plain: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .take(60)
        .collect::<String>()
        .trim()
        .to_string();
    if plain.is_empty() {
        plain = "Picture".into();
    }
    let root = pictures_root(&app)?;
    std::fs::create_dir_all(&root).map_err(|e| AppError::Storage(e.to_string()))?;
    let mut path = root.join(format!("{plain}.{ending}"));
    let mut n = 2;
    while path.exists() {
        path = root.join(format!("{plain} {n}.{ending}"));
        n += 1;
    }
    std::fs::write(&path, bytes).map_err(|e| AppError::Storage(e.to_string()))?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
fn open_pictures_folder(app: AppHandle) -> AppResult<()> {
    let root = pictures_root(&app)?;
    std::fs::create_dir_all(&root).map_err(|e| AppError::Storage(e.to_string()))?;
    std::process::Command::new("explorer.exe")
        .arg(&root)
        .spawn()
        .map_err(|e| AppError::Storage(e.to_string()))?;
    Ok(())
}

#[tauri::command]
async fn upload_action(hub: State<'_, Arc<Hub>>, kind: String, id: String, action: String) -> AppResult<()> {
    hub.upload_action(&kind, &id, &action).await
}

/// The people I blocked or muted.
#[tauri::command]
async fn get_moderations(hub: State<'_, Arc<Hub>>, fresh: bool) -> AppResult<serde_json::Value> {
    hub.moderations(fresh).await
}

/// Every icon frame and profile effect by id, for dressing a profile the way its owner did.
#[tauri::command]
async fn get_cosmetics(hub: State<'_, Arc<Hub>>) -> AppResult<serde_json::Value> {
    hub.cosmetics().await
}

/// One month of events: `which` is `mine`, `following` or `featured`; `month` is `YYYY-MM`.
#[tauri::command]
async fn get_calendar(hub: State<'_, Arc<Hub>>, which: String, month: String, fresh: bool) -> AppResult<serde_json::Value> {
    hub.calendar(&which, &month, fresh).await
}

/// Follows a group event, or stops following it.
#[tauri::command]
async fn follow_event(hub: State<'_, Arc<Hub>>, group_id: String, event_id: String, on: bool) -> AppResult<()> {
    hub.follow_event(&group_id, &event_id, on).await
}

/// Picture, rank and status of one person, for a list row.
#[tauri::command]
async fn get_person_brief(hub: State<'_, Arc<Hub>>, user_id: String) -> AppResult<serde_json::Value> {
    hub.person_brief(&user_id).await
}

/// Blocks or mutes someone (`kind` is `block` or `mute`), or takes it back with `on` false.
#[tauri::command]
async fn set_moderation(hub: State<'_, Arc<Hub>>, user_id: String, kind: String, on: bool) -> AppResult<()> {
    hub.set_moderation(&user_id, &kind, on).await
}

#[tauri::command]
async fn friend_action(hub: State<'_, Arc<Hub>>, user_id: String, action: String) -> AppResult<()> {
    hub.friend_action(&user_id, &action).await
}

#[tauri::command]
async fn get_friend_status(hub: State<'_, Arc<Hub>>, user_id: String) -> AppResult<serde_json::Value> {
    hub.friend_status(&user_id).await
}

#[tauri::command]
async fn rename_favorite_group(hub: State<'_, Arc<Hub>>, kind: String, group: String, name: String) -> AppResult<()> {
    hub.rename_favorite_group(&kind, &group, &name).await
}

/// A picture as a data address, fetched with the sign-in when VRChat wants it.
#[tauri::command]
async fn load_picture(hub: State<'_, Arc<Hub>>, url: String) -> AppResult<String> {
    use base64::Engine;
    let (kind, bytes) = hub.picture(&url).await?;
    if bytes.len() > 12 * 1024 * 1024 {
        return Err(AppError::CannotDo("That picture is too large to show here.".into()));
    }
    Ok(format!("data:{kind};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}


#[tauri::command(async)]
fn get_favorites_cached(hub: State<'_, Arc<Hub>>, kind: String) -> Option<serde_json::Value> {
    hub.favorites_cached(&kind)
}

#[tauri::command]
async fn group_action(hub: State<'_, Arc<Hub>>, group_id: String, action: String) -> AppResult<()> {
    hub.group_action(&group_id, &action).await
}

#[tauri::command(async)]
fn get_note(hub: State<'_, Arc<Hub>>, id: String) -> AppResult<String> {
    hub.note(&id)
}

#[tauri::command(async)]
fn get_avatar_history(hub: State<'_, Arc<Hub>>, limit: u32) -> AppResult<Vec<store::AvatarWorn>> {
    hub.avatar_history(limit)
}

#[tauri::command(async)]
fn search_feed(
    hub: State<'_, Arc<Hub>>,
    query: String,
    kinds: Vec<String>,
    before: Option<i64>,
    until: Option<String>,
    limit: u32,
) -> AppResult<Vec<FeedItem>> {
    hub.feed_search(&query, &kinds, before, until.as_deref(), limit)
}

#[tauri::command(async)]
fn get_together_counts(hub: State<'_, Arc<Hub>>) -> AppResult<std::collections::HashMap<String, i64>> {
    hub.together_counts()
}

#[tauri::command(async)]
fn get_local_favorites(hub: State<'_, Arc<Hub>>, kind: String) -> AppResult<Vec<store::LocalFavorite>> {
    hub.local_favorites(&kind)
}

#[tauri::command(async)]
fn add_local_favorite(hub: State<'_, Arc<Hub>>, favorite: store::LocalFavorite) -> AppResult<()> {
    hub.add_local_favorite(&favorite)
}

#[tauri::command(async)]
fn remove_local_favorite(hub: State<'_, Arc<Hub>>, kind: String, item_id: String) -> AppResult<()> {
    hub.remove_local_favorite(&kind, &item_id)
}

#[tauri::command]
async fn set_status(hub: State<'_, Arc<Hub>>, status: String, text: String) -> AppResult<()> {
    hub.set_status(&status, &text).await
}

#[tauri::command]
async fn get_service_status(hub: State<'_, Arc<Hub>>) -> AppResult<serde_json::Value> {
    hub.service_status().await
}

/// The window's own settings, kept in a file as well. The window stores them itself, but that
/// store is written to disk with a delay, so a change made just before the app is closed or
/// updated could be lost. The file is written at once.
fn settings_file(app: &AppHandle) -> AppResult<std::path::PathBuf> {
    app.path()
        .app_data_dir()
        .map(|d| d.join("window settings.json"))
        .map_err(|e| AppError::Storage(e.to_string()))
}

#[tauri::command]
fn save_settings(app: AppHandle, text: String) -> AppResult<()> {
    // Only a JSON object of modest size is kept.
    let ok = text.len() <= 200_000 && serde_json::from_str::<serde_json::Value>(&text).is_ok_and(|v| v.is_object());
    if !ok {
        return Err(AppError::CannotDo("Those settings could not be saved.".into()));
    }
    let file = settings_file(&app)?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| AppError::Storage(e.to_string()))?;
    }
    // Written beside the file first, then swapped in, so a crash never leaves half a file.
    let fresh = file.with_extension("json.new");
    std::fs::write(&fresh, text).map_err(|e| AppError::Storage(e.to_string()))?;
    std::fs::rename(&fresh, &file).map_err(|e| AppError::Storage(e.to_string()))
}

#[tauri::command]
fn load_settings(app: AppHandle) -> AppResult<Option<String>> {
    Ok(std::fs::read_to_string(settings_file(&app)?).ok())
}

/// Says what Discord should show on my profile, or clears it with nothing.
#[tauri::command]
fn set_discord(presence: Option<discord::Presence>) {
    discord::show(presence);
}

#[tauri::command]
fn set_own_feed(hub: State<'_, Arc<Hub>>, on: bool) {
    hub.set_own_feed(on);
}

/// User id to when this PC last saw them do anything.
#[tauri::command(async)]
fn get_last_seen(hub: State<'_, Arc<Hub>>) -> AppResult<std::collections::HashMap<String, String>> {
    hub.last_seen()
}

/// The full instances I am waiting to get into.
#[tauri::command(async)]
fn get_waiting(hub: State<'_, Arc<Hub>>) -> serde_json::Value {
    hub.waiting()
}

/// Starts waiting for room in a full instance. The app looks every 45 seconds and tells me.
#[tauri::command]
async fn wait_for_room(hub: State<'_, Arc<Hub>>, location: String, world_name: String) -> AppResult<()> {
    hub.inner().wait_for_room(&location, &world_name)
}

#[tauri::command(async)]
fn stop_waiting(hub: State<'_, Arc<Hub>>, location: String) {
    hub.stop_waiting(&location);
}

/// What is known about which of my friends are friends with each other.
#[tauri::command(async)]
fn get_mutuals(hub: State<'_, Arc<Hub>>) -> AppResult<serde_json::Value> {
    hub.mutuals()
}

/// Asks VRChat about every friend in turn, slowly. Runs until done, stopped or told to slow down.
#[tauri::command]
async fn fetch_mutuals(hub: State<'_, Arc<Hub>>, everyone: bool) -> AppResult<serde_json::Value> {
    // One friend every two seconds, so a long friend list never turns into a burst of requests.
    // VRChat answers "slow down" if it is still too many, and the run stops there.
    hub.fetch_mutuals(std::time::Duration::from_millis(2000), everyone).await
}

#[tauri::command]
fn stop_mutuals(hub: State<'_, Arc<Hub>>) {
    hub.stop_mutuals();
}

/// Which of these people I often saw in the same instance, for the lines on the friend map.
#[tauri::command(async)]
fn get_pairs(hub: State<'_, Arc<Hub>>, ids: Vec<String>) -> AppResult<serde_json::Value> {
    hub.pairs(&ids)
}

/// People and worlds in my own history whose name has the words in it.
#[tauri::command(async)]
fn log_find(hub: State<'_, Arc<Hub>>, query: String) -> AppResult<serde_json::Value> {
    hub.log_find(&query)
}

/// Every stay of mine with one person or in one world, newest first.
#[tauri::command(async)]
fn log_stays(hub: State<'_, Arc<Hub>>, kind: String, id: String, name: String) -> AppResult<serde_json::Value> {
    hub.log_stays(&kind, &id, &name)
}

/// Where and when I meet one person. From this PC's history only.
#[tauri::command(async)]
fn get_habits(hub: State<'_, Arc<Hub>>, user_id: String) -> AppResult<serde_json::Value> {
    hub.habits(&user_id)
}

/// How my own worlds stood day by day: visits, favourites and the most people seen.
#[tauri::command(async)]
fn get_world_stats(hub: State<'_, Arc<Hub>>, days: i64) -> AppResult<serde_json::Value> {
    hub.world_stats(days)
}

/// My stars, by world or avatar id.
#[tauri::command(async)]
fn get_ratings(hub: State<'_, Arc<Hub>>) -> AppResult<std::collections::HashMap<String, i64>> {
    hub.ratings()
}

#[tauri::command(async)]
fn set_rating(hub: State<'_, Arc<Hub>>, item_id: String, stars: i64) -> AppResult<()> {
    hub.set_rating(&item_id, stars)
}

/// Item id to my tags on it.
#[tauri::command(async)]
fn get_tags(hub: State<'_, Arc<Hub>>) -> AppResult<std::collections::HashMap<String, Vec<String>>> {
    hub.tags()
}

/// Replaces my tags on an avatar or a world; gives back the tags as saved.
#[tauri::command(async)]
fn set_tags(hub: State<'_, Arc<Hub>>, item_id: String, tags: Vec<String>) -> AppResult<Vec<String>> {
    hub.set_tags(&item_id, &tags)
}

/// World id to how often I visited it and when last.
#[tauri::command(async)]
fn get_world_visits(hub: State<'_, Arc<Hub>>) -> AppResult<std::collections::HashMap<String, (i64, String)>> {
    hub.world_visits()
}

#[tauri::command(async)]
fn set_alerts(hub: State<'_, Arc<Hub>>, alerts: hub::Alerts) {
    hub.set_alerts(alerts);
}

#[tauri::command(async)]
fn set_note(hub: State<'_, Arc<Hub>>, user_id: String, note: String) -> AppResult<()> {
    hub.set_note(&user_id, &note)
}

#[tauri::command(async)]
fn get_noted(hub: State<'_, Arc<Hub>>) -> Vec<String> {
    hub.noted()
}

#[tauri::command(async)]
fn get_friend_log(hub: State<'_, Arc<Hub>>, limit: u32) -> AppResult<Vec<FeedItem>> {
    hub.friend_log(limit)
}

#[tauri::command]
async fn search_users(hub: State<'_, Arc<Hub>>, query: String) -> AppResult<Vec<Person>> {
    hub.search_users(&query).await
}

#[tauri::command]
async fn search_worlds(hub: State<'_, Arc<Hub>>, query: String) -> AppResult<Vec<WorldCard>> {
    hub.search_worlds(&query).await
}

#[tauri::command]
async fn search_avatars(hub: State<'_, Arc<Hub>>, provider: String, query: String) -> AppResult<Vec<AvatarCard>> {
    hub.search_avatars(&provider, &query).await
}

/// Puts an avatar on. Only ids that look like avatar ids are passed on. How it went is also told
/// in the headset while VRChat runs, since the window is out of sight there; `name` is only for
/// those words.
#[tauri::command]
async fn wear_avatar(app: AppHandle, hub: State<'_, Arc<Hub>>, avatar_id: String, name: Option<String>) -> AppResult<()> {
    if !avatar_id.starts_with("avtr_") {
        return Err(AppError::CannotDo("That is not an avatar id.".into()));
    }
    let result = hub.api.select_avatar(&avatar_id).await;
    let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    let (title, body) = match (&result, name) {
        (Ok(()), Some(name)) => (format!("Now wearing {name}"), "VRChat switched your avatar. The game may take a moment to load it.".to_string()),
        (Ok(()), None) => ("Avatar changed".to_string(), "VRChat switched your avatar. The game may take a moment to load it.".to_string()),
        (Err(e), Some(name)) => (format!("{name} did not go on"), e.to_string()),
        (Err(e), None) => ("Avatar not changed".to_string(), e.to_string()),
    };
    vr_notice(&app, VrAlert { kind: "invite".into(), label: "Avatar".into(), title, body }, false);
    result
}

#[tauri::command]
async fn invite_here(hub: State<'_, Arc<Hub>>, user_id: String, slot: Option<u8>) -> AppResult<()> {
    hub.invite_here(&user_id, slot).await
}

/// Opens a web address from someone's profile in the default browser. Web links only.
#[tauri::command]
fn open_link(url: String) -> AppResult<()> {
    let plain = url.starts_with("https://") || url.starts_with("http://");
    let clean = !url.chars().any(|c| c.is_control() || c == '"' || c == ' ');
    if !plain || !clean || url.len() > 2000 {
        return Err(AppError::CannotDo("That is not a web link this app will open.".into()));
    }
    std::process::Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", &url])
        .spawn()
        .map_err(|e| AppError::Storage(e.to_string()))?;
    Ok(())
}

#[tauri::command(async)]
fn get_handled(hub: State<'_, Arc<Hub>>, limit: u32) -> AppResult<Vec<Handled>> {
    hub.handled(limit)
}

#[tauri::command]
async fn notice_action(
    hub: State<'_, Arc<Hub>>,
    id: String,
    action: String,
    reply_kind: Option<String>,
    reply_data: Option<String>,
) -> AppResult<()> {
    hub.notice_action(
        &id,
        &action,
        reply_kind.as_deref().unwrap_or(""),
        reply_data.as_deref().unwrap_or(""),
    )
    .await
}

#[tauri::command(async)]
fn get_instances(hub: State<'_, Arc<Hub>>) -> Vec<Instance> {
    hub.instances()
}

#[tauri::command]
async fn invite_myself(hub: State<'_, Arc<Hub>>, location: String) -> AppResult<()> {
    hub.invite_myself(&location).await
}

/// Makes a new instance of a world and gives back where it is.
#[tauri::command]
async fn create_instance(hub: State<'_, Arc<Hub>>, world_id: String, access: String, region: String) -> AppResult<String> {
    hub.create_instance(&world_id, &access, &region).await
}

#[tauri::command]
async fn request_invite(hub: State<'_, Arc<Hub>>, user_id: String, slot: Option<u8>) -> AppResult<()> {
    hub.request_invite(&user_id, slot).await
}

#[tauri::command(async)]
fn get_worlds(hub: State<'_, Arc<Hub>>) -> Vec<World> {
    hub.worlds()
}

/// Asks for a world's name and picture if the app does not have them yet.
#[tauri::command(async)]
fn want_world(hub: State<'_, Arc<Hub>>, world_id: String) -> AppResult<()> {
    hub.ask_world(&world_id)
}

#[tauri::command(async)]
fn get_feed(hub: State<'_, Arc<Hub>>, limit: u32, before: Option<i64>) -> AppResult<Vec<FeedItem>> {
    hub.feed(limit, before)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Here {
    visit: Visit,
    people: Vec<Sighting>,
}

#[tauri::command(async)]
fn get_here(hub: State<'_, Arc<Hub>>) -> AppResult<Option<Here>> {
    Ok(hub.here()?.map(|(visit, people)| Here { visit, people }))
}

#[tauri::command(async)]
fn get_visits(hub: State<'_, Arc<Hub>>, limit: u32, before: Option<i64>) -> AppResult<Vec<Visit>> {
    hub.visits(limit, before)
}

#[tauri::command(async)]
fn get_visit_people(hub: State<'_, Arc<Hub>>, visit_id: i64) -> AppResult<Vec<Sighting>> {
    hub.visit_people(visit_id)
}

#[tauri::command(async)]
fn get_videos(hub: State<'_, Arc<Hub>>, limit: u32) -> AppResult<Vec<store::VideoPlay>> {
    hub.videos(limit)
}

#[tauri::command(async)]
fn get_videos_page(hub: State<'_, Arc<Hub>>, query: String, skip_arrival: bool, before_at: Option<String>, before_id: Option<i64>, limit: u32) -> AppResult<serde_json::Value> {
    let before = before_at.zip(before_id);
    let (rows, total) = hub.videos_page(&query, skip_arrival, before, limit)?;
    Ok(serde_json::json!({ "rows": rows, "total": total }))
}

/// My own numbers over a range: "7d", "30d", "year" or "all".
#[tauri::command(async)]
fn get_my_stats(hub: State<'_, Arc<Hub>>, range: String) -> AppResult<stats::MyStats> {
    hub.my_stats(&range, GAME_RUNNING.load(std::sync::atomic::Ordering::Relaxed))
}

/// Friends whose friendship day is today or in the next days.
#[tauri::command(async)]
fn get_friend_days(hub: State<'_, Arc<Hub>>, ahead: u32) -> AppResult<Vec<hub::FriendDay>> {
    hub.friend_days(ahead)
}

#[tauri::command(async)]
fn get_visit_videos(hub: State<'_, Arc<Hub>>, visit_id: i64) -> AppResult<Vec<store::VideoPlay>> {
    hub.visit_videos(visit_id)
}

#[tauri::command]
async fn get_video_titles(hub: State<'_, Arc<Hub>>, ids: Vec<String>) -> AppResult<Vec<store::VideoTitle>> {
    hub.video_titles(ids).await
}

/// Exports go to their own folder in Documents, next to the backups.
fn exports_root(app: &AppHandle) -> AppResult<std::path::PathBuf> {
    app.path()
        .document_dir()
        .map(|d| d.join("Anonymous VRX exports"))
        .map_err(|e| AppError::Storage(e.to_string()))
}

/// Writes the parts of the history asked for as CSV files and says where they went.
#[tauri::command]
async fn export_history(app: AppHandle, hub: State<'_, Arc<Hub>>, parts: Vec<String>) -> AppResult<export::ExportInfo> {
    let root = exports_root(&app)?;
    let hub = hub.inner().clone();
    // A long history takes a moment to write, so it runs aside.
    tauri::async_runtime::spawn_blocking(move || hub.export_csv(&root, &parts))
        .await
        .map_err(|e| AppError::Storage(e.to_string()))?
}

/// Opens the exports folder, or one export in it.
#[tauri::command]
fn open_exports_folder(app: AppHandle, path: Option<String>) -> AppResult<()> {
    let root = exports_root(&app)?;
    std::fs::create_dir_all(&root).map_err(|e| AppError::Storage(e.to_string()))?;
    let target = match path {
        Some(p) => {
            let canon_root = root.canonicalize().map_err(|e| AppError::Storage(e.to_string()))?;
            let canon = std::path::PathBuf::from(&p).canonicalize().map_err(|e| AppError::Storage(e.to_string()))?;
            if !canon.starts_with(&canon_root) || !canon.is_dir() {
                return Err(AppError::CannotDo("That folder is not one of the exports.".into()));
            }
            // Explorer gets the path as written, not the long form Windows uses inside.
            std::path::PathBuf::from(p.replace('/', "\\"))
        }
        None => root,
    };
    std::process::Command::new("explorer.exe")
        .arg(&target)
        .spawn()
        .map_err(|e| AppError::Storage(e.to_string()))?;
    Ok(())
}

#[tauri::command(async)]
fn get_pipeline_status(hub: State<'_, Arc<Hub>>) -> &'static str {
    hub.pipeline_status()
}

/// Backups go to a folder in Documents so they are easy to find and copy elsewhere.
fn backups_root(app: &AppHandle) -> AppResult<std::path::PathBuf> {
    app.path()
        .document_dir()
        .map(|d| d.join("Anonymous VRX backups"))
        .map_err(|e| AppError::Storage(e.to_string()))
}

/// Copying a large history takes a while, so it runs aside and the window keeps answering.
#[tauri::command]
async fn backup_now(app: AppHandle, hub: State<'_, Arc<Hub>>, settings: String) -> AppResult<backup::BackupInfo> {
    let root = backups_root(&app)?;
    let hub = hub.inner().clone();
    tauri::async_runtime::spawn_blocking(move || hub.backup(&root, &settings, "backup"))
        .await
        .map_err(|e| AppError::Storage(e.to_string()))?
}

/// Where VRCX keeps its history on this PC.
fn vrcx_file() -> Option<std::path::PathBuf> {
    std::env::var_os("APPDATA").map(|d| std::path::PathBuf::from(d).join("VRCX").join("VRCX.sqlite3"))
}

/// Whether VRCX's history file is on this PC.
#[tauri::command]
fn vrcx_found() -> bool {
    vrcx_file().is_some_and(|f| f.is_file())
}

/// Brings VRCX's history for the signed-in account into this app. Makes a backup first.
#[tauri::command]
async fn import_vrcx(app: AppHandle, hub: State<'_, Arc<Hub>>, settings: String, by_itself: Option<bool>, since: Option<String>) -> AppResult<serde_json::Value> {
    let file = vrcx_file().ok_or_else(|| AppError::CannotDo("VRCX's history file was not found on this PC.".into()))?;
    let backups = backups_root(&app)?;
    let hub = hub.inner().clone();
    // Reading a long history takes a while, so it runs aside. Lists that ask the history
    // for something wait until it is done; the hourly look only reads what is new.
    tauri::async_runtime::spawn_blocking(move || hub.import_vrcx(&file, &backups, &settings, by_itself.unwrap_or(false), since.as_deref()))
        .await
        .map_err(|e| AppError::Storage(e.to_string()))?
}

/// A short text that changes whenever VRCX has written to its history: the size and the time
/// of its file and of the file it writes to first. Empty when VRCX is not on this PC.
#[tauri::command]
fn vrcx_stamp() -> String {
    let Some(file) = vrcx_file().filter(|f| f.is_file()) else {
        return String::new();
    };
    let mut wal = file.clone().into_os_string();
    wal.push("-wal");
    [file, std::path::PathBuf::from(wal)]
        .iter()
        .filter_map(|f| std::fs::metadata(f).ok())
        .map(|m| {
            let at = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
            format!("{}:{}", m.len(), at)
        })
        .collect::<Vec<_>>()
        .join("|")
}

// ---------- updates ----------

/// Where a newer version is announced. Only the edition published on GitHub has such a place; the full
/// edition is passed on by hand and never replaces itself.
const UPDATE_FEED: &str = "https://github.com/ElDiablo66/anonymous-vrx-app/releases/latest/download/latest.json";

/// An update that was found and waits to be installed.
static UPDATE_FOUND: Mutex<Option<tauri_plugin_updater::Update>> = Mutex::new(None);

/// The running version, and whether this edition can update itself.
#[tauri::command]
fn update_info(app: AppHandle) -> serde_json::Value {
    serde_json::json!({ "version": app.package_info().version.to_string(), "supported": !UPDATE_FEED.is_empty() })
}

/// Asks whether a newer version is out. Gives its number and notes, or nothing.
#[tauri::command]
async fn update_check(app: AppHandle) -> AppResult<Option<serde_json::Value>> {
    use tauri_plugin_updater::UpdaterExt;
    if UPDATE_FEED.is_empty() {
        return Ok(None);
    }
    let problem = |e: tauri_plugin_updater::Error| AppError::Network(e.to_string());
    let feed = UPDATE_FEED.parse().map_err(|_| AppError::CannotDo("The address for updates is not valid.".into()))?;
    let found = app
        .updater_builder()
        .endpoints(vec![feed])
        .map_err(problem)?
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(problem)?
        .check()
        .await
        .map_err(problem)?;
    let told = found.as_ref().map(|u| serde_json::json!({ "version": u.version, "notes": u.body.clone().unwrap_or_default() }));
    *UPDATE_FOUND.lock().unwrap() = found;
    Ok(told)
}

/// Downloads the update that was found, checks its signature and installs it. The installer
/// closes the app and opens the new version.
#[tauri::command]
async fn update_install(app: AppHandle) -> AppResult<()> {
    // A copy: when the download fails, the update is still there for a second try.
    let Some(mut update) = UPDATE_FOUND.lock().unwrap().clone() else {
        return Err(AppError::CannotDo("No update is waiting. Look for one first.".into()));
    };
    // The twenty seconds were for asking; the download takes as long as the line needs.
    update.timeout = None;
    let progress = app.clone();
    let mut got: u64 = 0;
    let bytes = update
        .download(
            move |chunk, total| {
                got += chunk as u64;
                let _ = progress.emit("update:progress", serde_json::json!({ "done": got, "total": total.unwrap_or(0) }));
            },
            || {},
        )
        .await
        .map_err(|e| AppError::Network(e.to_string()))?;
    discord::show(None);
    update.install(bytes).map_err(|e| AppError::CannotDo(format!("The update could not be installed: {e}")))
}

// ---------- drawing without the graphics card ----------

// ---------- when the app cannot go on ----------

/// The app's own folder, worked out by hand, since it is needed before Tauri starts: the same
/// folder Tauri uses.
fn own_folder() -> Option<std::path::PathBuf> {
    std::env::var_os("APPDATA").map(|d| std::path::PathBuf::from(d).join("app.anonymousvrx.desktop"))
}

/// Where the last crash is written down.
fn crash_file() -> Option<std::path::PathBuf> {
    own_folder().map(|d| d.join("crash log.txt"))
}

/// A plain Windows message box, which works without the app's window.
fn tell_windows(title: &str, text: &str) {
    // Loaded from Windows itself, so both editions can do it without another library.
    #[cfg(windows)]
    unsafe {
        type MessageBoxW = unsafe extern "system" fn(isize, *const u16, *const u16, u32) -> i32;
        // MB_OK | MB_ICONERROR | MB_SETFOREGROUND
        const STYLE: u32 = 0x0000_0000 | 0x0000_0010 | 0x0001_0000;
        let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let (title, text) = (wide(title), wide(text));
        if let Ok(user32) = libloading::Library::new("user32.dll") {
            if let Ok(show) = user32.get::<MessageBoxW>(b"MessageBoxW ") {
                show(0, text.as_ptr(), title.as_ptr(), STYLE);
            }
        }
    }
    #[cfg(not(windows))]
    let _ = (title, text);
}

/// The words written down for one crash: what went wrong, where, the version and Windows.
fn crash_words(what: &str, at: &str, when: &str) -> String {
    format!(
        "Anonymous VRX {} closed because something went wrong.\r\nWhen: {when}\r\nWhat: {what}\r\nWhere: {at}\r\nWindows: {}\r\n",
        env!("CARGO_PKG_VERSION"),
        std::env::var("OS").unwrap_or_default()
    )
}

/// A crash used to end the app without a word. Now what went wrong is written to a small file
/// and shown in a message box first, so it can be sent to me. No account details are in it.
fn keep_crashes() {
    std::panic::set_hook(Box::new(|info| {
        let what = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "no details".into());
        let at = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let when = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();
        let words = crash_words(&what, &at, &when);
        let saved = crash_file().and_then(|file| {
            std::fs::create_dir_all(file.parent()?).ok()?;
            std::fs::write(&file, &words).ok()?;
            Some(file)
        });
        let mut text = format!("Anonymous VRX had to close.\n\n{what}");
        if what.contains("WebView2") || what.contains("Failed to setup app") {
            text.push_str("\n\nThis is usually Microsoft Edge WebView2, the part of Windows that draws the app's window. Installing the Evergreen WebView2 Runtime from Microsoft fixes it: https://developer.microsoft.com/microsoft-edge/webview2/");
        }
        if let Some(file) = saved {
            text.push_str(&format!("\n\nWhat happened is written in:\n{}", file.display()));
        }
        tell_windows("Anonymous VRX", &text);
    }));
}

/// The window is drawn by Microsoft Edge WebView2. When Windows has none (some "debloat" tools
/// remove it), the app could not even open its window and closed without a word; now it says
/// what is missing and where to get it, and does not start.
fn window_parts_work() -> bool {
    #[cfg(windows)]
    if let Err(e) = tauri::webview_version() {
        tell_windows(
            "Anonymous VRX",
            &format!(
                "Anonymous VRX cannot open its window: Microsoft Edge WebView2 is missing or broken on this PC ({e}).\n\nInstall the Evergreen WebView2 Runtime from Microsoft, then start the app again:\nhttps://developer.microsoft.com/microsoft-edge/webview2/"
            ),
        );
        return false;
    }
    true
}

/// The file whose presence means "draw without the graphics card". It has to be found before
/// the window exists, so its place is worked out by hand: the same folder Tauri uses.
fn no_gpu_flag() -> Option<std::path::PathBuf> {
    own_folder().map(|d| d.join("draw without graphics card"))
}

/// Tells the window's browser part to leave the graphics card alone, when I asked for that.
/// Read once, before the window is made; a change counts from the next start.
fn apply_gpu_choice() {
    const NAME: &str = "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS";
    let wanted = no_gpu_flag().is_some_and(|f| f.is_file());
    NO_GPU_NOW.store(wanted, std::sync::atomic::Ordering::Relaxed);
    // A restart from inside the app hands its own settings on to the new copy, so the
    // argument is taken out first and only put back when it is still wanted.
    let mut args: Vec<String> = std::env::var(NAME).unwrap_or_default().split_whitespace().filter(|a| *a != "--disable-gpu").map(str::to_string).collect();
    if wanted {
        args.push("--disable-gpu".into());
    }
    if args.is_empty() {
        std::env::remove_var(NAME);
    } else {
        std::env::set_var(NAME, args.join(" "));
    }
}

/// How this run of the app draws: true when it started without the graphics card.
static NO_GPU_NOW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The marker a restart from inside the app leaves for the new copy: show the window, even
/// when the first copy was started by Windows to sit in the tray.
const SHOW_AFTER_RESTART: &str = "ANONYMOUS_VRX_SHOW";

/// `wanted` is the choice from the next start on, `now` is how this run draws.
#[tauri::command]
fn get_no_gpu() -> serde_json::Value {
    serde_json::json!({
        "wanted": no_gpu_flag().is_some_and(|f| f.is_file()),
        "now": NO_GPU_NOW.load(std::sync::atomic::Ordering::Relaxed),
    })
}

#[tauri::command]
fn set_no_gpu(on: bool) -> AppResult<()> {
    let file = no_gpu_flag().ok_or_else(|| AppError::Storage("Windows did not say where the app's files are.".into()))?;
    if on {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| AppError::Storage(e.to_string()))?;
        }
        std::fs::write(&file, b"The app draws without the graphics card while this file is here.").map_err(|e| AppError::Storage(e.to_string()))
    } else if file.is_file() {
        std::fs::remove_file(&file).map_err(|e| AppError::Storage(e.to_string()))
    } else {
        Ok(())
    }
}

/// Closes the app and opens it again, so a choice that counts from the start takes hold.
#[tauri::command]
fn restart_app(app: AppHandle) {
    discord::show(None);
    std::env::set_var(SHOW_AFTER_RESTART, "1");
    app.restart();
}

// ---------- VRChat's download cache ----------

fn game_folder() -> AppResult<std::path::PathBuf> {
    gamecache::game_folder().ok_or_else(|| AppError::CannotDo("Windows did not say where VRChat keeps its files.".into()))
}

/// How big VRChat's cache is. Counting tens of gigabytes takes a moment, so it runs aside.
#[tauri::command]
async fn game_cache() -> AppResult<gamecache::CacheInfo> {
    let game = game_folder()?;
    tauri::async_runtime::spawn_blocking(move || gamecache::measure(&game)).await.map_err(|e| AppError::Storage(e.to_string()))
}

/// Clears VRChat's cache: all of it, or only versions a newer download replaced. Refused while
/// the game runs, because it would be reading those files.
#[tauri::command]
async fn clear_game_cache(old_only: bool) -> AppResult<gamecache::Cleared> {
    let game = game_folder()?;
    tauri::async_runtime::spawn_blocking(move || {
        if !game_surely_closed() {
            return Err(AppError::CannotDo("Close VRChat first. It looks like it is still running or just starting, and it reads the cache then.".into()));
        }
        gamecache::clear(&game, old_only, &still_closed())
    })
    .await
    .map_err(|e| AppError::Storage(e.to_string()))?
}

/// Sets how big VRChat lets its cache grow, in gigabytes. VRChat reads it at its next start.
#[tauri::command]
fn set_game_cache_limit(gb: u64) -> AppResult<u64> {
    gamecache::set_limit(&game_folder()?, gb)
}

// ---------- notices in the headset ----------

static VR_RULES: Mutex<Option<vr::Rules>> = Mutex::new(None);

#[tauri::command]
fn set_vr_notices(rules: vr::Rules) {
    *VR_RULES.lock().unwrap() = Some(rules);
}

/// One thing to tell me in the headset. `kind` picks the colour of the app's own panel
/// (friend, invite, room, day, battery, tracking, steamvr) and `label` names it there.
pub struct VrAlert {
    pub kind: String,
    pub label: String,
    pub title: String,
    pub body: String,
}

/// Hands a notice to the one place in the headset that takes it first (see `vr::deliver`):
/// XSOverlay, OVR Toolkit (in view or on the wrist, one of them) or the app's own panel, in the
/// order set. The places after it are not tried, so one alert makes one notice. Quiet hours hold
/// here too.
/// Notices about VRChat only go while VRChat runs; SteamVR's own news goes whenever SteamVR does.
///
/// SteamVR's own news only comes when its alerts are switched on, and the app is in SteamVR then,
/// so its own panel is always a place for it, even with the notice panel switched off. The one
/// exception is SteamVR closing: the panel closes with it, so that goes to the other programs and
/// to Windows.
fn vr_notice(app: &AppHandle, alert: VrAlert, steamvr: bool) {
    let Some(mut rules) = *VR_RULES.lock().unwrap() else {
        return;
    };
    if steamvr {
        rules.own = true;
    }
    if !rules.any() || (!steamvr && !GAME_RUNNING.load(std::sync::atomic::Ordering::Relaxed)) || in_quiet_time() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let closing = steamvr && alert.kind == "steamvr";
        let own_ready = rules.own && !closing && app.try_state::<overlay::Overlay>().is_some_and(|o| o.connected());
        if vr::deliver(&rules, &alert.title, &alert.body, own_ready, vr::XS_URL, vr::OVR_URL).await == Some(vr::Target::Own) {
            // The window draws the panel and hands the picture back through `vr_overlay_notice`.
            let _ = app.emit("vr:draw", serde_json::json!({ "kind": alert.kind, "label": alert.label, "title": alert.title, "body": alert.body }));
        }
    });
}

#[tauri::command]
fn notify_vr(app: AppHandle, title: String, body: String, kind: Option<String>, label: Option<String>) {
    let alert = VrAlert { kind: kind.unwrap_or_else(|| "friend".into()), label: label.unwrap_or_default(), title, body };
    vr_notice(&app, alert, false);
}

// ---------- the app's own panels in SteamVR ----------

fn steamvr_running() -> bool {
    is_running("vrserver.exe")
}

/// A PNG's height divided by its width, from its header.
fn png_aspect(bytes: &[u8]) -> Option<f32> {
    if bytes.len() < 24 || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    (width > 0 && height > 0).then(|| height as f32 / width as f32)
}

/// Keeps a picture the window drew as a file SteamVR can read, and gives back where it is and
/// its height over its width. Each picture gets a new name, because SteamVR does not look again
/// at a name it knows. Of each kind only this picture and the one before stay: SteamVR may still
/// be reading that one.
fn keep_panel_picture(kind: &str, png_base64: &str) -> AppResult<(std::path::PathBuf, f32)> {
    use base64::Engine;
    static COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(png_base64.trim_start_matches("data:image/png;base64,"))
        .map_err(|_| AppError::CannotDo("The panel's picture arrived damaged.".into()))?;
    if bytes.len() > 8 * 1024 * 1024 || !bytes.starts_with(b"\x89PNG") {
        return Err(AppError::CannotDo("That is not a panel picture.".into()));
    }
    let aspect = png_aspect(&bytes).ok_or_else(|| AppError::CannotDo("That is not a panel picture.".into()))?;
    let dir = std::env::temp_dir().join("anonymous-vrx panels");
    std::fs::create_dir_all(&dir).map_err(|e| AppError::Storage(e.to_string()))?;
    let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mine = format!("{kind}-{}-", std::process::id());
    if let Ok(old) = std::fs::read_dir(&dir) {
        let mut same: Vec<(u32, std::path::PathBuf)> = Vec::new();
        for entry in old.filter_map(Result::ok) {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(number) = name.strip_prefix(&mine).and_then(|rest| rest.strip_suffix(".png")).and_then(|n| n.parse::<u32>().ok()) {
                same.push((number, entry.path()));
            } else if name.starts_with(&format!("{kind}-")) {
                // Left by an earlier run of the app.
                let _ = std::fs::remove_file(entry.path());
            }
        }
        same.sort_by_key(|(number, _)| *number);
        same.pop();
        for (_, path) in same {
            let _ = std::fs::remove_file(path);
        }
    }
    let file = dir.join(format!("{mine}{n}.png"));
    std::fs::write(&file, bytes).map_err(|e| AppError::Storage(e.to_string()))?;
    Ok((file, aspect))
}

/// Whether the app should be in SteamVR at all (a panel, the dashboard tab or SteamVR's alerts
/// is switched on), and whether battery, tracking and SteamVR closing are told.
#[tauri::command]
fn set_vr_overlay(panels: State<'_, overlay::Overlay>, on: bool, watch: Option<bool>) {
    panels.watch(watch.unwrap_or(false));
    panels.wanted(on);
}

/// Shows a notice the window drew, in front of the eyes, for a few seconds.
#[tauri::command]
fn vr_overlay_notice(panels: State<'_, overlay::Overlay>, png: String, seconds: f32, alpha: f32, above: Option<bool>) -> AppResult<()> {
    panels.notice(keep_panel_picture("notice", &png)?.0, seconds, alpha, above.unwrap_or(true));
    Ok(())
}

/// Puts the feed the window drew on an arm, or takes it off with no picture. `shift` is in
/// metres along the arm.
#[tauri::command]
fn vr_overlay_wrist(panels: State<'_, overlay::Overlay>, png: Option<String>, hand: overlay::Hand, spot: Option<overlay::Spot>, width: f32, alpha: f32, shift: Option<f32>) -> AppResult<()> {
    match png {
        Some(png) => {
            let (picture, aspect) = keep_panel_picture("wrist", &png)?;
            panels.wrist(Some(overlay::WristPanel { picture, hand, spot: spot.unwrap_or(overlay::Spot::Forearm), width, alpha, aspect, shift: shift.unwrap_or(0.0) }));
        }
        None => panels.wrist(None),
    }
    Ok(())
}

/// The tab in SteamVR's dashboard: its picture and, the first time, its small icon. No picture
/// takes the tab away.
#[tauri::command]
fn vr_overlay_dashboard(panels: State<'_, overlay::Overlay>, png: Option<String>, thumb: Option<String>) -> AppResult<()> {
    match png {
        Some(png) => {
            let (main, aspect) = keep_panel_picture("dash", &png)?;
            let icon = match thumb {
                Some(t) => Some(keep_panel_picture("thumb", &t)?.0),
                None => None,
            };
            panels.dashboard(Some((main, aspect, icon)));
        }
        None => panels.dashboard(None),
    }
    Ok(())
}

#[tauri::command]
fn vr_overlay_status(panels: State<'_, overlay::Overlay>) -> String {
    panels.status()
}

/// Sends one of SteamVR's own notifications and says how SteamVR took it.
#[tauri::command]
async fn vr_overlay_try_native(panels: State<'_, overlay::Overlay>) -> Result<String, ()> {
    Ok(panels.try_native())
}

/// Sends a sample notice now, game running or not, and says how each overlay program took it.
#[tauri::command]
async fn test_vr_notice(rules: vr::Rules) -> Vec<vr::Sent> {
    vr::send(rules, "Anonymous VRX".into(), "This is what a notice looks like in the headset.".into()).await
}


/// Makes the daily automatic backup when it is due. Nothing happens otherwise. Runs aside like
/// `backup_now`, so the window keeps answering while it copies.
#[tauri::command]
async fn auto_backup(app: AppHandle, hub: State<'_, Arc<Hub>>, settings: String, keep: usize) -> AppResult<Option<backup::BackupInfo>> {
    let root = backups_root(&app)?;
    let hub = hub.inner().clone();
    tauri::async_runtime::spawn_blocking(move || hub.auto_backup(&root, &settings, keep))
        .await
        .map_err(|e| AppError::Storage(e.to_string()))?
}

/// The backups Settings offers: the signed-in account's own and the older ones with no account
/// named, so only what can be restored here is listed.
#[tauri::command(async)]
fn list_backups(app: AppHandle, hub: State<'_, Arc<Hub>>) -> AppResult<Vec<backup::BackupInfo>> {
    Ok(hub.list_backups(&backups_root(&app)?))
}

/// Lines a backup up to replace the current history at the next start. It makes a backup of the
/// current history first, so it runs aside like `backup_now`.
#[tauri::command]
async fn restore_backup(app: AppHandle, hub: State<'_, Arc<Hub>>, name: String, settings: String) -> AppResult<String> {
    let root = backups_root(&app)?;
    let hub = hub.inner().clone();
    tauri::async_runtime::spawn_blocking(move || hub.restore_backup(&root, &name, &settings))
        .await
        .map_err(|e| AppError::Storage(e.to_string()))?
}

/// Runs a small Windows tool without flashing a console window.
fn quiet_run(program: &str, args: &[&str]) -> Option<std::process::Output> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
}

/// Runs a small Windows tool without flashing a console window, and returns what it printed.
fn quiet_output(program: &str, args: &[&str]) -> Option<String> {
    quiet_run(program, args).map(|out| String::from_utf8_lossy(&out.stdout).to_string())
}

/// `quiet_output` for a tool whose answer only counts when it says it finished well: nothing
/// when it could not start or ended with a failure.
fn quiet_answer(program: &str, args: &[&str]) -> Option<String> {
    quiet_run(program, args)
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).to_string())
}

/// Whether a program with this file name is running, or nothing when Windows did not say.
fn running_state(image: &str) -> Option<bool> {
    let filter = format!("IMAGENAME eq {image}");
    let text = quiet_output("tasklist", &["/FI", &filter, "/NH", "/FO", "CSV"])?;
    if text.trim().is_empty() {
        return None;
    }
    Some(text.to_ascii_lowercase().contains(&image.to_ascii_lowercase()))
}

/// When this app last started VRChat, so nothing touches the game's files while it comes up.
static LAUNCHED_AT: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// True only when VRChat and its launcher are known to be closed and this app did not just
/// start the game. "Could not tell" counts as running.
fn game_surely_closed() -> bool {
    let just_started = LAUNCHED_AT.lock().unwrap().is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(120));
    if just_started || running_state("VRChat.exe") != Some(false) {
        return false;
    }
    // The launcher and the anti-cheat starter have names other programs use too, so a match
    // only counts when it runs from VRChat's own folder.
    let helpers: Vec<&str> = ["start_protected_game", "launch"]
        .into_iter()
        .filter(|name| running_state(&format!("{name}.exe")) != Some(false))
        .collect();
    helpers.is_empty() || helpers_outside_game(&helpers)
}

/// True when every running program with one of these names (without `.exe`) is known to run
/// from somewhere other than VRChat's install folder. False when any of them is VRChat's, when
/// its place or VRChat's folder could not be read, or when PowerShell failed or broke off.
fn helpers_outside_game(names: &[&str]) -> bool {
    let Some(folder) = vrchat_launcher().and_then(|l| l.parent().map(|p| p.to_string_lossy().to_lowercase())) else {
        return false;
    };
    // The last line says PowerShell got to the end, so a run that broke off is not read as
    // "nothing running".
    let script = format!(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8; Get-Process -Name {} -ErrorAction SilentlyContinue | ForEach-Object {{ if ($_.Path) {{ $_.Path }} else {{ '?' }} }}; '{HELPERS_LISTED}'",
        names.join(",")
    );
    let Some(text) = quiet_answer("powershell", &["-NoProfile", "-NonInteractive", "-Command", &script]) else {
        return false;
    };
    helpers_answer_elsewhere(&text, &folder, || names.iter().all(|name| running_state(&format!("{name}.exe")) == Some(false)))
}

/// The line PowerShell prints once it has listed the helper programs.
const HELPERS_LISTED: &str = "listed";

/// Reads PowerShell's list of helper program paths. True only when the list is whole and every
/// path lies outside VRChat's folder (lower case). An empty list means the programs closed in
/// between, which only counts once `all_closed_now` agrees, since PowerShell may simply have
/// found nothing it could read.
fn helpers_answer_elsewhere(text: &str, folder: &str, all_closed_now: impl FnOnce() -> bool) -> bool {
    let mut paths: Vec<String> = text.lines().map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty()).collect();
    if paths.pop().as_deref() != Some(HELPERS_LISTED) {
        return false;
    }
    if paths.is_empty() {
        return all_closed_now();
    }
    paths.iter().all(|p| helper_elsewhere(p, folder))
}

/// Whether a helper program's path (lower case, "?" when Windows would not say) lies outside
/// VRChat's install folder (lower case).
fn helper_elsewhere(path: &str, folder: &str) -> bool {
    let folder = folder.trim_end_matches('\\');
    path != "?" && !folder.is_empty() && !path.starts_with(&format!("{folder}\\"))
}

/// `game_surely_closed` for use while clearing: asked again every two seconds at most, since
/// each look at the process list takes a moment and a cache has thousands of folders. Made
/// right after a look that said "closed", so it starts from that.
fn still_closed() -> impl Fn() -> bool {
    let last = std::cell::Cell::new((std::time::Instant::now(), true));
    move || {
        let (at, closed) = last.get();
        if !closed || at.elapsed() < std::time::Duration::from_secs(2) {
            return closed;
        }
        let now = game_surely_closed();
        last.set((std::time::Instant::now(), now));
        now
    }
}

/// Whether a program with this file name is running. Reads the process list only.
fn is_running(image: &str) -> bool {
    let filter = format!("IMAGENAME eq {image}");
    quiet_output("tasklist", &["/FI", &filter, "/NH", "/FO", "CSV"])
        .is_some_and(|text| text.to_ascii_lowercase().contains(&image.to_ascii_lowercase()))
}

/// VRChat's own launcher, as registered for its `vrchat://` links.
fn vrchat_launcher() -> Option<std::path::PathBuf> {
    for root in ["HKCU", "HKLM"] {
        let key = format!("{root}\\Software\\Classes\\VRChat\\shell\\open\\command");
        let Some(text) = quiet_output("reg", &["query", &key, "/ve"]) else { continue };
        if let Some(path) = vrchat::model::first_quoted_exe(&text) {
            let path = std::path::PathBuf::from(path);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

/// Starts VRChat in an instance. `mode` is `auto`, `desktop` or `vr`.
/// Returns `running`, `desktop` or `vr` to say what was done. It asks Windows about running
/// programs and the registry, so it runs off the window's own thread.
#[tauri::command(async)]
fn launch_instance(location: String, mode: String) -> AppResult<&'static str> {
    let url = vrchat::model::launch_link(&location)
        .ok_or_else(|| AppError::CannotDo("That is not an instance VRChat can be started in.".into()))?;
    *LAUNCHED_AT.lock().unwrap() = Some(std::time::Instant::now());
    let hand_over = |url: &str| {
        std::process::Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
            .map(|_| ())
            .map_err(|e| AppError::Storage(e.to_string()))
    };
    // An open game takes the link itself and moves there; no new start, no mode to pick.
    if is_running("VRChat.exe") {
        hand_over(&url)?;
        return Ok("running");
    }
    let desktop = match mode.as_str() {
        "desktop" => true,
        "vr" => false,
        // Auto: VR only when SteamVR is already up.
        _ => !is_running("vrserver.exe"),
    };
    if !desktop {
        hand_over(&url)?;
        return Ok("vr");
    }
    let launcher = vrchat_launcher().ok_or_else(|| {
        AppError::CannotDo(
            "Could not find VRChat's launcher to start in desktop mode. Start VRChat yourself first, or switch \"Start VRChat in\" to VR in Settings.".into(),
        )
    })?;
    std::process::Command::new(launcher)
        .args(["--no-vr", &url])
        .spawn()
        .map_err(|e| AppError::Storage(e.to_string()))?;
    Ok("desktop")
}

/// When Windows notifications stay quiet. Times are minutes after midnight, local time.
#[derive(Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuietRules {
    hours: bool,
    from: u16,
    to: u16,
    /// Stay quiet while VRChat is running.
    in_game: bool,
}

static QUIET: Mutex<Option<QuietRules>> = Mutex::new(None);
static GAME_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// True when `minute` of the day falls in the quiet stretch, which may run past midnight.
fn in_quiet_hours(from: u16, to: u16, minute: u16) -> bool {
    if from == to {
        return false;
    }
    if from < to {
        (from..to).contains(&minute)
    } else {
        minute >= from || minute < to
    }
}

/// Whether a notification may be shown right now.
#[tauri::command]
fn may_notify() -> bool {
    let rules = QUIET.lock().unwrap().unwrap_or_default();
    if rules.in_game && GAME_RUNNING.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    !in_quiet_time()
}

/// Whether the clock is inside the quiet hours I set.
fn in_quiet_time() -> bool {
    let rules = QUIET.lock().unwrap().unwrap_or_default();
    if !rules.hours {
        return false;
    }
    use chrono::Timelike;
    let now = chrono::Local::now();
    in_quiet_hours(rules.from, rules.to, (now.hour() * 60 + now.minute()) as u16)
}

#[tauri::command]
fn set_quiet(rules: QuietRules) {
    *QUIET.lock().unwrap() = Some(rules);
}

/// What the window asked for about VRChat starting and stopping.
#[derive(Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GameWatch {
    /// Bring the window up when VRChat starts.
    open_with_game: bool,
    /// After a crash: `off`, `ask` or `auto`.
    rejoin: String,
    /// How VRChat is started again: `auto`, `desktop` or `vr`.
    launch_mode: String,
    /// Empty VRChat's download cache each time the game closes.
    #[serde(default)]
    clear_cache: bool,
}

static GAME_WATCH: Mutex<Option<GameWatch>> = Mutex::new(None);

#[tauri::command]
fn set_game_watch(watch: GameWatch) {
    *GAME_WATCH.lock().unwrap() = Some(watch);
}

/// The argument Windows starts the app with at sign-in, so it stays in the tray.
const TRAY_FLAG: &str = "--tray";

/// Whether the app starts when I sign in to Windows.
#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// Turns starting with Windows on or off. It is one entry under my own account's Run key.
#[tauri::command]
fn set_autostart(app: AppHandle, on: bool) -> AppResult<()> {
    use tauri_plugin_autostart::ManagerExt;
    let auto = app.autolaunch();
    let done = if on { auto.enable() } else { auto.disable() };
    // Switching off something that was never on is not a problem.
    match done {
        Err(e) if on || auto.is_enabled().unwrap_or(false) => Err(AppError::Storage(e.to_string())),
        _ => Ok(()),
    }
}

/// How often the process list is looked at for VRChat.
const GAME_LOOK: std::time::Duration = std::time::Duration::from_secs(5);
/// After the game is gone, how long the log reader gets to see a goodbye line.
const QUIT_GRACE: std::time::Duration = std::time::Duration::from_secs(5);
/// The game is not started again by itself more often than this, so a game that keeps
/// crashing on the way in does not loop.
const REJOIN_GAP: std::time::Duration = std::time::Duration::from_secs(180);

/// Watches for VRChat starting and stopping. A stop with no goodbye line in the log while I
/// was in an instance counts as a crash; what happens then is the window's choice.
fn watch_game(app: AppHandle) {
    let mut was = is_running("VRChat.exe");
    GAME_RUNNING.store(was, std::sync::atomic::Ordering::Relaxed);
    let mut last_rejoin: Option<std::time::Instant> = None;
    loop {
        std::thread::sleep(GAME_LOOK);
        let running = is_running("VRChat.exe");
        GAME_RUNNING.store(running, std::sync::atomic::Ordering::Relaxed);
        let watch = GAME_WATCH.lock().unwrap().clone().unwrap_or_default();
        if running && !was {
            let _ = app.emit("game:started", ());
            if watch.open_with_game {
                show_window(&app);
            }
        }
        if !running && was {
            std::thread::sleep(QUIT_GRACE);
            // A clean exit closes the stay in the history. One still open means no goodbye.
            let open = app.state::<Arc<Hub>>().here().ok().flatten().map(|(visit, _)| visit);
            // The log itself is asked too: a goodbye line there means a normal exit, even if
            // the history missed it. A log that cannot be read is never called a crash, and a
            // missing goodbye gets a second look in case the last lines were still being written.
            let look = || gamelog::default_dir().and_then(|dir| gamelog::said_goodbye(&dir));
            let mut goodbye = look();
            if goodbye == Some(false) {
                std::thread::sleep(QUIT_GRACE);
                goodbye = look();
            }
            if let Some(visit) = open.filter(|v| goodbye == Some(false) && vrchat::model::launch_link(&v.location).is_some()) {
                if !is_running("VRChat.exe") {
                    crashed(&app, &watch, &visit, &mut last_rejoin);
                    app.state::<Arc<Hub>>().game_gone();
                }
            }
            // The cache is emptied when I asked for that, but not while the game is being
            // started again after a crash: it would be reading those files.
            let starting_again = last_rejoin.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(120));
            if watch.clear_cache && !starting_again && game_surely_closed() {
                if let Some(Ok(done)) = gamecache::game_folder().map(|game| gamecache::clear(&game, false, &still_closed())) {
                    if done.folders > 0 {
                        let _ = app.emit("cache:cleared", serde_json::json!({ "bytes": done.bytes }));
                    }
                }
            }
        }
        was = running;
    }
}

fn crashed(app: &AppHandle, watch: &GameWatch, visit: &Visit, last_rejoin: &mut Option<std::time::Instant>) {
    if watch.rejoin == "off" || watch.rejoin.is_empty() {
        return;
    }
    let world = if visit.world_name.is_empty() { "the world you were in".to_string() } else { visit.world_name.clone() };
    let again = watch.rejoin == "auto" && last_rejoin.is_none_or(|t| t.elapsed() >= REJOIN_GAP);
    let rejoined = again && launch_instance(visit.location.clone(), watch.launch_mode.clone()).is_ok();
    if rejoined {
        *last_rejoin = Some(std::time::Instant::now());
    }
    let _ = app.emit(
        "game:crashed",
        serde_json::json!({ "location": visit.location, "worldName": world, "rejoined": rejoined }),
    );
    use tauri_plugin_notification::NotificationExt;
    let body = if rejoined { format!("Starting it again in {world}.") } else { format!("Open Anonymous VRX to go back to {world}.") };
    // The note inside the window always shows; the Windows notification keeps to the quiet times.
    if may_notify() {
        let _ = app.notification().builder().title("VRChat closed without warning").body(body).show();
    }
}

/// Opens VRChat's own join page for an instance in the default browser.
#[tauri::command]
fn open_join_page(location: String) -> AppResult<()> {
    let url = vrchat::model::join_page(&location)
        .ok_or_else(|| AppError::CannotDo("That is not an instance with a join page.".into()))?;
    std::process::Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", &url])
        .spawn()
        .map_err(|e| AppError::Storage(e.to_string()))?;
    Ok(())
}

/// Where VRChat's camera saves photos: the folder named in VRChat's `config.json` when there
/// is one, or Pictures\VRChat.
fn photos_root(app: &AppHandle) -> AppResult<std::path::PathBuf> {
    if let Some(own) = gamecache::game_folder().and_then(|game| photos::picture_folder(&game)) {
        return Ok(own);
    }
    app.path()
        .picture_dir()
        .map(|d| d.join("VRChat"))
        .map_err(|e| AppError::Storage(e.to_string()))
}

/// Lets the window show pictures from a photo folder VRChat was told to use instead of
/// Pictures\VRChat, which the app's settings already allow.
fn allow_photo_folder(app: &AppHandle, root: &std::path::Path) {
    let usual = app.path().picture_dir().ok().map(|d| d.join("VRChat"));
    if usual.as_deref() != Some(root) && root.is_dir() {
        let _ = app.asset_protocol_scope().allow_directory(root, true);
    }
}

/// VRChat's photos on this PC, newest first, with the world and people the history has for each.
/// Searching looks up every photo in the history, so it runs off the window's own thread. The
/// answer also names the folder that was looked through, so the window can say where it looked.
#[tauri::command]
async fn photos(app: AppHandle, hub: State<'_, Arc<Hub>>, limit: usize, skip: usize, query: Option<String>, until: Option<String>) -> AppResult<serde_json::Value> {
    let root = photos_root(&app)?;
    allow_photo_folder(&app, &root);
    let folder = root.to_string_lossy().into_owned();
    let hub = hub.inner().clone();
    let mut page = tauri::async_runtime::spawn_blocking(move || hub.photos(&root, limit, skip, query.as_deref().unwrap_or(""), until.as_deref().unwrap_or("")))
        .await
        .map_err(|e| AppError::Storage(e.to_string()))??;
    if let Some(fields) = page.as_object_mut() {
        fields.insert("folder".into(), serde_json::Value::String(folder));
    }
    Ok(page)
}

/// Opens the folder of one VRChat photo with the file selected.
#[tauri::command]
fn show_photo(app: AppHandle, path: String) -> AppResult<()> {
    let root = photos_root(&app)?;
    let file = std::path::PathBuf::from(&path);
    // Only files inside the VRChat photo folder; the page cannot point Explorer anywhere else.
    let inside = file.canonicalize().ok().zip(root.canonicalize().ok()).is_some_and(|(f, r)| f.starts_with(r));
    if !inside || !file.is_file() {
        return Err(AppError::CannotDo("That is not one of your VRChat photos.".into()));
    }
    std::process::Command::new("explorer.exe")
        .arg(format!("/select,{}", file.display()))
        .spawn()
        .map_err(|e| AppError::Storage(e.to_string()))?;
    Ok(())
}

#[tauri::command]
fn open_backups_folder(app: AppHandle) -> AppResult<()> {
    let root = backups_root(&app)?;
    std::fs::create_dir_all(&root).map_err(|e| AppError::Storage(e.to_string()))?;
    std::process::Command::new("explorer.exe")
        .arg(&root)
        .spawn()
        .map_err(|e| AppError::Storage(e.to_string()))?;
    Ok(())
}

/// Opens the folder with the credits and the licence texts of everything the installer carries.
#[tauri::command]
fn open_licences_folder(app: AppHandle) -> AppResult<()> {
    let root = app
        .path()
        .resource_dir()
        .map_err(|e| AppError::Storage(e.to_string()))?
        .join("licences");
    if !root.is_dir() {
        return Err(AppError::CannotDo("The licence texts are not part of this build of the app.".into()));
    }
    std::process::Command::new("explorer.exe")
        .arg(&root)
        .spawn()
        .map_err(|e| AppError::Storage(e.to_string()))?;
    Ok(())
}

/// Brings the main window back, for example from the tray.
fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Hides the window. The app keeps running and stays reachable from the tray icon.
#[tauri::command]
fn hide_to_tray(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    discord::show(None);
    app.exit(0);
}

fn add_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Anonymous VRX", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Anonymous VRX")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_window(app),
            "quit" => {
                app.exit(0)
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

pub fn run() {
    keep_crashes();
    if !window_parts_work() {
        return;
    }
    apply_gpu_choice();
    tauri::Builder::default()
        // Starting the app a second time brings the running one to the front.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_window(app)
        }))
        // The window asks what to do instead of closing straight away.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.emit("window:close-asked", ());
            }
        })
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec![TRAY_FLAG])))
        .setup(|app| {
            let handle = app.handle().clone();
            let data_dir = app.path().app_data_dir()?;
            let api = Arc::new(ApiClient::new().map_err(|e: AppError| e.to_string())?);
            let emit: hub::Emit = Arc::new(move |name, payload| {
                if name == "popup" {
                    let text = |k: &str| payload.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let alert = VrAlert { kind: text("kind"), label: text("label"), title: text("title"), body: text("body") };
                    vr_notice(&handle, alert, false);
                    if !may_notify() {
                        return;
                    }
                    // Shown from here, so it works while the window is hidden in the tray.
                    use tauri_plugin_notification::NotificationExt;
                    let mut note = handle.notification().builder().title(text("title"));
                    let body = text("body");
                    if !body.is_empty() {
                        note = note.body(body);
                    }
                    let _ = note.show();
                    return;
                }
                let _ = handle.emit(name, payload);
            });
            app.manage(Hub::new(api, data_dir, emit, true, gamelog::default_dir()));
            // SteamVR's own news: alerts go to the headset (and SteamVR closing to Windows too, since
            // the headset view goes with it); the device list goes to the window for the wrist.
            let news_app = app.handle().clone();
            app.manage(overlay::Overlay::start(
                steamvr_running,
                Arc::new(move |news| match news {
                    overlay::News::Alert { kind, label, title, body } => {
                        if kind == "steamvr" && !in_quiet_time() {
                            use tauri_plugin_notification::NotificationExt;
                            let _ = news_app.notification().builder().title(&title).body(&body).show();
                        }
                        vr_notice(&news_app, VrAlert { kind: kind.into(), label: label.into(), title, body }, true);
                    }
                    overlay::News::Devices(list) => {
                        let _ = news_app.emit("vr:devices", list);
                    }
                    // The window drew the tab, so it knows what sits under the click.
                    overlay::News::DashClick { x, y } => {
                        let _ = news_app.emit("vr:dash-click", serde_json::json!({ "x": x, "y": y }));
                    }
                }),
            ));
            add_tray(app)?;
            // Started by Windows at sign-in, the app stays in the tray until it is asked for.
            if std::env::var_os(SHOW_AFTER_RESTART).is_some() || !std::env::args().any(|a| a == TRAY_FLAG) {
                show_window(app.handle());
            }
            let watcher = app.handle().clone();
            std::thread::spawn(move || watch_game(watcher));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            session_restore,
            login,
            verify_code,
            logout,
            get_friends,
            get_favorites,
            get_instances,
            invite_myself,
            create_instance,
            request_invite,
            get_notices,
            get_handled,
            get_profile,
            get_profile_part,
            get_thing,
            set_alerts,
            set_own_feed,
            get_last_seen,
            get_ratings,
            set_rating,
            get_world_visits,
            get_tags,
            set_tags,
            set_game_watch,
            set_quiet,
            may_notify,
            get_autostart,
            set_autostart,
            set_discord,
            save_settings,
            load_settings,
            get_avatar_history,
            get_note,
            get_favorites_cached,
            friend_action,
            get_moderations,
            get_person_brief,
            get_calendar,
            follow_event,
            set_moderation,
            upload_action,
            get_friend_status,
            rename_favorite_group,
            load_picture,
            group_action,
            search_feed,
            get_together_counts,
            get_local_favorites,
            add_local_favorite,
            remove_local_favorite,
            set_status,
            get_service_status,
            check_favorites,
            get_favorite_groups,
            boop,
            get_invite_messages,
            set_invite_message,
            update_profile,
            get_my_cosmetics,
            update_look,
            update_languages,
            get_moderations_for,
            get_my_groups,
            invite_to_group,
            save_picture,
            open_pictures_folder,
            get_favorites_of,
            add_favorite,
            remove_favorites,
            set_note,
            get_noted,
            get_friend_log,
            search_users,
            search_worlds,
            search_avatars,
            wear_avatar,
            invite_here,
            open_link,
            notice_action,
            get_worlds,
            want_world,
            get_feed,
            get_here,
            get_visits,
            get_visit_people,
            get_videos,
            get_videos_page,
            get_my_stats,
            get_friend_days,
            get_visit_videos,
            get_video_titles,
            export_history,
            open_exports_folder,
            get_pipeline_status,
            hide_to_tray,
            backup_now,
            list_backups,
            auto_backup,
            vrcx_found,
            import_vrcx,
            vrcx_stamp,
            get_cosmetics,
            get_world_stats,
            get_pairs,
            log_find,
            log_stays,
            get_habits,
            get_waiting,
            wait_for_room,
            stop_waiting,
            get_mutuals,
            fetch_mutuals,
            stop_mutuals,
            update_info,
            update_check,
            update_install,
            get_no_gpu,
            set_no_gpu,
            restart_app,
            game_cache,
            clear_game_cache,
            set_game_cache_limit,
            set_vr_notices,
            notify_vr,
            test_vr_notice,
            set_vr_overlay,
            vr_overlay_notice,
            vr_overlay_wrist,
            vr_overlay_dashboard,
            vr_overlay_status,
            vr_overlay_try_native,
            restore_backup,
            open_backups_folder,
            photos,
            show_photo,
            open_licences_folder,
            open_join_page,
            launch_instance,
            quit_app
        ])
        .run(tauri::generate_context!())
        .expect("the app window could not start");
}

#[cfg(test)]
mod crash_tests {
    #[test]
    fn a_crash_note_says_what_where_and_which_version() {
        let words = super::crash_words("index out of bounds", "src/hub.rs:12", "2026-11-27 10:00:00 UTC");
        assert!(words.contains(env!("CARGO_PKG_VERSION")));
        assert!(words.contains("What: index out of bounds\r\n") && words.contains("Where: src/hub.rs:12\r\n"));
        assert!(super::crash_file().is_some_and(|f| f.ends_with("app.anonymousvrx.desktop/crash log.txt")));
    }
}

#[cfg(test)]
mod helper_tests {
    use super::{helper_elsewhere, helpers_answer_elsewhere};

    #[test]
    fn only_helpers_in_vrchats_folder_count_as_the_game() {
        let folder = r"c:\program files (x86)\steam\steamapps\common\vrchat";
        assert!(!helper_elsewhere(r"c:\program files (x86)\steam\steamapps\common\vrchat\launch.exe", folder));
        assert!(!helper_elsewhere(r"c:\program files (x86)\steam\steamapps\common\vrchat\start_protected_game.exe", &format!("{folder}\\")));
        assert!(helper_elsewhere(r"d:\games\other\start_protected_game.exe", folder));
        // A folder whose name only starts the same is someone else's.
        assert!(helper_elsewhere(r"c:\program files (x86)\steam\steamapps\common\vrchat tools\launch.exe", folder));
        // A place Windows would not give counts as VRChat's, to be careful.
        assert!(!helper_elsewhere("?", folder));
        assert!(!helper_elsewhere(r"d:\games\launch.exe", ""));
    }

    #[test]
    fn a_broken_or_empty_powershell_answer_is_not_closed() {
        let folder = r"c:\games\vrchat";
        let never = || -> bool { panic!("only asked when the list is empty") };
        // A whole list with every helper elsewhere.
        assert!(helpers_answer_elsewhere("D:\\Tools\\launch.exe\r\nlisted\r\n", folder, never));
        // One of them is VRChat's.
        assert!(!helpers_answer_elsewhere("d:\\tools\\launch.exe\r\nc:\\games\\vrchat\\start_protected_game.exe\r\nlisted\r\n", folder, never));
        // PowerShell printed nothing, or broke off before the end of its list.
        assert!(!helpers_answer_elsewhere("", folder, never));
        assert!(!helpers_answer_elsewhere("d:\\tools\\launch.exe\r\n", folder, never));
        // An empty list counts only when the programs are known to be gone now.
        assert!(helpers_answer_elsewhere("listed\r\n", folder, || true));
        assert!(!helpers_answer_elsewhere("listed\r\n", folder, || false));
    }
}

#[cfg(test)]
mod quiet_tests {
    use super::in_quiet_hours;

    #[test]
    fn quiet_hours_can_run_past_midnight() {
        // 23:00 to 08:00
        assert!(in_quiet_hours(1380, 480, 1400));
        assert!(in_quiet_hours(1380, 480, 0));
        assert!(in_quiet_hours(1380, 480, 479));
        assert!(!in_quiet_hours(1380, 480, 480));
        assert!(!in_quiet_hours(1380, 480, 720));
        // 13:00 to 14:00
        assert!(in_quiet_hours(780, 840, 800));
        assert!(!in_quiet_hours(780, 840, 840));
        // The same time twice means no quiet stretch at all.
        assert!(!in_quiet_hours(600, 600, 600));
    }
}
