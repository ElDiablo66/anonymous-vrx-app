//! Notices sent into the headset. They go to an overlay program already running on this PC
//! that listens on a local port, or to the app's own notice panel in SteamVR.
//!
//! XSOverlay takes them on `ws://localhost:42070` and OVR Toolkit on `ws://127.0.0.1:11450/api`.
//! Nothing leaves the PC.
//!
//! One notice shows in one place: the places switched on are tried in order and the first that
//! takes it is the only one that gets it. OVR Toolkit is one place, in view or on the wrist; with
//! both switched on (settings saved before that was one choice), in view wins. The test notice is
//! the one exception: it goes to each program switched on, so each can be checked.

use std::time::Duration;

use futures_util::SinkExt;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

pub const XS_URL: &str = "ws://localhost:42070/?client=AnonymousVRX";
pub const OVR_URL: &str = "ws://127.0.0.1:11450/api";

/// Which place a notice tries first when the app's own panel and another program are both on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Route {
    /// The other programs first (XSOverlay, then OVR Toolkit); the app's own panel only when
    /// none of them takes it.
    #[default]
    Xs,
    /// The app's own panel first; the other programs only when the app is not in SteamVR.
    Own,
}

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
    #[serde(default)]
    pub route: Route,
    /// How long a notice stays up in XSOverlay and in the app's own panel.
    pub seconds: f32,
    /// 0.2 (faint) to 1 (solid). OVR Toolkit takes no opacity, so here it only reaches XSOverlay.
    pub opacity: f32,
}

impl Rules {
    pub fn any(&self) -> bool {
        self.xs || self.ovr_hud || self.ovr_wrist || self.own
    }

    /// Where OVR Toolkit shows a notice, if it is switched on. It is one place: a notice goes in
    /// view or on the wrist, never both.
    pub fn ovr(&self) -> Option<Target> {
        if self.ovr_hud {
            Some(Target::OvrHud)
        } else if self.ovr_wrist {
            Some(Target::OvrWrist)
        } else {
            None
        }
    }

    /// The places switched on, in the order a notice tries them.
    pub fn order(&self) -> Vec<Target> {
        let ovr = self.ovr();
        let others = [(self.xs, Target::Xs), (ovr == Some(Target::OvrHud), Target::OvrHud), (ovr == Some(Target::OvrWrist), Target::OvrWrist)];
        let own = [(self.own, Target::Own)];
        let all: Vec<(bool, Target)> = match self.route {
            Route::Xs => others.into_iter().chain(own).collect(),
            Route::Own => own.into_iter().chain(others).collect(),
        };
        all.into_iter().filter_map(|(on, target)| on.then_some(target)).collect()
    }
}

/// A place a notice can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Xs,
    Own,
    OvrHud,
    OvrWrist,
}

