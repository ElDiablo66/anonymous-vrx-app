//! Small client for the parts of the VRChat web API this app uses.
//!
//! Every request goes through one gate that spaces calls out, and a 429
//! answer makes the client wait before it tries again.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use base64::Engine;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use reqwest::header::{HeaderMap, AUTHORIZATION, COOKIE, RETRY_AFTER, SET_COOKIE};
use reqwest::{Method, StatusCode};
use serde_json::Value;
use tokio::sync::{Mutex, Semaphore};
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
    /// Longest an action I asked for (a boop, an avatar switch) waits behind a slowdown before
    /// it gives up and says how long is left.
    pub action_wait: Duration,
    /// How long an action VRChat turned down with a 429 is held back when no Retry-After came.
    pub action_hold: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            gap: Duration::from_millis(60),
            backoff: Duration::from_secs(5),
            max_wait: Duration::from_secs(120),
            attempts: 3,
            action_wait: Duration::from_secs(10),
            action_hold: Duration::from_secs(60),
        }
    }
}

/// The longest wait VRChat may ask for that is kept. Anything longer is taken as an hour.
const LONGEST_ASKED: Duration = Duration::from_secs(3600);

/// How many signed pictures are fetched from VRChat's API at the same time.
const PICTURES_AT_ONCE: usize = 3;

/// The wait VRChat gave in a 429 answer, when it gave one in whole seconds.
fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get(RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

/// Whether an address that came from outside (a picture, a download, an avatar database) is one
/// this app will ask. Only https, and never a server on this PC or on the home network, so data
/// from a third party cannot point the app at a local service. Plain http to this same PC is let
/// through only when `local_ok` is set, which is the case only when the client itself talks to a
/// stand-in server on this PC, as the tests do.
pub fn address_allowed(url: &str, local_ok: bool) -> bool {
    if url.len() > 2000 || url.chars().any(|c| c.is_control() || c == ' ') {
        return false;
    }
    let Ok(parsed) = reqwest::Url::parse(url) else { return false };
    let Some(host) = parsed.host_str() else { return false };
    let host = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    match parsed.scheme() {
        "https" => !is_local_host(&host) || local_ok,
        "http" => local_ok && host == "127.0.0.1",
        _ => false,
    }
}

/// True for names and numbers that lead to this PC or the home network.
fn is_local_host(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => local_v4(ip),
        Ok(std::net::IpAddr::V6(ip)) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return local_v4(v4);
            }
            let first = ip.segments()[0];
            ip.is_loopback()
                || ip.is_unspecified()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

fn local_v4(ip: std::net::Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified() || ip.is_broadcast()
}

/// Follows at most ten redirects, and only to addresses `address_allowed` lets through, so a
/// server cannot send the app on to a local service either.
fn redirects(local_ok: bool) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 10 {
            attempt.error("too many redirects")
        } else if address_allowed(attempt.url().as_str(), local_ok) {
            attempt.follow()
        } else {
            attempt.stop()
        }
    })
}

/// How a request waits for its turn at the gate.
#[derive(Clone, Copy, PartialEq)]
enum Turn {
    /// Something I asked for: it does not queue behind reads and never waits long.
    Action,
    /// A read: it takes its turn a gap after the last one and waits out a slowdown of up to
    /// `max_wait`. A longer one is handed back at once with the time left.
    Read,
    /// Signing out: it takes its turn like a read but waits out a slowdown of any length, so
    /// VRChat is still told.
    Patient,
}

/// Seconds, rounded up, so "wait 0 seconds" never shows.
fn whole_seconds(d: Duration) -> u64 {
    d.as_secs() + u64::from(d.subsec_nanos() > 0)
}

/// What a 429 on an action holds back. VRChat limits switching avatars as a whole, so every
/// avatar switch shares one hold; anything else is held for its own address only, so booping
/// one friend too often never stops booping another.
fn hold_key(path: &str) -> String {
    if path.starts_with("/avatars/") && path.ends_with("/select") {
        "/avatars/*/select".into()
    } else {
        path.split('?').next().unwrap_or(path).to_string()
    }
}

