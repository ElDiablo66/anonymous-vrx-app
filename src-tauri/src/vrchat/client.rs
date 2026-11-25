//! Small client for the parts of the VRChat web API this app uses.
//!
//! Every request goes through one gate that spaces calls out, and a 429
//! answer makes the client wait before it tries again.

use std::sync::RwLock;
use std::time::Duration;

use base64::Engine;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use reqwest::header::{HeaderMap, AUTHORIZATION, COOKIE, RETRY_AFTER, SET_COOKIE};
use reqwest::{Method, StatusCode};
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::error::{AppError, AppResult};

pub const DEFAULT_BASE: &str = "https://api.vrchat.cloud/api/1";
pub const PIPELINE_URL: &str = "wss://pipeline.vrchat.cloud/";

/// Sent with every request and with the websocket handshake.
pub fn user_agent() -> String {
    format!(
        "{}/{} ({})",
        crate::APP_SLUG,
        env!("CARGO_PKG_VERSION"),
        crate::APP_CONTACT
    )
}

/// Same characters JavaScript's encodeURIComponent leaves alone.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Session {
    pub auth: Option<String>,
    pub two_factor: Option<String>,
}

pub enum LoginOutcome {
    SignedIn(Value),
    /// The account needs a code. Holds the methods VRChat offers,
    /// for example `totp`, `otp` (recovery code) or `emailOtp`.
    NeedsCode(Vec<String>),
}

pub struct Timing {
    /// Shortest pause between two requests.
    pub gap: Duration,
    /// First wait after a 429 when the server gives no Retry-After.
    pub backoff: Duration,
    /// Longest single wait after a 429.
    pub max_wait: Duration,
    pub attempts: u32,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            gap: Duration::from_millis(60),
            backoff: Duration::from_secs(5),
            max_wait: Duration::from_secs(120),
            attempts: 3,
        }
    }
}

pub struct ApiClient {
    http: reqwest::Client,
    /// For large downloads: no overall time limit.
    slow: reqwest::Client,
    base: String,
    session: RwLock<Session>,
    next_slot: Mutex<Instant>,
    status_url: String,
    timing: Timing,
}

impl ApiClient {
    pub fn new() -> AppResult<Self> {
        Self::with_base(DEFAULT_BASE, Timing::default())
    }