/// Hands one notice to the first place that takes it, and says which one that was. The places
/// after it are not tried, so one alert makes one notice.
/// `own_ready` is whether the app's own panel is in SteamVR right now; the caller draws it.
pub async fn deliver(rules: &Rules, title: &str, body: &str, own_ready: bool, xs_url: &str, ovr_url: &str) -> Option<Target> {
    for target in rules.order() {
        let took = match target {
            Target::Xs => hand_over(xs_url, xs_message(title, body, rules)).await.is_ok(),
            Target::Own => own_ready,
            Target::OvrHud => hand_over(ovr_url, ovr_message(false, title, body)).await.is_ok(),
            Target::OvrWrist => hand_over(ovr_url, ovr_message(true, title, body)).await.is_ok(),
        };
        if took {
            return Some(target);
        }
    }
    None
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

/// Sends one test notice to every overlay program switched on, and says how each took it. OVR
/// Toolkit gets it in the one place a real notice would use.
pub async fn send(rules: Rules, title: String, body: String) -> Vec<Sent> {
    send_to(rules, title, body, XS_URL, OVR_URL).await
}

async fn send_to(rules: Rules, title: String, body: String, xs_url: &str, ovr_url: &str) -> Vec<Sent> {
    let mut out = Vec::new();
    let mut note = |target: &'static str, done: Result<(), String>| {
        out.push(Sent { target, ok: done.is_ok(), problem: done.err().unwrap_or_default() });
    };
    if rules.xs {
        note("XSOverlay", hand_over(xs_url, xs_message(&title, &body, &rules)).await);
    }
    match rules.ovr() {
        Some(Target::OvrHud) => note("OVR Toolkit, in view", hand_over(ovr_url, ovr_message(false, &title, &body)).await),
        Some(Target::OvrWrist) => note("OVR Toolkit, on the wrist", hand_over(ovr_url, ovr_message(true, &title, &body)).await),
        _ => {}
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

    #[test]
    fn the_places_are_tried_in_the_order_chosen() {
        let all = Rules { xs: true, own: true, ovr_wrist: true, ..Rules::default() };
        assert_eq!(all.order(), [Target::Xs, Target::OvrWrist, Target::Own]);
        let own_first = Rules { route: Route::Own, ..all };
        assert_eq!(own_first.order(), [Target::Own, Target::Xs, Target::OvrWrist]);
        // OVR Toolkit is one place: with both of its switches on, in view wins.
        let both = Rules { ovr_hud: true, ..all };
        assert_eq!(both.order(), [Target::Xs, Target::OvrHud, Target::Own]);
        // OVR Toolkit without XSOverlay also comes before the app's panel when the others go first.
        let ovr = Rules { own: true, ovr_hud: true, ..Rules::default() };
        assert_eq!(ovr.order(), [Target::OvrHud, Target::Own]);
        assert_eq!(Rules { route: Route::Own, ..ovr }.order(), [Target::Own, Target::OvrHud]);
        let only_ovr = Rules { ovr_wrist: true, ..Rules::default() };
        assert_eq!(only_ovr.order(), [Target::OvrWrist]);
        // Settings saved before the choice existed read as XSOverlay first.
        let old: Rules = serde_json::from_value(json!({ "xs": true, "ovrHud": false, "ovrWrist": false, "own": true, "seconds": 4, "opacity": 1 })).unwrap();
        assert_eq!(old.route, Route::Xs);
    }

    /// A stand-in for XSOverlay on a free local port. It counts the messages it is handed.
    async fn fake_xsoverlay() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use futures_util::StreamExt;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/?client=AnonymousVRX", listener.local_addr().unwrap());
        let count = std::sync::Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let seen = seen.clone();
                tokio::spawn(async move {
                    let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else { return };
                    while let Some(Ok(message)) = socket.next().await {
                        if let Message::Text(text) = message {
                            let outer: Value = serde_json::from_str(&text).unwrap();
                            assert_eq!(outer["command"], "SendNotification");
                            seen.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                });
            }
        });
        (url, count)
    }

    /// A stand-in for any overlay program on a free local port. It counts every text message.
    async fn counting_port() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use futures_util::StreamExt;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/api", listener.local_addr().unwrap());
        let count = std::sync::Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let seen = seen.clone();
                tokio::spawn(async move {
                    let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else { return };
                    while let Some(Ok(message)) = socket.next().await {
                        if let Message::Text(_) = message {
                            seen.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                });
            }
        });
        (url, count)
    }

    /// A local address where nothing listens.
    async fn nobody() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/", listener.local_addr().unwrap());
        drop(listener);
        url
    }

    #[tokio::test]
    async fn with_xsoverlay_running_one_alert_makes_exactly_one_notice() {
        use std::sync::atomic::Ordering;
        let (xs, count) = fake_xsoverlay().await;
        let ovr = nobody().await;
        let rules = Rules { xs: true, own: true, seconds: 4.0, opacity: 1.0, ..Rules::default() };
        // XSOverlay first: XSOverlay takes it, and the app's own panel is not drawn.
        assert_eq!(deliver(&rules, "Mossy is online", "In Quiet Pier", true, &xs, &ovr).await, Some(Target::Xs));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
        // The app's own panel first: XSOverlay hears nothing.
        let own_first = Rules { route: Route::Own, ..rules };
        assert_eq!(deliver(&own_first, "Mossy is online", "In Quiet Pier", true, &xs, &ovr).await, Some(Target::Own));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
        // The own panel out of SteamVR: XSOverlay takes it instead, once.
        assert_eq!(deliver(&own_first, "Mossy is online", "", false, &xs, &ovr).await, Some(Target::Xs));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn with_nothing_else_running_the_own_panel_takes_it() {
        let (xs, ovr) = (nobody().await, nobody().await);
        let rules = Rules { xs: true, own: true, ovr_hud: true, seconds: 4.0, opacity: 1.0, ..Rules::default() };
        assert_eq!(deliver(&rules, "Invite", "", true, &xs, &ovr).await, Some(Target::Own));
        // Nobody to take it at all.
        assert_eq!(deliver(&rules, "Invite", "", false, &xs, &ovr).await, None);
    }

    #[tokio::test]
    async fn with_ovr_toolkit_one_alert_makes_exactly_one_notice() {
        use std::sync::atomic::Ordering;
        let (xs, xs_count) = fake_xsoverlay().await;
        let (ovr, ovr_count) = counting_port().await;
        let gone = nobody().await;
        // Both OVR Toolkit switches on, as settings saved before it was one choice can be.
        let rules = Rules { xs: true, ovr_hud: true, ovr_wrist: true, own: true, seconds: 4.0, opacity: 1.0, ..Rules::default() };
        // XSOverlay running: it takes the notice and OVR Toolkit hears nothing.
        assert_eq!(deliver(&rules, "Invite", "From Mika", true, &xs, &ovr).await, Some(Target::Xs));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(xs_count.load(Ordering::SeqCst), 1);
        assert_eq!(ovr_count.load(Ordering::SeqCst), 0);
        // XSOverlay not running: OVR Toolkit takes it once, in view, and the own panel is not drawn.
        assert_eq!(deliver(&rules, "Invite", "", true, &gone, &ovr).await, Some(Target::OvrHud));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(ovr_count.load(Ordering::SeqCst), 1);
        // On the wrist alone: the wrist takes it, once.
        let wrist = Rules { ovr_hud: false, ..rules };
        assert_eq!(deliver(&wrist, "Invite", "", true, &gone, &ovr).await, Some(Target::OvrWrist));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(ovr_count.load(Ordering::SeqCst), 2);
        // The own panel first and in SteamVR: the other programs hear nothing.
        let own_first = Rules { route: Route::Own, ..rules };
        assert_eq!(deliver(&own_first, "Invite", "", true, &xs, &ovr).await, Some(Target::Own));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(xs_count.load(Ordering::SeqCst), 1);
        assert_eq!(ovr_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn the_test_notice_reaches_ovr_toolkit_in_the_one_place_set() {
        use std::sync::atomic::Ordering;
        let (xs, xs_count) = fake_xsoverlay().await;
        let (ovr, ovr_count) = counting_port().await;
        let rules = Rules { xs: true, ovr_hud: true, ovr_wrist: true, seconds: 4.0, opacity: 1.0, ..Rules::default() };
        let sent = send_to(rules, "Anonymous VRX".into(), "Test".into(), &xs, &ovr).await;
        let names: Vec<&str> = sent.iter().map(|s| s.target).collect();
        assert_eq!(names, ["XSOverlay", "OVR Toolkit, in view"]);
        assert!(sent.iter().all(|s| s.ok));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(xs_count.load(Ordering::SeqCst), 1);
        assert_eq!(ovr_count.load(Ordering::SeqCst), 1);
    }
}