pub struct ApiClient {
    http: reqwest::Client,
    /// For large downloads: no overall time limit.
    slow: reqwest::Client,
    base: String,
    session: RwLock<Session>,
    next_slot: Mutex<Instant>,
    /// The same spacing for the public avatar databases, kept apart from VRChat's.
    db_next_slot: Mutex<Instant>,
    /// Actions VRChat answered with a 429, by `hold_key`, and when they may be sent again.
    held: std::sync::Mutex<std::collections::HashMap<String, Instant>>,
    /// Until when everything is held back because a read met a 429. Kept apart from the gate
    /// itself, so a line of ordinary requests is never taken for a slowdown.
    slowed_until: std::sync::Mutex<Instant>,
    /// Until when signing in and sending a code are held back because VRChat answered one of
    /// them with a 429. Kept apart from `held`, so signing out or a new sign-in never clears it.
    sign_in_held: std::sync::Mutex<Option<Instant>>,
    /// Keeps signed picture fetches to a few at a time.
    pictures: Semaphore,
    /// True only when the client talks to a stand-in server on this PC, as the tests do.
    local_ok: bool,
    status_url: String,
    timing: Timing,
}

impl ApiClient {
    pub fn new() -> AppResult<Self> {
        Self::with_base(DEFAULT_BASE, Timing::default())
    }

