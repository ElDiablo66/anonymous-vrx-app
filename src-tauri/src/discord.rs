//! Discord rich presence: "what I am doing" on my Discord profile.
//!
//! Discord's desktop app listens on a local pipe (`\\.\pipe\discord-ipc-0` up to `-9`). A program
//! says which Discord application it is, then sends the activity to show. Nothing goes over
//! the network from here; Discord itself passes it on. If Discord is not running, nothing is
//! sent and the worker simply tries again later.
//!
//! Messages on the pipe are a little-endian header (operation, length) followed by JSON.

use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

/// What to show. Every text is already decided by the window; this module only delivers it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Presence {
    /// The Discord application the activity is shown under. Digits only.
    pub client_id: String,
    pub details: String,
    pub state: String,
    /// When I arrived, in seconds since 1970. Discord shows the time elapsed.
    pub started_at: Option<i64>,
    /// A picture address, or empty.
    pub image: String,
    pub image_text: String,
    pub button_label: String,
    pub button_url: String,
    /// A second button, shown under the first.
    pub button2_label: String,
    pub button2_url: String,
    /// The small round picture in the corner: the name of an art asset uploaded to the Discord
    /// application, or a picture address. Empty for none.
    pub small_image: String,
    pub small_text: String,
}

/// What I want shown right now. `None` clears it.
static WANT: Mutex<Option<Presence>> = Mutex::new(None);
static WORKER: Once = Once::new();

/// A client id is a Discord snowflake: 17 to 20 digits.
pub fn valid_client_id(id: &str) -> bool {
    (17..=20).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
}

/// Discord rejects very short or very long lines; keeps a text between 2 and 128 characters.
fn fit(text: &str) -> Option<String> {
    let text = text.trim();
    let count = text.chars().count();
    if count < 2 {
        return None;
    }
    Some(if count > 128 { text.chars().take(127).chain(std::iter::once('…')).collect() } else { text.to_string() })
}

/// One message for the pipe.
pub fn frame(op: u32, body: &Value) -> Vec<u8> {
    let text = body.to_string();
    let mut out = Vec::with_capacity(8 + text.len());
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
    out
}

/// The "set activity" command for one presence, or the command that clears it.
pub fn activity_command(presence: Option<&Presence>, pid: u32, nonce: u64) -> Value {
    let activity = presence.map(|p| {
        let mut a = json!({ "type": 0 });
        if let Some(details) = fit(&p.details) {
            a["details"] = Value::String(details);
        }
        if let Some(state) = fit(&p.state) {
            a["state"] = Value::String(state);
        }
        if let Some(start) = p.started_at.filter(|s| *s > 0) {
            a["timestamps"] = json!({ "start": start });
        }
        if p.image.starts_with("https://") {
            a["assets"] = json!({ "large_image": p.image });
            if let Some(text) = fit(&p.image_text) {
                a["assets"]["large_text"] = Value::String(text);
            }
            // The corner picture only shows on top of a large one. An asset name is plain
            // letters, digits, dashes and underscores.
            let small = p.small_image.trim();
            let named = !small.is_empty() && small.len() <= 64 && small.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
            if named || small.starts_with("https://") {
                a["assets"]["small_image"] = Value::String(small.to_string());
                if let Some(text) = fit(&p.small_text) {
                    a["assets"]["small_text"] = Value::String(text);
                }
            }
        }
        let mut buttons = Vec::new();
        for (label, url) in [(&p.button_label, &p.button_url), (&p.button2_label, &p.button2_url)] {
            let label: String = label.trim().chars().take(32).collect();
            if !label.is_empty() && url.starts_with("https://") && url.len() <= 512 {
                buttons.push(json!({ "label": label, "url": url }));
            }
        }
        if !buttons.is_empty() {
            a["buttons"] = Value::Array(buttons);
        }
        a
    });
    json!({
        "cmd": "SET_ACTIVITY",
        "args": { "pid": pid, "activity": activity },
        "nonce": format!("avrx-{nonce}"),
    })
}

/// Says what should be shown from now on. Starts the worker the first time.
pub fn show(presence: Option<Presence>) {
    let presence = presence.filter(|p| valid_client_id(&p.client_id));
    *WANT.lock().unwrap_or_else(|e| e.into_inner()) = presence;
    WORKER.call_once(|| {
        let _ = std::thread::Builder::new().name("discord".into()).spawn(worker);
    });
}

#[cfg(windows)]
fn connect(client_id: &str) -> Option<std::fs::File> {
    use std::io::{Read, Write};
    for n in 0..10 {
        let path = format!(r"\\.\pipe\discord-ipc-{n}");
        let Ok(mut pipe) = std::fs::OpenOptions::new().read(true).write(true).open(&path) else { continue };
        if pipe.write_all(&frame(0, &json!({ "v": 1, "client_id": client_id }))).is_err() {
            continue;
        }
        // Discord answers the greeting; a refusal (unknown application) closes the pipe.
        let mut head = [0u8; 8];
        if pipe.read_exact(&mut head).is_err() {
            continue;
        }
        let op = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        let mut body = vec![0u8; len.min(64 * 1024)];
        if pipe.read_exact(&mut body).is_err() || op == 2 {
            continue;
        }
        return Some(pipe);
    }
    None
}

