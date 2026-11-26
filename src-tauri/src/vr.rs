//! Notices sent into the headset. This app draws nothing in VR itself: it hands the text to
//! an overlay program that is already running on this PC and listens on a local port.
//!
//! XSOverlay takes them on `ws://localhost:42070` and OVR Toolkit on `ws://127.0.0.1:11450/api`.
//! Nothing leaves the PC.

use std::time::Duration;

use futures_util::SinkExt;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

/// What the window asked for.
#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rules {
    pub xs: bool,
    pub ovr_hud: bool,
    pub ovr_wrist: bool,
    /// The app's own notice panel in SteamVR.
    #[serde(default)]
    pub own: bool,
    /// How long a notice stays up in XSOverlay and in the app's own panel.
    pub seconds: f32,
    /// 0.2 (faint) to 1 (solid), XSOverlay only.
    pub opacity: f32,
}

impl Rules {
    pub fn any(&self) -> bool {
        self.xs || self.ovr_hud || self.ovr_wrist || self.own
    }
}

/// How one overlay program took a notice.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sent {
    pub target: &'static str,
    pub ok: bool,
    /// Why not, in plain words. Empty when it went through.
    pub problem: String,
}

const NAME: &str = "Anonymous VRX";

/// The message XSOverlay expects. Its `jsonData` is JSON written out as text.
pub fn xs_message(title: &str, body: &str, rules: &Rules) -> String {
    let notice = json!({
        "type": 1,
        "title": title,
        "content": body,
        "timeout": rules.seconds.clamp(1.0, 30.0),
        // A notice with two lines of text needs more room than a title alone.
        "height": if body.is_empty() { 110 } else { 175 },
        "opacity": rules.opacity.clamp(0.2, 1.0),
        "volume": 0.7,
        "audioPath": "default",
        "icon": "default",
        "useBase64Icon": false,
        "sourceApp": NAME,
    });
    json!({
        "sender": NAME,
        "target": "xsoverlay",
        "command": "SendNotification",
        "jsonData": notice.to_string(),
        "rawData": Value::Null,
    })
    .to_string()
}

/// The message OVR Toolkit expects, for its HUD or for the wrist. `json` is JSON written out as text.
pub fn ovr_message(wrist: bool, title: &str, body: &str) -> String {
    if wrist {
        let line = if body.is_empty() { title.to_string() } else { format!("{title}: {body}") };
        json!({ "messageType": "SendWristNotification", "json": json!({ "body": line }).to_string() }).to_string()
    } else {
        json!({ "messageType": "SendNotification", "json": json!({ "title": title, "body": body, "icon": Value::Null }).to_string() }).to_string()
    }
}

/// Opens the local port, hands over one message and closes again.
async fn hand_over(url: &str, text: String) -> Result<(), String> {
    let work = async {
        let (mut socket, _) = tokio_tungstenite::connect_async(url).await.map_err(|_| "It is not running, or its outside messages are switched off.".to_string())?;
        socket.send(Message::text(text)).await.map_err(|e| e.to_string())?;
        let _ = socket.close(None).await;
        Ok(())
    };
    match tokio::time::timeout(Duration::from_secs(3), work).await {
        Ok(done) => done,
        Err(_) => Err("It did not answer within three seconds.".into()),
    }
}

/// Sends one notice to every overlay program switched on, and says how each took it.
pub async fn send(rules: Rules, title: String, body: String) -> Vec<Sent> {
    let mut out = Vec::new();
    let mut note = |target: &'static str, done: Result<(), String>| {
        out.push(Sent { target, ok: done.is_ok(), problem: done.err().unwrap_or_default() });
    };
    if rules.xs {
        note("XSOverlay", hand_over("ws://localhost:42070/?client=AnonymousVRX", xs_message(&title, &body, &rules)).await);
    }
    if rules.ovr_hud {
        note("OVR Toolkit, in view", hand_over("ws://127.0.0.1:11450/api", ovr_message(false, &title, &body)).await);
    }
    if rules.ovr_wrist {
        note("OVR Toolkit, on the wrist", hand_over("ws://127.0.0.1:11450/api", ovr_message(true, &title, &body)).await);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xsoverlay_gets_its_notice_as_text_inside_the_envelope() {
        let rules = Rules { xs: true, seconds: 99.0, opacity: 0.0, ..Rules::default() };
        let outer: Value = serde_json::from_str(&xs_message("Mika is online", "In The Pier", &rules)).unwrap();
        assert_eq!(outer["target"], "xsoverlay");
        assert_eq!(outer["command"], "SendNotification");
        let inner: Value = serde_json::from_str(outer["jsonData"].as_str().expect("text, not an object")).unwrap();
        assert_eq!(inner["title"], "Mika is online");
        assert_eq!(inner["content"], "In The Pier");
        // Out-of-range choices are pulled back to what XSOverlay shows properly.
        assert_eq!(inner["timeout"], 30.0);
        assert!((inner["opacity"].as_f64().unwrap() - 0.2).abs() < 1e-6);
    }

    #[test]
    fn ovr_toolkit_gets_a_title_in_view_and_one_line_on_the_wrist() {
        let hud: Value = serde_json::from_str(&ovr_message(false, "Invite", "From Mika")).unwrap();
        assert_eq!(hud["messageType"], "SendNotification");
        let inner: Value = serde_json::from_str(hud["json"].as_str().unwrap()).unwrap();
        assert_eq!(inner["title"], "Invite");
        assert_eq!(inner["body"], "From Mika");

        let wrist: Value = serde_json::from_str(&ovr_message(true, "Invite", "From Mika")).unwrap();
        assert_eq!(wrist["messageType"], "SendWristNotification");
        let inner: Value = serde_json::from_str(wrist["json"].as_str().unwrap()).unwrap();
        assert_eq!(inner["body"], "Invite: From Mika");
    }
}