    pub fn with_base(base: &str, timing: Timing) -> AppResult<Self> {
        let local_ok = base.starts_with("http://127.0.0.1:");
        let http = reqwest::Client::builder()
            .user_agent(user_agent())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .redirect(redirects(local_ok))
            .build()?;
        let slow = reqwest::Client::builder()
            .user_agent(user_agent())
            .connect_timeout(Duration::from_secs(15))
            .redirect(redirects(local_ok))
            .build()?;
        Ok(Self {
            http,
            slow,
            base: base.trim_end_matches('/').to_string(),
            session: RwLock::new(Session::default()),
            next_slot: Mutex::new(Instant::now()),
            db_next_slot: Mutex::new(Instant::now()),
            held: std::sync::Mutex::new(std::collections::HashMap::new()),
            slowed_until: std::sync::Mutex::new(Instant::now()),
            sign_in_held: std::sync::Mutex::new(None),
            pictures: Semaphore::new(PICTURES_AT_ONCE),
            local_ok,
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

    /// Forgets the sign-in but keeps the "remember this device" cookie. Waits VRChat asked for
    /// on single actions belonged to that sign-in and go with it.
    pub fn clear_auth(&self) {
        self.session.write().unwrap().auth = None;
        self.held.lock().unwrap().clear();
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

    /// Spaces out requests to the public avatar databases. They are other servers, so a
    /// slowdown VRChat asked for does not hold them up.
    async fn space_out(&self) {
        let start = {
            let mut next = self.db_next_slot.lock().await;
            let start = (*next).max(Instant::now());
            *next = start + self.timing.gap;
            start
        };
        tokio::time::sleep_until(start).await;
    }

    /// Waits for this request's moment to go. Reads take turns, a gap apart, and also wait out
    /// a slowdown VRChat asked for, including one that starts while they wait. An action I asked
    /// for does not queue behind reads, and it never waits long without a word: when the
    /// slowdown has more than `action_wait` left, it gives up and gives back the time left.
    /// A read does the same past `max_wait`, since VRChat may ask for many minutes.
    /// The gate is never held while waiting, so nobody waits behind a sleeper they cannot see.
    async fn wait_for_slot(&self, turn: Turn) -> Result<(), Duration> {
        loop {
            let start = {
                let mut next = self.next_slot.lock().await;
                let now = Instant::now();
                let slowed = *self.slowed_until.lock().unwrap();
                let left = slowed.saturating_duration_since(now);
                if turn == Turn::Action {
                    if left > self.timing.action_wait {
                        return Err(left);
                    }
                    now.max(slowed)
                } else {
                    if turn == Turn::Read && left > self.timing.max_wait {
                        return Err(left);
                    }
                    let start = (*next).max(now).max(slowed);
                    *next = start + self.timing.gap;
                    start
                }
            };
            tokio::time::sleep_until(start).await;
            if *self.slowed_until.lock().unwrap() <= Instant::now() {
                return Ok(());
            }
        }
    }

    /// How long is left of a wait VRChat asked for on signing in, when one is running.
    fn sign_in_wait(&self) -> Option<Duration> {
        let mut held = self.sign_in_held.lock().unwrap();
        let left = held.map(|until| until.saturating_duration_since(Instant::now())).filter(|left| !left.is_zero());
        if left.is_none() {
            *held = None;
        }
        left
    }

    /// Holds signing in and sending a code back for the time VRChat asked, or a minute.
    fn hold_sign_in(&self, asked: Option<Duration>) -> AppError {
        let pause = asked.unwrap_or(self.timing.action_hold).min(LONGEST_ASKED);
        let mut held = self.sign_in_held.lock().unwrap();
        let until = Instant::now() + pause;
        *held = Some(held.map_or(until, |was| was.max(until)));
        AppError::RateLimited(Some(whole_seconds(pause).max(1)))
    }

    /// How long is left of a slowdown VRChat asked for, when one is running.
    fn slowdown_left(&self) -> Option<Duration> {
        let left = self.slowed_until.lock().unwrap().saturating_duration_since(Instant::now());
        (!left.is_zero()).then_some(left)
    }

    /// Holds every read back for `pause`. A longer wait already set by another answer is kept.
    fn slow_down(&self, pause: Duration) {
        let mut slowed = self.slowed_until.lock().unwrap();
        *slowed = (*slowed).max(Instant::now() + pause);
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        basic: Option<&str>,
        body: Option<&Value>,
    ) -> AppResult<(StatusCode, Value)> {
        self.send_with(method, path, basic, body, None).await
    }

    /// `send`, with the sign-in cookie given instead of the current one when `cookie` is set.
    /// Signing out uses that, so a sign-out still waiting its turn never carries a newer sign-in.
    async fn send_with(
        &self,
        method: Method,
        path: &str,
        basic: Option<&str>,
        body: Option<&Value>,
        cookie: Option<&str>,
    ) -> AppResult<(StatusCode, Value)> {
        let url = format!("{}{}", self.base, path);
        // Anything but a read is something I asked for. It is sent once: never again by itself
        // after a 429, and never after a long wait I cannot see.
        let action = method != Method::GET;
        // Signing out waits its turn like a read, so VRChat is told even during a slowdown.
        let turn = if !action {
            Turn::Read
        } else if path == "/logout" {
            Turn::Patient
        } else {
            Turn::Action
        };
        let key = format!("{method} {}", hold_key(path));
        let mut wait = self.timing.backoff;
        for attempt in 1..=self.timing.attempts {
            if action {
                let now = Instant::now();
                let held = {
                    let mut held = self.held.lock().unwrap();
                    held.retain(|_, until| *until > now);
                    held.get(&key).map(|until| *until - now)
                };
                if let Some(left) = held {
                    return Err(AppError::RateLimited(Some(whole_seconds(left))));
                }
            }
            if let Err(left) = self.wait_for_slot(turn).await {
                return Err(AppError::RateLimited(Some(whole_seconds(left))));
            }
            let mut req = self.http.request(method.clone(), &url);
            if let Some(cookie) = cookie.map(str::to_string).or_else(|| self.cookie_header()) {
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
                let asked = retry_after(resp.headers());
                // A sign-in carries the password, so it is sent once and never repeated here.
                // Signing in then waits out VRChat's time, so pressing Sign in again inside it
                // is answered here and nothing goes out.
                if basic.is_some() {
                    return Err(self.hold_sign_in(asked));
                }
                if action {
                    // A code VRChat turned down for coming too often holds signing in back too.
                    if path.starts_with("/auth/twofactorauth/") {
                        self.hold_sign_in(asked);
                    }
                    // VRChat limits some actions on their own, like booping the same person
                    // again soon or switching avatars quickly. Only that action waits, for
                    // VRChat's own time up to an hour, and it says how long; reads and other
                    // actions go on as before.
                    let pause = asked.unwrap_or(self.timing.action_hold).min(LONGEST_ASKED);
                    self.held.lock().unwrap().insert(key, Instant::now() + pause);
                    return Err(AppError::RateLimited(Some(whole_seconds(pause).max(1))));
                }
                // Every other request waits the same time, the last try included. The time
                // VRChat gave is kept in full, up to an hour; only the app's own guess, when
                // VRChat gave none, stays under `max_wait`.
                let pause = match asked {
                    Some(asked) => asked.min(LONGEST_ASKED),
                    None => wait.min(self.timing.max_wait),
                };
                self.slow_down(pause);
                // A wait longer than this request is willing to sleep is handed back at once,
                // so nothing goes to VRChat again inside the time it asked for.
                if attempt == self.timing.attempts || pause > self.timing.max_wait {
                    return Err(AppError::RateLimited(asked.map(|a| whole_seconds(a.min(LONGEST_ASKED)))));
                }
                wait = (wait * 2).min(self.timing.max_wait);
                continue;
            }
            // A sign-out sent with an older cookie must not change the current sign-in.
            if cookie.is_none() {
                self.remember_cookies(resp.headers());
            }
            let text = resp.text().await?;
            let json = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
            return Ok((status, json));
        }
        Err(AppError::RateLimited(None))
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
        // Inside a wait VRChat asked for, the password is not sent again.
        if let Some(left) = self.sign_in_wait() {
            return Err(AppError::RateLimited(Some(whole_seconds(left).max(1))));
        }
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
        if let Some(left) = self.sign_in_wait() {
            return Err(AppError::RateLimited(Some(whole_seconds(left).max(1))));
        }
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
    /// `emoji` is one of VRChat's own emoji names or the file id of one I uploaded there.
    pub async fn boop(&self, user_id: &str, emoji: Option<&str>) -> AppResult<()> {
        let path = format!("/users/{}/boop", Self::part(user_id));
        let body = match emoji {
            Some(id) => serde_json::json!({ "emojiId": id }),
            None => serde_json::json!({}),
        };
        self.act(Method::POST, path, Some(body)).await
    }

    /// My twelve saved messages of one kind: `message` goes with an invite, `request` with asking for one.
    pub async fn invite_messages(&self, user_id: &str, kind: &str) -> AppResult<Value> {
        self.read(&format!("/message/{}/{}", Self::part(user_id), Self::part(kind))).await
    }

    /// Changes the text of one saved message. VRChat lets a slot change once an hour and
    /// answers with all twelve as they now stand.
    pub async fn set_invite_message(&self, user_id: &str, kind: &str, slot: u8, text: &str) -> AppResult<Value> {
        let path = format!("/message/{}/{}/{}", Self::part(user_id), Self::part(kind), slot);
        let (status, body) = self.send(Method::PUT, &path, None, Some(&serde_json::json!({ "message": text }))).await?;
        if !status.is_success() {
            return Err(Self::api_error(status, &body));
        }
        Ok(body)
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

    /// Writes part of my profile record: the bio and its links, the frame, effects and
    /// background. Only the keys in `body` change. Gives back the record as VRChat now has it.
    pub async fn update_profile_record(&self, user_id: &str, body: Value) -> AppResult<Value> {
        let (status, answer) = self.send(Method::PUT, &format!("/profile/{}", Self::part(user_id)), None, Some(&body)).await?;
        if !status.is_success() {
            return Err(Self::api_error(status, &answer));
        }
        Ok(answer)
    }

    /// Adds tags to my account (`add`) or takes them off. Languages live here as `language_xxx`.
    pub async fn change_tags(&self, user_id: &str, add: bool, tags: &[String]) -> AppResult<()> {
        let path = format!("/users/{}/{}", Self::part(user_id), if add { "addTags" } else { "removeTags" });
        self.act(Method::POST, path, Some(serde_json::json!({ "tags": tags }))).await
    }

    /// Invites someone into one of my groups.
    pub async fn invite_to_group(&self, group_id: &str, user_id: &str) -> AppResult<()> {
        let path = format!("/groups/{}/invites", Self::part(group_id));
        self.act(Method::POST, path, Some(serde_json::json!({ "userId": user_id, "confirmOverrideBlock": false }))).await
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
        if !address_allowed(url, self.local_ok) {
            return Err(AppError::CannotDo("That is not a picture address this app will fetch.".into()));
        }
        // Pictures come from many servers, so a failure is not put down to VRChat.
        let failed = |e: reqwest::Error| AppError::CannotDo(format!("The picture could not be fetched ({}).", e.without_url()));
        let mut request = self.http.get(url);
        // A signed picture is a request to VRChat's API like any other: it waits out a slowdown
        // VRChat asked for, takes its turn a gap apart, and only a few go at once.
        let mut _turn = None;
        let to_vrchat = signed && url.starts_with(&format!("{}/", self.base));
        if to_vrchat {
            if let Some(left) = self.slowdown_left() {
                return Err(AppError::RateLimited(Some(whole_seconds(left).max(1))));
            }
            _turn = self.pictures.acquire().await.ok();
            if let Some(left) = self.slowdown_left() {
                return Err(AppError::RateLimited(Some(whole_seconds(left).max(1))));
            }
            if let Err(left) = self.wait_for_slot(Turn::Read).await {
                return Err(AppError::RateLimited(Some(whole_seconds(left).max(1))));
            }
            if let Some(cookie) = self.cookie_header() {
                request = request.header(COOKIE, cookie);
            }
        }
        let reply = request.send().await.map_err(failed)?;
        if to_vrchat && reply.status() == StatusCode::TOO_MANY_REQUESTS {
            // Held the same way a read is, and not asked again from here.
            let asked = retry_after(reply.headers());
            let pause = match asked {
                Some(asked) => asked.min(LONGEST_ASKED),
                None => self.timing.backoff.min(self.timing.max_wait),
            };
            self.slow_down(pause);
            return Err(AppError::RateLimited(Some(whole_seconds(pause).max(1))));
        }
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
        let bytes = reply.bytes().await.map_err(failed)?;
        if bytes.len() > LIMIT {
            return Err(AppError::CannotDo("That picture is too large to save.".into()));
        }
        Ok((kind, bytes.to_vec()))
    }

    /// Invites a user to the given instance.
    pub async fn invite_user(&self, user_id: &str, location: &str, slot: Option<u8>) -> AppResult<()> {
        let path = format!("/invite/{}", Self::part(user_id));
        let mut body = serde_json::json!({ "instanceId": location });
        if let Some(slot) = slot {
            body["messageSlot"] = serde_json::json!(slot);
        }
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

    /// An avatar database that did not answer. It is a third party, so VRChat is not named.
    fn database_silent(e: reqwest::Error) -> AppError {
        AppError::CannotDo(format!("The avatar search service did not answer ({}).", e.without_url()))
    }

    /// An avatar database that answered with something that is not a list of avatars.
    fn database_odd(e: reqwest::Error) -> AppError {
        AppError::CannotDo(format!("The avatar search service sent something unexpected ({}).", e.without_url()))
    }

    /// Asks a public avatar database one question (`fileId` or `authorId`).
    /// Nothing about the signed-in account is sent.
    pub async fn ask_avatar_database(&self, provider: &str, key: &str, value: &str) -> AppResult<Value> {
        if !address_allowed(provider, self.local_ok) || provider.contains(['?', '#', ' ']) || !matches!(key, "fileId" | "authorId") {
            return Err(AppError::CannotDo("The avatar search address must be a plain https address.".into()));
        }
        self.space_out().await;
        let url = format!("{provider}?{key}={}", Self::part(value));
        let resp = self.http.get(url).send().await.map_err(Self::database_silent)?;
        if !resp.status().is_success() {
            return Ok(Value::Array(Vec::new()));
        }
        resp.json().await.map_err(Self::database_odd)
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

    /// One month of calendar events. `which` is empty for the events of my own groups,
    /// `following` for the ones I follow, `featured` for the ones VRChat puts forward.
    /// `month` is any moment in that month, RFC 3339. One request, up to 100 events.
    pub async fn calendar(&self, which: &str, month: &str) -> AppResult<Value> {
        let tail = if which.is_empty() { String::new() } else { format!("/{}", Self::part(which)) };
        self.read(&format!("/calendar{tail}?date={}&n=100", Self::part(month))).await
    }

    /// Events across all of VRChat: the ones on now and the ones coming up, soonest first.
    /// One request, up to 100 events.
    pub async fn discover_events(&self) -> AppResult<Value> {
        self.read("/calendar/discover?scope=all&n=100&upcomingSort=startTime_ascending").await
    }

    /// Follows a group's calendar event, or stops following it.
    pub async fn follow_event(&self, group_id: &str, calendar_id: &str, on: bool) -> AppResult<()> {
        let path = format!("/calendar/{}/{}/follow", Self::part(group_id), Self::part(calendar_id));
        self.act(Method::POST, path, Some(serde_json::json!({ "isFollowing": on }))).await
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
        if !address_allowed(provider, self.local_ok) || provider.contains(['?', '#', ' ']) {
            return Err(AppError::CannotDo("The avatar search address must be a plain https address.".into()));
        }
        self.space_out().await;
        let url = format!("{provider}?search={}&n=60", Self::part(query));
        let resp = self.http.get(url).send().await.map_err(Self::database_silent)?;
        if !resp.status().is_success() {
            return Err(AppError::CannotDo(format!("The avatar search service refused the search ({}).", resp.status().as_u16())));
        }
        let body: Value = resp.json().await.map_err(Self::database_odd)?;
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
    pub async fn request_invite(&self, user_id: &str, slot: Option<u8>) -> AppResult<()> {
        let path = format!("/requestInvite/{}", Self::part(user_id));
        let body = match slot {
            Some(slot) => serde_json::json!({ "requestSlot": slot }),
            None => serde_json::json!({}),
        };
        self.act(Method::POST, path, Some(body)).await
    }

    pub async fn world(&self, world_id: &str) -> AppResult<Value> {
        let path = format!("/worlds/{}", utf8_percent_encode(world_id, COMPONENT));
        let (status, body) = self.send(Method::GET, &path, None, None).await?;
        if !status.is_success() {
            return Err(Self::api_error(status, &body));
        }
        Ok(body)
    }

    /// Signs out here at once. VRChat is told in the background: during a slowdown that waits
    /// its turn like a read, and it carries the cookie of the sign-in that is leaving, so a new
    /// sign-in made in the meantime is never sent along or undone.
    pub async fn logout(self: &Arc<Self>) -> AppResult<()> {
        let leaving = self.auth_token().is_some().then(|| self.cookie_header()).flatten();
        self.clear_auth();
        if let Some(cookie) = leaving {
            let client = self.clone();
            tokio::spawn(async move {
                let _ = client.send_with(Method::PUT, "/logout", None, None, Some(&cookie)).await;
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::address_allowed;

    #[test]
    fn outside_addresses_never_lead_to_this_pc_or_the_home_network() {
        assert!(address_allowed("https://api.vrchat.cloud/api/1/file/file_1/1/file", false));
        assert!(address_allowed("https://images.example.org/pic.png", false));
        for url in [
            "http://127.0.0.1:8080/admin/delete?all=1",
            "http://images.example.org/pic.png",
            "https://localhost/pic.png",
            "https://127.0.0.1:8443/pic.png",
            "https://192.168.1.20/pic.png",
            "https://10.0.0.1/pic.png",
            "https://169.254.169.254/latest",
            "https://[::1]/pic.png",
            "https://[fd00::1]/pic.png",
            "https://[::ffff:127.0.0.1]/pic.png",
            "file:///c:/secret.txt",
            "https://example.org/a b.png",
        ] {
            assert!(!address_allowed(url, false), "{url}");
        }
        // Only a client that itself talks to a stand-in server on this PC may use one.
        assert!(address_allowed("http://127.0.0.1:8080/pic.png", true));
        assert!(!address_allowed("http://192.168.1.20/pic.png", true));
    }
}