    pub fn with_base(base: &str, timing: Timing) -> AppResult<Self> {
        let http = reqwest::Client::builder()
            .user_agent(user_agent())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .build()?;
        let slow = reqwest::Client::builder()
            .user_agent(user_agent())
            .connect_timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self {
            http,
            slow,
            base: base.trim_end_matches('/').to_string(),
            session: RwLock::new(Session::default()),
            next_slot: Mutex::new(Instant::now()),
            status_url: if base == DEFAULT_BASE {
                "https://status.vrchat.com/api/v2/status.json".to_string()
            } else {
                format!("{}/status.json", base.trim_end_matches('/'))
            },
            timing,
        })
    }

    pub fn session(&self) -> Session {
        self.session.read().unwrap().clone()
    }

    pub fn set_session(&self, session: Session) {
        *self.session.write().unwrap() = session;
    }

    /// Forgets the sign-in but keeps the "remember this device" cookie.
    pub fn clear_auth(&self) {
        self.session.write().unwrap().auth = None;
    }

    pub fn auth_token(&self) -> Option<String> {
        self.session.read().unwrap().auth.clone()
    }

    fn cookie_header(&self) -> Option<String> {
        let s = self.session.read().unwrap();
        let mut parts = Vec::new();
        if let Some(a) = &s.auth {
            parts.push(format!("auth={a}"));
        }
        if let Some(t) = &s.two_factor {
            parts.push(format!("twoFactorAuth={t}"));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("; "))
        }
    }

    fn remember_cookies(&self, headers: &HeaderMap) {
        let mut s = self.session.write().unwrap();
        for value in headers.get_all(SET_COOKIE) {
            let Ok(text) = value.to_str() else { continue };
            let pair = text.split(';').next().unwrap_or("");
            let Some((name, val)) = pair.split_once('=') else { continue };
            let val = val.trim();
            match name.trim() {
                "auth" if !val.is_empty() => s.auth = Some(val.to_string()),
                "twoFactorAuth" if !val.is_empty() => s.two_factor = Some(val.to_string()),
                _ => {}
            }
        }
    }

    async fn wait_for_slot(&self) {
        let mut next = self.next_slot.lock().await;
        let now = Instant::now();
        if *next > now {
            tokio::time::sleep_until(*next).await;
        }
        *next = Instant::now() + self.timing.gap;
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        basic: Option<&str>,
        body: Option<&Value>,
    ) -> AppResult<(StatusCode, Value)> {
        let url = format!("{}{}", self.base, path);
        let mut wait = self.timing.backoff;
        for attempt in 1..=self.timing.attempts {
            self.wait_for_slot().await;
            let mut req = self.http.request(method.clone(), &url);
            if let Some(cookie) = self.cookie_header() {
                req = req.header(COOKIE, cookie);
            }
            if let Some(b) = basic {
                req = req.header(AUTHORIZATION, format!("Basic {b}"));
            }
            if let Some(b) = body {
                req = req.json(b);
            }
            let resp = req.send().await?;
            let status = resp.status();
            if status == StatusCode::TOO_MANY_REQUESTS {
                if attempt == self.timing.attempts {
                    return Err(AppError::RateLimited);
                }
                let asked = resp
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .map(Duration::from_secs);
                let pause = asked.unwrap_or(wait).min(self.timing.max_wait);
                // Hold every other request back for the same time.
                {
                    let mut next = self.next_slot.lock().await;
                    *next = Instant::now() + pause;
                }
                wait = (wait * 2).min(self.timing.max_wait);
                continue;
            }
            self.remember_cookies(resp.headers());
            let text = resp.text().await?;
            let json = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
            return Ok((status, json));
        }
        Err(AppError::RateLimited)
    }

    fn api_error(status: StatusCode, body: &Value) -> AppError {
        if status == StatusCode::UNAUTHORIZED {
            return AppError::Unauthorized;
        }
        let message = body
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("no details given")
            .trim_matches('"')
            .to_string();
        AppError::Api {
            status: status.as_u16(),
            message,
        }
    }

    fn code_methods(body: &Value) -> Option<Vec<String>> {
        body.get("requiresTwoFactorAuth")?.as_array().map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
    }

    /// Signs in with a username and password. Neither is kept anywhere.
    pub async fn login(&self, username: &str, password: &str) -> AppResult<LoginOutcome> {
        let pair = format!(
            "{}:{}",
            utf8_percent_encode(username, COMPONENT),
            utf8_percent_encode(password, COMPONENT)
        );
        let basic = base64::engine::general_purpose::STANDARD.encode(pair);
        // A stale auth cookie would win over the new credentials.
        self.clear_auth();
        let (status, body) = self.send(Method::GET, "/auth/user", Some(&basic), None).await?;
        if !status.is_success() {
            return Err(Self::api_error(status, &body));
        }
        if let Some(methods) = Self::code_methods(&body) {
            return Ok(LoginOutcome::NeedsCode(methods));
        }
        Ok(LoginOutcome::SignedIn(body))
    }

    /// Sends the code for one of the methods `login` returned.
    pub async fn verify_code(&self, method: &str, code: &str) -> AppResult<()> {
        let path = match method {
            "totp" => "/auth/twofactorauth/totp/verify",
            "emailOtp" => "/auth/twofactorauth/emailotp/verify",
            "otp" => "/auth/twofactorauth/otp/verify",
            other => return Err(AppError::Parse(format!("unknown code method {other}"))),
        };
        let body = serde_json::json!({ "code": code.trim() });
        let (status, json) = self.send(Method::POST, path, None, Some(&body)).await?;
        if status == StatusCode::BAD_REQUEST {
            return Err(AppError::BadCode);
        }
        if !status.is_success() {
            return Err(Self::api_error(status, &json));
        }
        if json.get("verified").and_then(Value::as_bool) == Some(true) {
            Ok(())
        } else {
            Err(AppError::BadCode)
        }
    }

    /// The signed-in user, including the id lists of online, active and offline friends.
    pub async fn current_user(&self) -> AppResult<Value> {
        if self.auth_token().is_none() {
            return Err(AppError::NotSignedIn);
        }
        let (status, body) = self.send(Method::GET, "/auth/user", None, None).await?;
        if !status.is_success() {
            return Err(Self::api_error(status, &body));
        }
        if Self::code_methods(&body).is_some() {
            return Err(AppError::TwoFactorRequired);
        }
        Ok(body)
    }

    /// One group of friends (online and active, or offline), all pages.
    pub async fn friends(&self, offline: bool) -> AppResult<Vec<Value>> {
        const PAGE: usize = 100;
        let mut all = Vec::new();
        let mut offset = 0usize;
        loop {
            let path = format!("/auth/user/friends?offset={offset}&n={PAGE}&offline={offline}");
            let (status, body) = self.send(Method::GET, &path, None, None).await?;
            if !status.is_success() {
                return Err(Self::api_error(status, &body));
            }
            let page = body
                .as_array()
                .ok_or_else(|| AppError::Parse("friend list was not a list".into()))?;
            let count = page.len();
            all.extend(page.iter().cloned());
            if count < PAGE {
                break;
            }
            offset += PAGE;
        }
        Ok(all)
    }

    async fn pages(&self, path_and_query: &str) -> AppResult<Vec<Value>> {
        const PAGE: usize = 100;
        let mut all = Vec::new();
        let mut offset = 0usize;
        loop {
            let joiner = if path_and_query.contains('?') { '&' } else { '?' };
            let path = format!("{path_and_query}{joiner}n={PAGE}&offset={offset}");
            let (status, body) = self.send(Method::GET, &path, None, None).await?;
            if !status.is_success() {
                return Err(Self::api_error(status, &body));
            }
            let page = body
                .as_array()
                .ok_or_else(|| AppError::Parse("expected a list".into()))?;
            let count = page.len();
            all.extend(page.iter().cloned());
            if count < PAGE {
                break;
            }
            offset += PAGE;
        }
        Ok(all)
    }

    /// Every friend the user has marked as a favourite, with the group tag.
    pub async fn favorite_friends(&self) -> AppResult<Vec<Value>> {
        self.pages("/favorites?type=friend").await
    }

    /// The user's favourite groups of every kind, with their display names.
    pub async fn favorite_groups(&self) -> AppResult<Vec<Value>> {
        self.pages("/favorite/groups").await
    }

    /// Friend requests, invites and the other classic notifications.
    pub async fn notifications(&self) -> AppResult<Vec<Value>> {
        self.pages("/auth/user/notifications").await
    }

    /// The newer notifications, such as group invites and announcements.
    pub async fn notifications_v2(&self) -> AppResult<Vec<Value>> {
        let (status, body) = self
            .send(Method::GET, "/notifications?limit=100", None, None)
            .await?;
        if !status.is_success() {
            return Err(Self::api_error(status, &body));
        }
        Ok(body.as_array().cloned().unwrap_or_default())
    }

    async fn act(&self, method: Method, path: String, body: Option<Value>) -> AppResult<()> {
        let (status, json) = self.send(method, &path, None, body.as_ref()).await?;
        if status.is_success() {
            Ok(())
        } else {
            Err(Self::api_error(status, &json))
        }
    }

    fn part(text: &str) -> String {
        utf8_percent_encode(text, COMPONENT).to_string()
    }

    pub async fn accept_friend_request(&self, notification_id: &str) -> AppResult<()> {
        let path = format!("/auth/user/notifications/{}/accept", Self::part(notification_id));
        self.act(Method::PUT, path, None).await
    }

    /// Removes a classic notification. For a friend request this declines it.
    pub async fn hide_notification(&self, notification_id: &str) -> AppResult<()> {
        let path = format!("/auth/user/notifications/{}/hide", Self::part(notification_id));
        self.act(Method::PUT, path, None).await
    }

    pub async fn delete_notification_v2(&self, notification_id: &str) -> AppResult<()> {
        let path = format!("/notifications/{}", Self::part(notification_id));
        self.act(Method::DELETE, path, None).await
    }

    /// Answers a newer notification with one of the replies it offered.
    pub async fn respond_notification_v2(&self, notification_id: &str, kind: &str, data: &str) -> AppResult<()> {
        let path = format!("/notifications/{}/respond", Self::part(notification_id));
        let body = serde_json::json!({ "responseType": kind, "responseData": data });
        self.act(Method::POST, path, Some(body)).await
    }

    /// Asks to join a group. Open groups let you in at once; others file a request.
    pub async fn join_group(&self, group_id: &str) -> AppResult<()> {
        self.act(Method::POST, format!("/groups/{}/join", Self::part(group_id)), Some(serde_json::json!({}))).await
    }

    pub async fn leave_group(&self, group_id: &str) -> AppResult<()> {
        self.act(Method::POST, format!("/groups/{}/leave", Self::part(group_id)), Some(serde_json::json!({}))).await
    }

    /// Changes my own status and status text.
    pub async fn update_me(&self, user_id: &str, body: Value) -> AppResult<()> {
        self.act(Method::PUT, format!("/users/{}", Self::part(user_id)), Some(body)).await
    }

    /// VRChat's public service status page. No cookie is sent.
    pub async fn service_status(&self) -> AppResult<Value> {
        let url = self.status_url.clone();
        let reply = self.http.get(&url).send().await?;
        if !reply.status().is_success() {
            return Err(AppError::CannotDo("VRChat's status page did not answer.".into()));
        }
        Ok(reply.json::<Value>().await.map_err(|e| AppError::Parse(e.to_string()))?)
    }

    /// Boops a friend, the small nudge VRChat shows as a notification.
    pub async fn boop(&self, user_id: &str) -> AppResult<()> {
        let path = format!("/users/{}/boop", Self::part(user_id));
        self.act(Method::POST, path, Some(serde_json::json!({}))).await
    }


    /// Makes one of my avatars public or private.
    pub async fn set_avatar_release(&self, avatar_id: &str, status: &str) -> AppResult<()> {
        let path = format!("/avatars/{}", Self::part(avatar_id));
        self.act(Method::PUT, path, Some(serde_json::json!({ "releaseStatus": status }))).await
    }

    pub async fn delete_avatar(&self, avatar_id: &str) -> AppResult<()> {
        self.act(Method::DELETE, format!("/avatars/{}", Self::part(avatar_id)), None).await
    }

    /// Publishes one of my worlds, or takes it back to private.
    pub async fn set_world_public(&self, world_id: &str, public: bool) -> AppResult<()> {
        let path = format!("/worlds/{}/publish", Self::part(world_id));
        let method = if public { Method::PUT } else { Method::DELETE };
        self.act(method, path, None).await
    }

    pub async fn delete_world(&self, world_id: &str) -> AppResult<()> {
        self.act(Method::DELETE, format!("/worlds/{}", Self::part(world_id)), None).await
    }

    /// Sends a friend request.
    pub async fn friend_request(&self, user_id: &str) -> AppResult<()> {
        self.act(Method::POST, format!("/user/{}/friendRequest", Self::part(user_id)), Some(serde_json::json!({}))).await
    }

    /// Takes back a friend request I sent.
    pub async fn cancel_friend_request(&self, user_id: &str) -> AppResult<()> {
        self.act(Method::DELETE, format!("/user/{}/friendRequest", Self::part(user_id)), None).await
    }

    pub async fn unfriend(&self, user_id: &str) -> AppResult<()> {
        self.act(Method::DELETE, format!("/auth/user/friends/{}", Self::part(user_id)), None).await
    }

    /// Every choice I made about another person: blocks, mutes and the rest.
    pub async fn player_moderations(&self) -> AppResult<Value> {
        self.read("/auth/user/playermoderations").await
    }

    /// Blocks or mutes someone. `kind` is VRChat's own word, `block` or `mute`.
    pub async fn moderate(&self, user_id: &str, kind: &str) -> AppResult<()> {
        self.act(Method::POST, "/auth/user/playermoderations".into(), Some(serde_json::json!({ "moderated": user_id, "type": kind }))).await
    }

    /// Takes a block or a mute back.
    pub async fn unmoderate(&self, user_id: &str, kind: &str) -> AppResult<()> {
        self.act(Method::PUT, "/auth/user/unplayermoderate".into(), Some(serde_json::json!({ "moderated": user_id, "type": kind }))).await
    }

    /// Gives one of my favourite groups a new name.
    pub async fn rename_favorite_group(&self, kind: &str, group: &str, owner: &str, name: &str) -> AppResult<()> {
        let path = format!("/favorite/group/{}/{}/{}", Self::part(kind), Self::part(group), Self::part(owner));
        self.act(Method::PUT, path, Some(serde_json::json!({ "displayName": name }))).await
    }

    /// Downloads one picture and returns its type and bytes. The sign-in cookie goes along
    /// only when `signed` is set and the address is VRChat's own API, which some pictures need.
    pub async fn picture(&self, url: &str) -> AppResult<(String, Vec<u8>)> {
        self.picture_as(url, false).await
    }

    pub async fn picture_as(&self, url: &str, signed: bool) -> AppResult<(String, Vec<u8>)> {
        const LIMIT: usize = 40 * 1024 * 1024;
        let allowed = url.starts_with("https://") || url.starts_with("http://127.0.0.1:");
        if !allowed || url.len() > 2000 || url.chars().any(|c| c.is_control() || c == ' ') {
            return Err(AppError::CannotDo("That is not a picture address this app will fetch.".into()));
        }
        let mut request = self.http.get(url);
        if signed && url.starts_with(&format!("{}/", self.base)) {
            if let Some(cookie) = self.cookie_header() {
                request = request.header(COOKIE, cookie);
            }
        }
        let reply = request.send().await?;
        if !reply.status().is_success() {
            return Err(AppError::CannotDo(format!("The picture could not be fetched ({}).", reply.status().as_u16())));
        }
        let kind = reply
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !kind.starts_with("image/") {
            return Err(AppError::CannotDo("That address did not answer with a picture.".into()));
        }
        let bytes = reply.bytes().await?;
        if bytes.len() > LIMIT {
            return Err(AppError::CannotDo("That picture is too large to save.".into()));
        }
        Ok((kind, bytes.to_vec()))
    }

    /// Invites a user to the given instance.
    pub async fn invite_user(&self, user_id: &str, location: &str) -> AppResult<()> {
        let path = format!("/invite/{}", Self::part(user_id));
        let body = serde_json::json!({ "instanceId": location });
        self.act(Method::POST, path, Some(body)).await
    }

    /// Any read-only list or object from the API, as plain JSON.
    pub async fn read(&self, path: &str) -> AppResult<Value> {
        let (status, body) = self.send(Method::GET, path, None, None).await?;
        if !status.is_success() {
            return Err(Self::api_error(status, &body));
        }
        Ok(body)
    }

    /// Asks a public avatar database one question (`fileId` or `authorId`).
    /// Nothing about the signed-in account is sent.
    pub async fn ask_avatar_database(&self, provider: &str, key: &str, value: &str) -> AppResult<Value> {
        let allowed = provider.starts_with("https://") || provider.starts_with("http://127.0.0.1:");
        if !allowed || provider.contains(['?', '#', ' ']) || !matches!(key, "fileId" | "authorId") {
            return Err(AppError::CannotDo("The avatar search address must be a plain https address.".into()));
        }
        self.wait_for_slot().await;
        let url = format!("{provider}?{key}={}", Self::part(value));
        let resp = self.http.get(url).send().await?;
        if !resp.status().is_success() {
            return Ok(Value::Array(Vec::new()));
        }
        resp.json().await.map_err(|e| AppError::Parse(e.without_url().to_string()))
    }

    /// Adds something to a favourite group. `kind` is `world`, `avatar` or `friend`.
    pub async fn add_favorite(&self, kind: &str, id: &str, group: &str) -> AppResult<()> {
        let body = serde_json::json!({ "type": kind, "favoriteId": id, "tags": [group] });
        self.act(Method::POST, "/favorites".into(), Some(body)).await
    }

    /// Removes one favourite by the id of the favourite entry (not of the world or avatar).
    pub async fn remove_favorite(&self, entry_id: &str) -> AppResult<()> {
        self.act(Method::DELETE, format!("/favorites/{}", Self::part(entry_id)), None).await
    }

    /// Every page of a list.
    pub async fn read_all(&self, path_and_query: &str) -> AppResult<Vec<Value>> {
        self.pages(path_and_query).await
    }

    /// One user's public profile.
    pub async fn user(&self, user_id: &str) -> AppResult<Value> {
        let path = format!("/users/{}", Self::part(user_id));
        let (status, body) = self.send(Method::GET, &path, None, None).await?;
        if !status.is_success() {
            return Err(Self::api_error(status, &body));
        }
        Ok(body)
    }

    async fn list(&self, path: String) -> AppResult<Vec<Value>> {
        let (status, body) = self.send(Method::GET, &path, None, None).await?;
        if !status.is_success() {
            return Err(Self::api_error(status, &body));
        }
        Ok(body.as_array().cloned().unwrap_or_default())
    }

    /// Up to 24 users whose name matches.
    pub async fn search_users(&self, query: &str) -> AppResult<Vec<Value>> {
        self.list(format!("/users?search={}&n=24", Self::part(query))).await
    }

    /// Up to 24 worlds that match, best match first.
    pub async fn search_worlds(&self, query: &str) -> AppResult<Vec<Value>> {
        self.list(format!("/worlds?search={}&n=24&sort=relevance", Self::part(query)))
            .await
    }

    /// Searches a public avatar database. This goes to a third party, so no
    /// VRChat cookie or other account detail is sent, only the search text.
    pub async fn search_avatars(&self, provider: &str, query: &str) -> AppResult<Vec<Value>> {
        // Plain http is only let through for a server on this same PC, which is what the tests use.
        let allowed = provider.starts_with("https://") || provider.starts_with("http://127.0.0.1:");
        if !allowed || provider.contains(['?', '#', ' ']) {
            return Err(AppError::CannotDo("The avatar search address must be a plain https address.".into()));
        }
        self.wait_for_slot().await;
        let url = format!("{provider}?search={}&n=60", Self::part(query));
        let resp = self.http.get(url).send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Api {
                status: resp.status().as_u16(),
                message: "the avatar search service refused the request".into(),
            });
        }
        let body: Value = resp.json().await.map_err(|e| AppError::Parse(e.without_url().to_string()))?;
        Ok(body.as_array().cloned().unwrap_or_default())
    }

    /// Switches the signed-in user to an avatar.
    pub async fn select_avatar(&self, avatar_id: &str) -> AppResult<()> {
        let path = format!("/avatars/{}/select", Self::part(avatar_id));
        self.act(Method::PUT, path, None).await
    }

    /// Keeps the colon between world and instance readable in the address.
    fn location_part(location: &str) -> String {
        Self::part(location).replace("%3A", ":")
    }

    /// Details of one instance, including how many people are in it.
    pub async fn instance(&self, location: &str) -> AppResult<Value> {
        let path = format!("/instances/{}", Self::location_part(location));
        let (status, body) = self.send(Method::GET, &path, None, None).await?;
        if !status.is_success() {
            return Err(Self::api_error(status, &body));
        }
        Ok(body)
    }

    /// Sends the signed-in user an invite to an instance they are allowed to join.
    pub async fn invite_myself(&self, location: &str) -> AppResult<()> {
        let path = format!("/invite/myself/to/{}", Self::location_part(location));
        self.act(Method::POST, path, None).await
    }

    /// Makes a new instance of a world and gives back its full location.
    /// `kind` is VRChat's own word (`public`, `hidden`, `friends`, `private`); `owner` is the
    /// signed-in user for everything but public.
    pub async fn create_instance(
        &self,
        world_id: &str,
        kind: &str,
        region: &str,
        owner: Option<&str>,
        can_request_invite: bool,
    ) -> AppResult<String> {
        let mut body = serde_json::json!({ "worldId": world_id, "type": kind, "region": region });
        if let Some(owner) = owner {
            body["ownerId"] = Value::String(owner.to_string());
        }
        if can_request_invite {
            body["canRequestInvite"] = Value::Bool(true);
        }
        let (status, reply) = self.send(Method::POST, "/instances", None, Some(&body)).await?;
        if !status.is_success() {
            return Err(Self::api_error(status, &reply));
        }
        let location = reply["location"].as_str().unwrap_or_default();
        if !location.starts_with("wrld_") || !location.contains(':') {
            return Err(AppError::CannotDo("VRChat did not give back an instance.".into()));
        }
        Ok(location.to_string())
    }

    /// Asks a user to invite the signed-in user to where they are.
    pub async fn request_invite(&self, user_id: &str) -> AppResult<()> {
        let path = format!("/requestInvite/{}", Self::part(user_id));
        self.act(Method::POST, path, Some(serde_json::json!({}))).await
    }

    pub async fn world(&self, world_id: &str) -> AppResult<Value> {
        let path = format!("/worlds/{}", utf8_percent_encode(world_id, COMPONENT));
        let (status, body) = self.send(Method::GET, &path, None, None).await?;
        if !status.is_success() {
            return Err(Self::api_error(status, &body));
        }
        Ok(body)
    }

    pub async fn logout(&self) -> AppResult<()> {
        if self.auth_token().is_some() {
            let _ = self.send(Method::PUT, "/logout", None, None).await;
        }
        self.clear_auth();
        Ok(())
    }
}