#[cfg(windows)]
fn send(pipe: &mut std::fs::File, command: &Value) -> bool {
    use std::io::{Read, Write};
    if pipe.write_all(&frame(1, command)).is_err() {
        return false;
    }
    // Every command gets an answer; reading it keeps the pipe from filling up.
    let mut head = [0u8; 8];
    if pipe.read_exact(&mut head).is_err() {
        return false;
    }
    let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
    let mut body = vec![0u8; len.min(64 * 1024)];
    pipe.read_exact(&mut body).is_ok()
}

#[cfg(windows)]
fn worker() {
    let pid = std::process::id();
    let mut pipe: Option<(String, std::fs::File)> = None;
    let mut shown: Option<Presence> = None;
    let mut nonce = 0u64;
    let mut last_try = Instant::now() - Duration::from_secs(60);
    let mut last_sent = Instant::now() - Duration::from_secs(60);
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let want = WANT.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if want == shown {
            continue;
        }
        // Discord allows only a few updates in a row; changes closer than 4 seconds wait.
        if last_sent.elapsed() < Duration::from_secs(4) {
            continue;
        }
        // A different application id needs a new greeting.
        if let (Some((id, _)), Some(w)) = (&pipe, &want) {
            if *id != w.client_id {
                pipe = None;
            }
        }
        if pipe.is_none() {
            let Some(id) = want.as_ref().map(|w| w.client_id.clone()) else {
                // Nothing to show and no connection: nothing to clear either.
                shown = None;
                continue;
            };
            // Discord may simply not be running; look again every 15 seconds.
            if last_try.elapsed() < Duration::from_secs(15) {
                continue;
            }
            last_try = Instant::now();
            pipe = connect(&id).map(|p| (id, p));
        }
        let Some((_, file)) = pipe.as_mut() else { continue };
        nonce += 1;
        if send(file, &activity_command(want.as_ref(), pid, nonce)) {
            shown = want;
            last_sent = Instant::now();
        } else {
            pipe = None;
        }
    }
}

#[cfg(not(windows))]
fn worker() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Presence {
        Presence {
            client_id: "123456789012345678".into(),
            details: "Quiet Pier at Dusk".into(),
            state: "Friends+, 3 people".into(),
            started_at: Some(1_795_000_000),
            image: "https://example.test/pier.png".into(),
            image_text: "Quiet Pier at Dusk".into(),
            button_label: "Join".into(),
            button_url: "https://vrchat.com/home/launch?worldId=wrld_a&instanceId=1".into(),
            button2_label: "World page".into(),
            button2_url: "https://vrchat.com/home/world/wrld_a".into(),
            small_image: "logo".into(),
            small_text: "Anonymous VRX".into(),
        }
    }

    #[test]
    fn frames_carry_operation_and_length() {
        let bytes = frame(1, &json!({ "a": 1 }));
        assert_eq!(&bytes[0..4], &1u32.to_le_bytes());
        assert_eq!(&bytes[4..8], &7u32.to_le_bytes());
        assert_eq!(&bytes[8..], br#"{"a":1}"#);
    }

    #[test]
    fn activity_holds_what_was_asked_and_nothing_else() {
        let cmd = activity_command(Some(&sample()), 42, 7);
        let a = &cmd["args"]["activity"];
        assert_eq!(cmd["cmd"], "SET_ACTIVITY");
        assert_eq!(cmd["args"]["pid"], 42);
        assert_eq!(a["details"], "Quiet Pier at Dusk");
        assert_eq!(a["state"], "Friends+, 3 people");
        assert_eq!(a["timestamps"]["start"], 1_795_000_000i64);
        assert_eq!(a["assets"]["large_image"], "https://example.test/pier.png");
        assert_eq!(a["buttons"][0]["label"], "Join");
        assert_eq!(a["buttons"][1]["url"], "https://vrchat.com/home/world/wrld_a");
        assert_eq!(a["assets"]["small_image"], "logo");
        assert_eq!(a["assets"]["small_text"], "Anonymous VRX");

        // Empty parts are left out instead of being sent empty.
        let bare = Presence { client_id: "123456789012345678".into(), details: "In a private world".into(), ..Default::default() };
        let a = activity_command(Some(&bare), 1, 1)["args"]["activity"].clone();
        assert_eq!(a["details"], "In a private world");
        assert!(a.get("state").is_none() && a.get("assets").is_none() && a.get("buttons").is_none() && a.get("timestamps").is_none());

        // Clearing sends no activity at all.
        assert!(activity_command(None, 1, 1)["args"]["activity"].is_null());
    }

    #[test]
    fn only_real_application_ids_and_safe_links_pass() {
        assert!(valid_client_id("123456789012345678"));
        assert!(!valid_client_id(""));
        assert!(!valid_client_id("12345"));
        assert!(!valid_client_id("12345678901234567x"));
        let mut odd = sample();
        odd.button_url = "file:///C:/secret".into();
        odd.button2_url = "javascript:alert(1)".into();
        odd.image = "http://plain.test/a.png".into();
        let a = activity_command(Some(&odd), 1, 1)["args"]["activity"].clone();
        assert!(a.get("buttons").is_none() && a.get("assets").is_none());
        // Long texts are cut to what Discord takes.
        odd.details = "x".repeat(300);
        let a = activity_command(Some(&odd), 1, 1)["args"]["activity"].clone();
        assert_eq!(a["details"].as_str().unwrap().chars().count(), 128);
    }
}
