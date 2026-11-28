//! The app's own panels in SteamVR: a notice in front of the eyes, a small feed on the arm and a
//! tab in SteamVR's dashboard. It also watches what SteamVR says about the controllers, trackers
//! and headset (battery, tracking, switched off) and when SteamVR is about to close.
//!
//! The window draws each panel as a picture; this file only hands the picture to SteamVR and
//! says where it floats. It talks to SteamVR through `openvr_api.dll` from SteamVR's own
//! folder, loaded when SteamVR is running and let go of when SteamVR closes. Nothing is
//! bundled, and a PC without SteamVR never loads anything.
//!
//! The numbers below (which function sits where) belong to `IVROverlay_028`, `IVRSystem_026`
//! and `IVRNotifications_002` in Valve's `openvr.h` and `openvr_capi.h`. A SteamVR that does
//! not know those versions refuses the request, and the panels stay off.

use std::collections::BTreeMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const OVERLAY_TABLE: &str = "FnTable:IVROverlay_028";
const SYSTEM_TABLE: &str = "FnTable:IVRSystem_026";
const NOTIFICATIONS_TABLE: &str = "FnTable:IVRNotifications_002";

// Places in the overlay table.
const CREATE_OVERLAY: usize = 1;
const DESTROY_OVERLAY: usize = 3;
const SET_FLAG: usize = 11;
const SET_ALPHA: usize = 16;
const SET_SORT_ORDER: usize = 20;
const SET_WIDTH: usize = 22;
const SET_TRANSFORM_DEVICE: usize = 35;
const SHOW_OVERLAY: usize = 43;
const HIDE_OVERLAY: usize = 44;
const POLL_OVERLAY_EVENT: usize = 48;
const SET_INPUT_METHOD: usize = 50;
const SET_MOUSE_SCALE: usize = 52;
const SET_FROM_FILE: usize = 63;
const CREATE_DASHBOARD: usize = 67;
// Places in the system table.
const TRACKING_POSE: usize = 12;
const CONTROLLER_FOR_ROLE: usize = 18;
const ROLE_FOR_DEVICE: usize = 19;
const DEVICE_CLASS: usize = 20;
const BOOL_PROPERTY: usize = 22;
const FLOAT_PROPERTY: usize = 23;
const POLL_EVENT: usize = 30;
const ACKNOWLEDGE_QUIT: usize = 47;
// Places in the notifications table.
const CREATE_NOTIFICATION: usize = 0;

const APP_OVERLAY: i32 = 2;
const HMD: u32 = 0;
const NO_DEVICE: u32 = u32::MAX;
const MAX_DEVICES: usize = 64;

// Events, from `EVREventType`.
const EVENT_ACTIVATED: u32 = 100;
const EVENT_DEACTIVATED: u32 = 101;
const EVENT_PROPERTY_CHANGED: u32 = 111;
const EVENT_WIRELESS_DISCONNECT: u32 = 112;
const EVENT_WIRELESS_RECONNECT: u32 = 113;
const EVENT_MOUSE_UP: u32 = 302;
const EVENT_QUIT: u32 = 700;
const EVENT_DRIVER_QUIT: u32 = 704;

// `EVRMouseButton::VRMouseButton_Left` and `VROverlayInputMethod::Mouse`.
const MOUSE_LEFT: u32 = 1;
const INPUT_MOUSE: i32 = 1;
/// `VROverlayFlags_VisibleInDashboard`: without it a panel hides while SteamVR's dashboard is open.
const VISIBLE_IN_DASHBOARD: i32 = 1 << 15;
/// Every panel picture is this many pixels wide; the dashboard's clicks come back in pixels.
const PICTURE_WIDTH: f32 = 1024.0;

// Device properties, from `ETrackedDeviceProperty`.
const PROP_CHARGING: i32 = 1011;
const PROP_BATTERY: i32 = 1012;
const PROP_HAS_BATTERY: i32 = 1026;

// Kinds of device, from `ETrackedDeviceClass`.
const CLASS_HMD: i32 = 1;
const CLASS_CONTROLLER: i32 = 2;
const CLASS_TRACKER: i32 = 3;
// `ETrackedControllerRole`.
const ROLE_LEFT: i32 = 1;
const ROLE_RIGHT: i32 = 2;
// `ETrackingResult::Running_OK` and `ETrackingUniverseOrigin::Standing`.
const TRACKING_OK: i32 = 200;
const STANDING: i32 = 1;

/// How long a device may go without a good pose before it counts as lost.
const LOST_AFTER: Duration = Duration::from_secs(3);
/// How long it has to be seen again before a new loss is told.
const BACK_AFTER: Duration = Duration::from_secs(1);
/// Losses that come this close together are told as one notice.
const LOSS_BATCH: Duration = Duration::from_millis(1500);
/// The shortest time between two notices about the same device losing tracking or switching off.
const LOSS_AGAIN_AFTER: Duration = Duration::from_secs(60);
/// The shortest time between two tracking notices of any kind; what comes in between waits and
/// goes out together.
const NOTICE_GAP: Duration = Duration::from_secs(15);
/// How long after joining SteamVR the dashboard tab is made.
const DASHBOARD_SETTLE: Duration = Duration::from_secs(8);

type Handle = u64;
/// Three rows of four: a turn, and in the last column a move in metres.
type Matrix = [[f32; 4]; 3];

type InitFn = unsafe extern "C" fn(*mut i32, i32) -> isize;
type ShutdownFn = unsafe extern "C" fn();
type InterfaceFn = unsafe extern "C" fn(*const c_char, *mut i32) -> isize;
type ErrorNameFn = unsafe extern "C" fn(i32) -> *const c_char;

/// `VREvent_t` as SteamVR lays it out on 64-bit Windows: three 4-byte fields, then a union of
/// 48 bytes that starts on an 8-byte boundary. 64 bytes in all; SteamVR checks the size given.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct VrEvent {
    kind: u32,
    device: u32,
    age: f32,
    data: [u64; 6],
}

/// `TrackedDevicePose_t`: where a device is, how it moves, and whether that can be trusted.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct Pose {
    to_absolute: Matrix,
    velocity: [f32; 3],
    angular_velocity: [f32; 3],
    result: i32,
    valid: bool,
    connected: bool,
}

/// Which hand wears the feed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Hand {
    Left,
    Right,
}

/// Where on the arm the feed sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Spot {
    /// Further up the arm than the back of the wrist, so it sits above a watch-like panel
    /// another program (XSOverlay) keeps on the wrist itself.
    Forearm,
    /// On the back of the wrist, like a watch.
    Wrist,
    /// On the inside of the wrist, read with the palm turned up.
    Inside,
}

/// The feed for the arm, as the window drew it.
#[derive(Clone, Debug)]
pub struct WristPanel {
    pub picture: PathBuf,
    pub hand: Hand,
    pub spot: Spot,
    /// Width in metres.
    pub width: f32,
    pub alpha: f32,
    /// The picture's height divided by its width.
    pub aspect: f32,
    /// Metres along the arm on top of where the spot puts it: more is nearer the elbow.
    pub shift: f32,
}

/// A device SteamVR knows, as the wrist and the dashboard show it.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceState {
    /// Short: Left, Right, Tracker 2, Headset.
    pub name: String,
    /// Whole percent, or nothing when the device does not say.
    pub battery: Option<u8>,
    pub charging: bool,
    /// Tracking lost for longer than three seconds and not back yet.
    pub lost: bool,
}

/// What the watch has to tell the rest of the app.
#[derive(Clone, Debug, PartialEq)]
pub enum News {
    /// Something for the headset. `kind` picks the panel's colour, `label` names it.
    Alert { kind: &'static str, label: &'static str, title: String, body: String },
    /// Every device now, sent when anything shown about them changes.
    Devices(Vec<DeviceState>),
    /// A click on the dashboard tab, in pixels of its picture from the top left corner.
    DashClick { x: f32, y: f32 },
}

pub type NewsFn = Arc<dyn Fn(News) + Send + Sync>;

enum Cmd {
    /// Whether the app should be in SteamVR at all.
    Wanted(bool),
    /// Whether battery, tracking and closing are told as alerts.
    Watch(bool),
    Notice { picture: PathBuf, seconds: f32, alpha: f32, above: bool },
    /// `None` takes the feed off the arm.
    Wrist(Option<WristPanel>),
    /// The dashboard tab's picture, its height over its width, and its small icon, or `None`
    /// to take the tab away.
    Dashboard(Option<(PathBuf, f32, Option<PathBuf>)>),
    /// Sends one of SteamVR's own notifications and answers with how SteamVR took it.
    TryNative(Sender<String>),
}

/// The handle the rest of the app holds. Everything behind it runs on one thread of its own.
pub struct Overlay {
    tx: Mutex<Sender<Cmd>>,
    status: Arc<Mutex<String>>,
    connected: Arc<AtomicBool>,
}

impl Overlay {
    /// `steamvr_running` is asked every few seconds while the panels are wanted. `news` hears
    /// about devices and gets the alerts to show.
    pub fn start(steamvr_running: fn() -> bool, news: NewsFn) -> Self {
        let (tx, rx) = channel();
        let status = Arc::new(Mutex::new("Switched off.".to_string()));
        let connected = Arc::new(AtomicBool::new(false));
        let (shared, link) = (status.clone(), connected.clone());
        std::thread::spawn(move || run(rx, shared, link, steamvr_running, news));
        Self { tx: Mutex::new(tx), status, connected }
    }

    fn send(&self, cmd: Cmd) {
        let _ = self.tx.lock().unwrap().send(cmd);
    }

    pub fn wanted(&self, on: bool) {
        self.send(Cmd::Wanted(on));
    }

    pub fn watch(&self, on: bool) {
        self.send(Cmd::Watch(on));
    }

    pub fn notice(&self, picture: PathBuf, seconds: f32, alpha: f32, above: bool) {
        self.send(Cmd::Notice { picture, seconds, alpha, above });
    }

    pub fn wrist(&self, feed: Option<WristPanel>) {
        self.send(Cmd::Wrist(feed));
    }

    pub fn dashboard(&self, pictures: Option<(PathBuf, f32, Option<PathBuf>)>) {
        self.send(Cmd::Dashboard(pictures));
    }

    /// True while the app is inside SteamVR and its notice panel can be shown.
    pub fn connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// Sends a notification through SteamVR's own notification system and says how it went.
    pub fn try_native(&self) -> String {
        let (tx, rx) = channel();
        self.send(Cmd::TryNative(tx));
        rx.recv_timeout(Duration::from_secs(4)).unwrap_or_else(|_| "SteamVR did not answer in time.".into())
    }

    /// Where things stand, in words for the settings screen.
    pub fn status(&self) -> String {
        self.status.lock().unwrap().clone()
    }
}

/// SteamVR's folder, from the file SteamVR itself keeps for programs that look for it.
pub fn steamvr_folder() -> Option<PathBuf> {
    let file = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("openvr").join("openvrpaths.vrpath");
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
    json["runtime"].as_array()?.iter().filter_map(|p| p.as_str()).map(PathBuf::from).find(|p| p.is_dir())
}

/// Floats a notice in front of the eyes, an arm's length away. `above` puts it a little over
/// the middle, clear of XSOverlay's notices, which sit a little under it; otherwise it goes
/// a little under the middle.
pub fn in_view(above: bool) -> Matrix {
    [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, if above { 0.16 } else { -0.16 }], [0.0, 0.0, 1.0, -0.85]]
}

type Turn = [[f32; 3]; 3];

/// A wrist panel another program keeps on the arm: its turn and place relative to the
/// controller in SteamVR's terms, its width in metres, and the hand it is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OtherWrist {
    turn: Turn,
    at: [f32; 3],
    width: f32,
    hand: Hand,
}

/// XSOverlay's wrist panel is a strip about half as tall as it is wide. Its exact height is
/// not in its settings, so this is a little generous; the slider takes up any difference.
const XS_WRIST_ASPECT: f32 = 0.55;
/// The space left between XSOverlay's wrist panel and the feed.
const GAP: f32 = 0.015;

/// XSOverlay's wrist panel, from the settings file XSOverlay keeps for itself. Nothing when
/// XSOverlay is not installed or its file says something unexpected.
pub fn xsoverlay_wrist() -> Option<OtherWrist> {
    let file = PathBuf::from(std::env::var_os("USERPROFILE")?)
        .join("AppData")
        .join("LocalLow")
        .join("Xiexe")
        .join("XSOverlay")
        .join("config")
        .join("overlaySettings.json");
    xsoverlay_wrist_from(&std::fs::read_to_string(file).ok()?)
}

/// Unity's way of writing a turn as three angles in degrees: z first, then x, then y.
fn unity_turn([x, y, z]: [f32; 3]) -> Turn {
    let (sx, cx) = x.to_radians().sin_cos();
    let (sy, cy) = y.to_radians().sin_cos();
    let (sz, cz) = z.to_radians().sin_cos();
    let rx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
    let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
    let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
    times(ry, times(rx, rz))
}

fn times(a: Turn, b: Turn) -> Turn {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

/// Reads XSOverlay's settings. It is a Unity program, and Unity counts z the other way round
/// from SteamVR, so z is turned round on both sides of the turn and in the place.
fn xsoverlay_wrist_from(json: &str) -> Option<OtherWrist> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    // Hidden in XSOverlay's own settings: there is no panel to line up with or keep clear of.
    if v.get("HideWristOverlay").and_then(serde_json::Value::as_bool) == Some(true) {
        return None;
    }
    let three = |key: &str| -> Option<[f32; 3]> {
        let o = v.get(key)?;
        let n = |k: &str| o.get(k)?.as_f64().filter(|x| x.is_finite()).map(|x| x as f32);
        Some([n("x")?, n("y")?, n("z")?])
    };
    let place = three("WristOffsets")?;
    let unity = unity_turn(three("WristRotation")?);
    if place.iter().any(|p| p.abs() > 0.5) {
        return None;
    }
    let flip = |i: usize| if i == 2 { -1.0 } else { 1.0 };
    let turn = std::array::from_fn(|i| std::array::from_fn(|j| unity[i][j] * flip(i) * flip(j)));
    let width = v.get("WristScale").and_then(serde_json::Value::as_f64).map(|w| w as f32).filter(|w| (0.03..=0.6).contains(w)).unwrap_or(0.17);
    // XSOverlay keeps its wrist panel on the hand that is not the main one; 0 is right-handed.
    let hand = if v.get("DominantHand").and_then(serde_json::Value::as_i64) == Some(1) { Hand::Right } else { Hand::Left };
    Some(OtherWrist { turn, at: [place[0], place[1], -place[2]], width, hand })
}

/// The same place on the other hand: left and right swap, nothing else changes.
fn mirrored(turn: Turn, at: [f32; 3]) -> (Turn, [f32; 3]) {
    let sign = |i: usize| if i == 0 { -1.0 } else { 1.0 };
    (std::array::from_fn(|i| std::array::from_fn(|j| turn[i][j] * sign(i) * sign(j))), [-at[0], at[1], at[2]])
}

fn column(turn: &Turn, c: usize) -> [f32; 3] {
    [turn[0][c], turn[1][c], turn[2][c]]
}

/// Where the feed sits on the arm, relative to the controller in that hand.
///
/// Like XSOverlay's wrist panel, it lies on the back of the wrist, facing out of the back of
/// the hand, turned sideways with its top towards the elbow, and it turns with the wrist. When
/// XSOverlay's wrist panel is on the same hand, its own turn and place from its settings are
/// used, so the two line up: "up the arm" stacks the feed just above XSOverlay's panel, and
/// "back of the wrist" takes XSOverlay's place, just in front of its panel and covering it. The
/// inside of the wrist is the same panel turned round to face out of the palm. `shift` moves it
/// along the arm, more towards the elbow.
pub fn on_arm(hand: Hand, spot: Spot, height: f32, shift: f32, other: Option<&OtherWrist>) -> Matrix {
    let beside = other.filter(|o| o.hand == hand);
    let (turn, wrist, below) = match beside {
        Some(o) => (o.turn, o.at, o.width * XS_WRIST_ASPECT),
        None => {
            // On the left hand the back of the hand is towards minus x, and with the
            // controller held the arm runs back and a little down from it.
            let left: Turn = [[0.0, 0.0, -1.0], [-0.8, -0.6, 0.0], [-0.6, 0.8, 0.0]];
            let at = [-0.055, 0.01, 0.13];
            let (turn, at) = if hand == Hand::Left { (left, at) } else { mirrored(left, at) };
            (turn, at, 0.07)
        }
    };
    let up = column(&turn, 1);
    let (turn, from) = match spot {
        // Next to XSOverlay's panel this is its very place, so the feed sits half a centimetre in
        // front of it (and is drawn over it) rather than fighting it for the same spot.
        Spot::Wrist if beside.is_some() => {
            let face = column(&turn, 2);
            (turn, std::array::from_fn(|i| wrist[i] + face[i] * 0.005))
        }
        Spot::Wrist => (turn, wrist),
        Spot::Forearm => {
            let along = below / 2.0 + GAP + height / 2.0;
            (turn, std::array::from_fn(|i| wrist[i] + up[i] * along))
        }
        Spot::Inside => {
            // Half a turn about the arm: it faces out of the palm, still with its top to the elbow.
            let right = column(&turn, 0);
            let face = column(&turn, 2);
            let turned: Turn = std::array::from_fn(|i| [-right[i], up[i], -face[i]]);
            // Across the wrist from the back of the hand, a little closer in.
            let across: [f32; 3] = std::array::from_fn(|i| wrist[i] - face[i] * 0.09);
            (turned, across)
        }
    };
    let at: [f32; 3] = std::array::from_fn(|i| from[i] + up[i] * shift);
    std::array::from_fn(|i| [turn[i][0], turn[i][1], turn[i][2], at[i]])
}

/// A left click let go of, from one panel event, in pixels from the top left. SteamVR counts
/// from the bottom left (`VREvent_Mouse_t`: x and y as floats, then the button), so y is turned
/// round with the picture's height.
fn click(event: &VrEvent, height: f32) -> Option<(f32, f32)> {
    if event.kind != EVENT_MOUSE_UP {
        return None;
    }
    let first = event.data[0].to_le_bytes();
    let x = f32::from_le_bytes(first[0..4].try_into().ok()?);
    let y = f32::from_le_bytes(first[4..8].try_into().ok()?);
    let button = (event.data[1] & 0xffff_ffff) as u32;
    (button == MOUSE_LEFT && x.is_finite() && y.is_finite()).then_some((x, height - y))
}

/// The battery step to tell about now, if any: 20 at or under 20%, 10 at or under 10%. Each
/// is told once; charging, or climbing back over 25%, lets them be told again.
fn battery_step(level: f32, charging: bool, told: &mut u8) -> Option<u8> {
    if charging || level > 0.25 {
        *told = 0;
        return None;
    }
    let step = if level <= 0.10 {
        10
    } else if level <= 0.20 {
        20
    } else {
        return None;
    };
    if *told == 0 || step < *told {
        *told = step;
        Some(step)
    } else {
        None
    }
}

/// What SteamVR says about one device, and what has been told about it.
#[derive(Debug, Default, Clone)]
struct Watched {
    class: i32,
    role: i32,
    connected: bool,
    has_battery: bool,
    battery: Option<f32>,
    charging: bool,
    /// Had a good pose since it came on; a device that never had one is not "lost".
    seen_good: bool,
    /// How long it has gone without a good pose, counted only while the headset tracks, and the
    /// last look that was counted.
    bad_for: Option<Duration>,
    last_counted: Option<Instant>,
    good_since: Option<Instant>,
    lost: bool,
    battery_told: u8,
    off_told: bool,
    /// When its lost tracking was last told, so one that keeps dropping is not told every time.
    lost_told_at: Option<Instant>,
    /// Lost and not told yet. It stays until it is told, even when the device is back by then,
    /// so a loss that has to wait (the headset out, the gap between notices, or the minute since
    /// the last one) is told later instead of never.
    untold: bool,
    /// When its switching off was last told.
    off_told_at: Option<Instant>,
    /// Switched off and not told yet, with its short and long names as they were at that moment.
    /// Like `untold`, it waits for the gap between tracking notices and the minute since the last
    /// one; it is dropped when the device comes back. Whether it is worth telling and what it is
    /// called are settled when it switches off, because SteamVR can take a switched-off
    /// controller's hand away before the notice goes out.
    off_untold: Option<(String, String)>,
}

impl Watched {
    /// One look at the pose. Returns true the moment tracking counts as lost. `counts` is false
    /// while the headset itself is not tracked: that time is not added, but what was added before
    /// stays, so a headset that blinks out now and then does not hide a real loss.
    fn pose(&mut self, good: bool, now: Instant, counts: bool) -> bool {
        if good {
            self.bad_for = None;
            self.last_counted = None;
            self.seen_good = true;
            let since = *self.good_since.get_or_insert(now);
            if self.lost && now.duration_since(since) >= BACK_AFTER {
                self.lost = false;
            }
            return false;
        }
        self.good_since = None;
        if !counts {
            self.last_counted = None;
            return false;
        }
        if !self.seen_good || self.lost {
            return false;
        }
        let added = self.last_counted.map_or(Duration::ZERO, |last| now.duration_since(last));
        let bad = self.bad_for.unwrap_or_default() + added;
        self.bad_for = Some(bad);
        self.last_counted = Some(now);
        if bad >= LOST_AFTER {
            self.lost = true;
            return true;
        }
        false
    }

    fn reset(&mut self) {
        self.bad_for = None;
        self.last_counted = None;
        self.good_since = None;
        self.lost = false;
        self.untold = false;
    }
}

/// Every device worth showing, by index, and the order trackers were first seen in.
#[derive(Default)]
struct Devices {
    by_index: BTreeMap<u32, Watched>,
    trackers: Vec<u32>,
    /// When the first of the losses waiting to be told was seen, with the headset tracking.
    lost_since: Option<Instant>,
    /// When the last tracking notice of any kind went out.
    told_any_at: Option<Instant>,
}

impl Devices {
    /// A device with a name worth telling about. A controller SteamVR gives no hand is left out:
    /// with some headsets those are stand-ins (hand tracking, for one) that come and go and
    /// never track while the real controllers are in use.
    fn tellable(&self, index: u32) -> bool {
        self.by_index.get(&index).is_some_and(|w| match w.class {
            CLASS_HMD => false,
            CLASS_CONTROLLER => matches!(w.role, ROLE_LEFT | ROLE_RIGHT),
            _ => true,
        })
    }

    /// The losses that may be told now: lost, not told yet, worth a name, and not told about
    /// within the last minute.
    fn losses_to_tell(&self, now: Instant) -> Vec<u32> {
        self.by_index
            .iter()
            .filter(|(&i, w)| w.untold && self.tellable(i) && w.lost_told_at.is_none_or(|at| now.duration_since(at) >= LOSS_AGAIN_AFTER))
            .map(|(&i, _)| i)
            .collect()
    }

    /// True once the gap since the last tracking notice has passed.
    fn gap_passed(&self, now: Instant) -> bool {
        self.told_any_at.is_none_or(|at| now.duration_since(at) >= NOTICE_GAP)
    }

    /// SteamVR says a device switched off. A device worth a name that had tracked waits to be
    /// told by `off_notice`, under the name it has now; a second word about the same switch-off
    /// adds nothing.
    fn switched_off(&mut self, index: u32) {
        let tell = self.tellable(index) && self.by_index.get(&index).is_some_and(|w| w.seen_good && !w.off_told);
        let names = tell.then(|| self.names(index));
        if let Some(w) = self.by_index.get_mut(&index) {
            w.connected = false;
            w.reset();
            w.off_told = true;
            if names.is_some() {
                w.off_untold = names;
            }
        }
    }

    /// SteamVR says a device switched on again. A switch-off still waiting is not told now.
    fn switched_on(&mut self, index: u32) {
        if let Some(w) = self.by_index.get_mut(&index) {
            w.off_told = false;
            w.seen_good = false;
            w.off_untold = None;
        }
    }

    /// One notice for the devices that switched off and wait to be told, a minute at least since
    /// each was last told, and never sooner than the gap after the last tracking notice; what
    /// switches off meanwhile joins this one. With `tell` false (the alerts off, or SteamVR
    /// closing) those waiting are dropped instead. It does not ask again whether a device is
    /// worth telling: that was settled when it switched off, and its role may read as no hand by
    /// now.
    fn off_notice(&mut self, now: Instant, tell: bool) -> Option<News> {
        if !tell {
            for w in self.by_index.values_mut() {
                w.off_untold = None;
            }
            return None;
        }
        let ready: Vec<u32> = self
            .by_index
            .iter()
            .filter(|(_, w)| w.off_untold.is_some() && w.off_told_at.is_none_or(|at| now.duration_since(at) >= LOSS_AGAIN_AFTER))
            .map(|(&i, _)| i)
            .collect();
        if ready.is_empty() || !self.gap_passed(now) {
            return None;
        }
        let mut told: Vec<(String, String)> = Vec::new();
        for i in &ready {
            if let Some(w) = self.by_index.get_mut(i) {
                w.off_told_at = Some(now);
                told.extend(w.off_untold.take());
            }
        }
        let notice = match told.as_slice() {
            [] => return None,
            [(_, long)] => alert("tracking", "Switched off", format!("{long} switched off"), ""),
            [first @ .., (last, _)] => {
                let list = format!("{} and {last}.", first.iter().map(|(short, _)| short.as_str()).collect::<Vec<_>>().join(", "));
                alert("tracking", "Switched off", format!("{} devices switched off", told.len()), &list)
            }
        };
        self.told_any_at = Some(now);
        Some(notice)
    }

    /// One notice for the devices that lost tracking close together, once the first has waited
    /// `LOSS_BATCH` with the headset tracking. Those told are marked as told.
    fn lost_notice(&mut self, now: Instant) -> Option<News> {
        let ready = self.losses_to_tell(now);
        if ready.is_empty() {
            self.lost_since = None;
            return None;
        }
        let since = *self.lost_since.get_or_insert(now);
        // Losses close together go out as one, and never sooner than the gap after the last
        // tracking notice; what comes in meanwhile joins this one.
        if now.duration_since(since) < LOSS_BATCH || !self.gap_passed(now) {
            return None;
        }
        self.lost_since = None;
        self.told_any_at = Some(now);
        for i in &ready {
            if let Some(w) = self.by_index.get_mut(i) {
                w.lost_told_at = Some(now);
                w.untold = false;
            }
        }
        match ready.as_slice() {
            [] => None,
            [one] => Some(alert(
                "tracking",
                "Tracking lost",
                format!("{} lost tracking", self.names(*one).1),
                "Not seen for 3 seconds. Check the base stations or cameras.",
            )),
            [first @ .., last] => {
                let short = |i: &u32| self.names(*i).0;
                let list = format!("{} and {}", first.iter().map(short).collect::<Vec<_>>().join(", "), short(last));
                Some(alert("tracking", "Tracking lost", format!("{} devices lost tracking", ready.len()), &format!("{list}. Check the base stations or cameras.")))
            }
        }
    }

    /// Short and long names: "Left" and "Left controller", "Tracker 2" twice, "Headset".
    fn names(&self, index: u32) -> (String, String) {
        let Some(w) = self.by_index.get(&index) else {
            return ("Device".into(), "A device".into());
        };
        match w.class {
            CLASS_HMD => ("Headset".into(), "Headset".into()),
            CLASS_CONTROLLER => match w.role {
                ROLE_LEFT => ("Left".into(), "Left controller".into()),
                ROLE_RIGHT => ("Right".into(), "Right controller".into()),
                _ => ("Controller".into(), "A controller".into()),
            },
            _ => {
                let n = self.trackers.iter().position(|&i| i == index).map_or(0, |p| p + 1);
                let name = if n == 0 { "Tracker".to_string() } else { format!("Tracker {n}") };
                (name.clone(), name)
            }
        }
    }

    /// The list the wrist and the dashboard show: connected devices, controllers first.
    fn list(&self) -> Vec<DeviceState> {
        let mut shown: Vec<(u8, u32, DeviceState)> = self
            .by_index
            .iter()
            // The headset only when it has a battery; a controller without a hand only when it
            // has one too, so stand-ins without one stay off the list.
            .filter(|(_, w)| w.connected && (w.has_battery || !(w.class == CLASS_HMD || (w.class == CLASS_CONTROLLER && !matches!(w.role, ROLE_LEFT | ROLE_RIGHT)))))
            .map(|(&i, w)| {
                let order = match (w.class, w.role) {
                    (CLASS_CONTROLLER, ROLE_LEFT) => 0,
                    (CLASS_CONTROLLER, ROLE_RIGHT) => 1,
                    (CLASS_CONTROLLER, _) => 2,
                    (CLASS_TRACKER, _) => 3,
                    _ => 4,
                };
                let state = DeviceState {
                    name: self.names(i).0,
                    battery: if w.has_battery { w.battery.map(|b| (b.clamp(0.0, 1.0) * 100.0).round() as u8) } else { None },
                    charging: w.charging,
                    lost: w.lost,
                };
                (order, i, state)
            })
            .collect();
        shown.sort_by_key(|(order, i, _)| (*order, *i));
        shown.into_iter().map(|(_, _, s)| s).collect()
    }
}

/// One stay in SteamVR, from signing in to letting go.
struct Session {
    // Kept so the DLL stays loaded for as long as the tables below are used.
    _lib: libloading::Library,
    shutdown: ShutdownFn,
    overlay: *const *const c_void,
    system: *const *const c_void,
    /// SteamVR's own notifications; missing on a SteamVR that does not offer them.
    notifications: Option<*const *const c_void>,
    notice: Handle,
    wrist: Handle,
    dash: Handle,
    dash_thumb: Handle,
}

impl Session {
    unsafe fn open(folder: &Path) -> Result<Self, String> {
        let dll = folder.join("bin").join("win64").join("openvr_api.dll");
        let lib = libloading::Library::new(&dll).map_err(|_| "SteamVR's openvr_api.dll could not be loaded.".to_string())?;
        let init: InitFn = *lib.get(b"VR_InitInternal\0").map_err(|e| e.to_string())?;
        let shutdown: ShutdownFn = *lib.get(b"VR_ShutdownInternal\0").map_err(|e| e.to_string())?;
        let interface: InterfaceFn = *lib.get(b"VR_GetGenericInterface\0").map_err(|e| e.to_string())?;
        let error_name: ErrorNameFn = *lib.get(b"VR_GetVRInitErrorAsEnglishDescription\0").map_err(|e| e.to_string())?;
        let words = |code: i32| {
            let text = error_name(code);
            if text.is_null() {
                format!("SteamVR said no (error {code}).")
            } else {
                format!("SteamVR said: {}", CStr::from_ptr(text).to_string_lossy())
            }
        };

        let mut error = 0;
        init(&mut error, APP_OVERLAY);
        if error != 0 {
            return Err(words(error));
        }
        let table = |name: &str| -> Result<*const *const c_void, String> {
            let name = CString::new(name).unwrap();
            let mut error = 0;
            let found = interface(name.as_ptr(), &mut error);
            if error != 0 || found == 0 {
                Err("This SteamVR is too old for the app's panels. Update SteamVR.".to_string())
            } else {
                Ok(found as *const *const c_void)
            }
        };
        let tables = table(OVERLAY_TABLE).and_then(|o| table(SYSTEM_TABLE).map(|s| (o, s)));
        let (overlay, system) = match tables {
            Ok(found) => found,
            Err(why) => {
                shutdown();
                return Err(why);
            }
        };
        let notifications = table(NOTIFICATIONS_TABLE).ok();
        let mut session = Self { _lib: lib, shutdown, overlay, system, notifications, notice: 0, wrist: 0, dash: 0, dash_thumb: 0 };
        session.notice = session.create("anonymous-vrx.notice", "Anonymous VRX notice");
        session.wrist = session.create("anonymous-vrx.wrist", "Anonymous VRX wrist feed");
        if session.notice == 0 || session.wrist == 0 {
            // Dropping the session lets go of SteamVR again.
            return Err("SteamVR did not make the panels. Another copy of the app may be holding them.".to_string());
        }
        // Notices sit above other panels, the wrist feed among them, and just in front of a
        // wrist panel of another program's when it takes the same place.
        session.call_u32(SET_SORT_ORDER, session.notice, 50);
        session.call_u32(SET_SORT_ORDER, session.wrist, 1);
        // Both stay in sight while the dashboard is open, so a setting changed in the tab, or
        // its test notice, shows at once.
        let flag: unsafe extern "system" fn(Handle, i32, bool) -> i32 = session.slot(session.overlay, SET_FLAG);
        flag(session.notice, VISIBLE_IN_DASHBOARD, true);
        flag(session.wrist, VISIBLE_IN_DASHBOARD, true);
        Ok(session)
    }

    unsafe fn slot<T: Copy>(&self, table: *const *const c_void, place: usize) -> T {
        std::mem::transmute_copy(&*table.add(place))
    }

    unsafe fn create(&self, key: &str, name: &str) -> Handle {
        let f: unsafe extern "system" fn(*const c_char, *const c_char, *mut Handle) -> i32 = self.slot(self.overlay, CREATE_OVERLAY);
        let (key, name) = (CString::new(key).unwrap(), CString::new(name).unwrap());
        let mut handle: Handle = 0;
        if f(key.as_ptr(), name.as_ptr(), &mut handle) == 0 {
            handle
        } else {
            0
        }
    }

    /// The tab in SteamVR's dashboard: a main panel and the small icon on its button.
    unsafe fn create_dashboard(&mut self) -> bool {
        let f: unsafe extern "system" fn(*const c_char, *const c_char, *mut Handle, *mut Handle) -> i32 = self.slot(self.overlay, CREATE_DASHBOARD);
        let (key, name) = (CString::new("anonymous-vrx.dashboard").unwrap(), CString::new("Anonymous VRX").unwrap());
        let (mut main, mut thumb): (Handle, Handle) = (0, 0);
        if f(key.as_ptr(), name.as_ptr(), &mut main, &mut thumb) != 0 || main == 0 {
            return false;
        }
        self.dash = main;
        self.dash_thumb = thumb;
        self.call_f32(SET_WIDTH, main, 1.5);
        // Dashboard tabs take SteamVR's pointer as a mouse; said once more to be sure.
        let input: unsafe extern "system" fn(Handle, i32) -> i32 = self.slot(self.overlay, SET_INPUT_METHOD);
        input(main, INPUT_MOUSE);
        true
    }

    /// How big the panel is in the units its mouse events should come in: here, its pixels.
    unsafe fn mouse_scale(&self, handle: Handle, width: f32, height: f32) {
        let f: unsafe extern "system" fn(Handle, *const [f32; 2]) -> i32 = self.slot(self.overlay, SET_MOUSE_SCALE);
        f(handle, &[width, height]);
    }

    /// Every event SteamVR has waiting for one panel.
    unsafe fn overlay_events(&self, handle: Handle) -> Vec<VrEvent> {
        let poll: unsafe extern "system" fn(Handle, *mut VrEvent, u32) -> bool = self.slot(self.overlay, POLL_OVERLAY_EVENT);
        let mut out = Vec::new();
        let mut event: VrEvent = std::mem::zeroed();
        while out.len() < 64 && poll(handle, &mut event, std::mem::size_of::<VrEvent>() as u32) {
            out.push(event);
        }
        out
    }

    unsafe fn drop_dashboard(&mut self) {
        for handle in [self.dash, self.dash_thumb] {
            if handle != 0 {
                self.call(DESTROY_OVERLAY, handle);
            }
        }
        self.dash = 0;
        self.dash_thumb = 0;
    }

    unsafe fn call(&self, place: usize, handle: Handle) {
        let f: unsafe extern "system" fn(Handle) -> i32 = self.slot(self.overlay, place);
        f(handle);
    }

    unsafe fn call_f32(&self, place: usize, handle: Handle, value: f32) {
        let f: unsafe extern "system" fn(Handle, f32) -> i32 = self.slot(self.overlay, place);
        f(handle, value);
    }

    unsafe fn call_u32(&self, place: usize, handle: Handle, value: u32) {
        let f: unsafe extern "system" fn(Handle, u32) -> i32 = self.slot(self.overlay, place);
        f(handle, value);
    }

    unsafe fn picture(&self, handle: Handle, file: &Path) -> bool {
        let Ok(path) = CString::new(file.to_string_lossy().as_bytes()) else {
            return false;
        };
        let f: unsafe extern "system" fn(Handle, *const c_char) -> i32 = self.slot(self.overlay, SET_FROM_FILE);
        f(handle, path.as_ptr()) == 0
    }

    unsafe fn place(&self, handle: Handle, device: u32, at: &Matrix) {
        let f: unsafe extern "system" fn(Handle, u32, *const Matrix) -> i32 = self.slot(self.overlay, SET_TRANSFORM_DEVICE);
        f(handle, device, at);
    }

    /// The tracked device a hand holds, or `NO_DEVICE` while that controller is off.
    unsafe fn controller(&self, hand: Hand) -> u32 {
        let f: unsafe extern "system" fn(i32) -> u32 = self.slot(self.system, CONTROLLER_FOR_ROLE);
        f(if hand == Hand::Left { ROLE_LEFT } else { ROLE_RIGHT })
    }

    unsafe fn class(&self, index: u32) -> i32 {
        let f: unsafe extern "system" fn(u32) -> i32 = self.slot(self.system, DEVICE_CLASS);
        f(index)
    }

    unsafe fn role(&self, index: u32) -> i32 {
        let f: unsafe extern "system" fn(u32) -> i32 = self.slot(self.system, ROLE_FOR_DEVICE);
        f(index)
    }

    unsafe fn flag(&self, index: u32, prop: i32) -> Option<bool> {
        let f: unsafe extern "system" fn(u32, i32, *mut i32) -> bool = self.slot(self.system, BOOL_PROPERTY);
        let mut error = 0;
        let value = f(index, prop, &mut error);
        (error == 0).then_some(value)
    }

    unsafe fn number(&self, index: u32, prop: i32) -> Option<f32> {
        let f: unsafe extern "system" fn(u32, i32, *mut i32) -> f32 = self.slot(self.system, FLOAT_PROPERTY);
        let mut error = 0;
        let value = f(index, prop, &mut error);
        (error == 0 && value.is_finite()).then_some(value)
    }

    unsafe fn poses(&self) -> [Pose; MAX_DEVICES] {
        let f: unsafe extern "system" fn(i32, f32, *mut Pose, u32) = self.slot(self.system, TRACKING_POSE);
        let mut poses: [Pose; MAX_DEVICES] = std::mem::zeroed();
        f(STANDING, 0.0, poses.as_mut_ptr(), MAX_DEVICES as u32);
        poses
    }

    /// Every event SteamVR has waiting, as (event, device).
    unsafe fn events(&self) -> Vec<(u32, u32)> {
        let poll: unsafe extern "system" fn(*mut VrEvent, u32) -> bool = self.slot(self.system, POLL_EVENT);
        let mut out = Vec::new();
        let mut event: VrEvent = std::mem::zeroed();
        // A cap, so a flood of events cannot hold the panels up.
        while out.len() < 256 && poll(&mut event, std::mem::size_of::<VrEvent>() as u32) {
            out.push((event.kind, event.device));
        }
        out
    }

    unsafe fn acknowledge_quit(&self) {
        let acknowledge: unsafe extern "system" fn() = self.slot(self.system, ACKNOWLEDGE_QUIT);
        acknowledge();
    }

    /// One of SteamVR's own notifications, sent through the notice panel. The answer is
    /// SteamVR's own code, in words.
    unsafe fn native_notification(&self, text: &str) -> String {
        let Some(table) = self.notifications else {
            return "This SteamVR does not offer its own notifications to other programs (no IVRNotifications_002).".into();
        };
        let f: unsafe extern "system" fn(Handle, u64, i32, *const c_char, i32, *const c_void, *mut u32) -> i32 = self.slot(table, CREATE_NOTIFICATION);
        let text = CString::new(text).unwrap_or_default();
        let mut id = 0u32;
        // Transient, in the application's style, no picture.
        let code = f(self.notice, 1, 0, text.as_ptr(), 100, std::ptr::null(), &mut id);
        match code {
            0 => format!("SteamVR took it (code 0, notification {id}). Look in the headset: if nothing showed, SteamVR does not show these any more."),
            101 => "SteamVR said its notification queue is full (code 101).".into(),
            102 => "SteamVR did not accept the panel the notification hangs on (code 102).".into(),
            104 => "SteamVR said its notification service is not available (code 104).".into(),
            other => format!("SteamVR refused it with code {other}."),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            self.drop_dashboard();
            for handle in [self.notice, self.wrist] {
                if handle != 0 {
                    self.call(DESTROY_OVERLAY, handle);
                }
            }
            (self.shutdown)();
        }
    }
}

fn say(status: &Mutex<String>, text: &str) {
    let mut held = status.lock().unwrap();
    if *held != text {
        *held = text.to_string();
    }
}

/// What has to be said after one look at the devices.
fn alert(kind: &'static str, label: &'static str, title: String, body: &str) -> News {
    News::Alert { kind, label, title, body: body.to_string() }
}

/// Reads every device's class, role and battery, and says what crossed a battery step.
unsafe fn read_devices(s: &Session, devices: &mut Devices, out: &mut Vec<News>, tell: bool) {
    for index in 0..MAX_DEVICES as u32 {
        let class = s.class(index);
        if !matches!(class, CLASS_HMD | CLASS_CONTROLLER | CLASS_TRACKER) {
            if let Some(w) = devices.by_index.get_mut(&index) {
                w.connected = false;
            }
            continue;
        }
        if class == CLASS_TRACKER && !devices.trackers.contains(&index) {
            devices.trackers.push(index);
            devices.trackers.sort_unstable();
        }
        let has_battery = s.flag(index, PROP_HAS_BATTERY).unwrap_or(false);
        let battery = if has_battery { s.number(index, PROP_BATTERY) } else { None };
        let charging = s.flag(index, PROP_CHARGING).unwrap_or(false);
        let role = if class == CLASS_CONTROLLER { s.role(index) } else { 0 };
        let w = devices.by_index.entry(index).or_default();
        w.class = class;
        w.role = role;
        w.has_battery = has_battery;
        w.battery = battery;
        w.charging = charging;
        let step = battery.and_then(|level| battery_step(level, charging, &mut w.battery_told));
        if let (Some(step), true) = (step, tell) {
            let percent = (battery.unwrap_or(0.0) * 100.0).round() as u8;
            let name = devices.names(index).1;
            let body = if step == 10 { "It will switch off soon." } else { "Charge it soon." };
            out.push(alert("battery", "Battery low", format!("{name} at {percent}%"), body));
        }
    }
}

/// One look at every pose: connected or not, and tracking lost for longer than three seconds.
unsafe fn read_poses(s: &Session, devices: &mut Devices, out: &mut Vec<News>, tell: bool) {
    let poses = s.poses();
    look_at_poses(&poses, devices, out, tell, Instant::now());
}

/// The part of `read_poses` that does not talk to SteamVR, so it can be tried without it.
fn look_at_poses(poses: &[Pose], devices: &mut Devices, out: &mut Vec<News>, tell: bool, now: Instant) {
    // When the headset itself is not tracked, everything drops with it: a headset that tracks
    // its own controllers loses them, and one that went to sleep stops them all. That is no
    // news about any one device, so nothing is told then.
    let head = poses.get(HMD as usize).copied();
    let head_ok = head.is_some_and(|h| h.connected && h.valid && h.result == TRACKING_OK);
    for (&index, w) in devices.by_index.iter_mut() {
        let Some(pose) = poses.get(index as usize) else { continue };
        w.connected = pose.connected;
        if !pose.connected {
            w.reset();
            continue;
        }
        let good = pose.valid && pose.result == TRACKING_OK;
        if w.pose(good, now, head_ok) {
            w.untold = true;
        }
    }
    if !tell {
        // Losses from while the alerts were off are not told once they come on.
        for w in devices.by_index.values_mut() {
            w.untold = false;
        }
        devices.lost_since = None;
        return;
    }
    if !head_ok {
        // Held, not dropped, while the headset is out; told once it tracks again.
        return;
    }
    if let Some(notice) = devices.lost_notice(now) {
        out.push(notice);
    }
}

fn run(rx: Receiver<Cmd>, status: Arc<Mutex<String>>, connected: Arc<AtomicBool>, steamvr_running: fn() -> bool, news: NewsFn) {
    let mut wanted = false;
    let mut watch = false;
    let mut session: Option<Session> = None;
    let mut next_look = Instant::now();
    // What the wrist should show, kept so it is put back after SteamVR restarts.
    let mut feed: Option<WristPanel> = None;
    let mut feed_shown = false;
    let mut feed_device = NO_DEVICE;
    let mut feed_changed = false;
    // XSOverlay's wrist panel, so the feed can line up with it, and when to look again.
    let mut other: Option<OtherWrist> = None;
    let mut next_other = Instant::now();
    let mut notice_until: Option<Instant> = None;
    // The dashboard tab: wanted or not, and whether its pictures changed since they were handed over.
    let mut dash: Option<(PathBuf, f32, Option<PathBuf>)> = None;
    let mut dash_changed = false;
    let mut thumb_given = false;
    // Not before this: right after SteamVR starts, its dashboard is still starting too, and a tab
    // made in that moment came out broken until it was made again.
    let mut dash_after = Instant::now();
    // The dashboard picture's height in pixels, to turn SteamVR's bottom-up clicks round.
    let mut dash_height = 0.0;
    // The devices, and when to look at them next.
    let mut devices = Devices::default();
    let mut shown_devices: Option<Vec<DeviceState>> = None;
    let mut next_poses = Instant::now();
    let mut next_props = Instant::now();
    let mut closing = false;

    // Letting go of SteamVR also empties the device list the window shows.
    let drop_session = |session: &mut Option<Session>, connected: &AtomicBool, shown: &mut Option<Vec<DeviceState>>| {
        *session = None;
        connected.store(false, Ordering::Relaxed);
        if shown.take().is_some_and(|list| !list.is_empty()) {
            news(News::Devices(Vec::new()));
        }
    };

    loop {
        match rx.recv_timeout(Duration::from_millis(if session.is_some() { 150 } else { 1000 })) {
            Ok(Cmd::Wanted(on)) => {
                wanted = on;
                // Never sooner than a wait already set: right after SteamVR said it is closing,
                // asking again would start it back up.
                if next_look < Instant::now() {
                    next_look = Instant::now();
                }
                if !on {
                    drop_session(&mut session, &connected, &mut shown_devices);
                    feed_shown = false;
                    notice_until = None;
                    thumb_given = false;
                    say(&status, "Switched off.");
                }
            }
            Ok(Cmd::Watch(on)) => watch = on,
            Ok(Cmd::Notice { picture, seconds, alpha, above }) => {
                if let Some(s) = &session {
                    unsafe {
                        if s.picture(s.notice, &picture) {
                            s.call_f32(SET_WIDTH, s.notice, 0.46);
                            s.call_f32(SET_ALPHA, s.notice, alpha.clamp(0.2, 1.0));
                            s.place(s.notice, HMD, &in_view(above));
                            s.call(SHOW_OVERLAY, s.notice);
                            notice_until = Some(Instant::now() + Duration::from_secs_f32(seconds.clamp(1.0, 30.0)));
                        }
                    }
                }
            }
            Ok(Cmd::Wrist(new)) => {
                feed = new;
                feed_changed = true;
            }
            Ok(Cmd::Dashboard(new)) => {
                if new.as_ref().is_some_and(|(_, _, thumb)| thumb.is_some()) {
                    thumb_given = false;
                }
                // A picture without a new icon keeps the icon already handed over.
                dash = match (new, dash.take()) {
                    (Some((main, aspect, None)), Some((_, _, old_thumb))) => Some((main, aspect, old_thumb)),
                    (new, _) => new,
                };
                dash_changed = true;
            }
            Ok(Cmd::TryNative(answer)) => {
                let text = match &session {
                    Some(s) => unsafe { s.native_notification("Anonymous VRX: this is one of SteamVR's own notifications.") },
                    None => "The app is not in SteamVR right now. Switch on a panel and start SteamVR first.".to_string(),
                };
                let _ = answer.send(text);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }

        if !wanted {
            continue;
        }

        if session.is_none() {
            if Instant::now() < next_look {
                continue;
            }
            next_look = Instant::now() + Duration::from_secs(10);
            if !steamvr_running() {
                say(&status, "Waiting for SteamVR to start.");
                continue;
            }
            let Some(folder) = steamvr_folder() else {
                say(&status, "SteamVR was not found on this PC.");
                continue;
            };
            match unsafe { Session::open(&folder) } {
                Ok(s) => {
                    session = Some(s);
                    connected.store(true, Ordering::Relaxed);
                    feed_shown = false;
                    feed_device = NO_DEVICE;
                    feed_changed = true;
                    dash_changed = true;
                    thumb_given = false;
                    dash_after = Instant::now() + DASHBOARD_SETTLE;
                    next_other = Instant::now();
                    devices = Devices::default();
                    shown_devices = None;
                    closing = false;
                    next_poses = Instant::now();
                    next_props = Instant::now();
                    say(&status, "Connected to SteamVR.");
                }
                Err(why) => say(&status, &why),
            }
            continue;
        }

        // SteamVR can also die without a word; then there is nobody left to talk to.
        if Instant::now() >= next_look {
            next_look = Instant::now() + Duration::from_secs(15);
            if !steamvr_running() {
                drop_session(&mut session, &connected, &mut shown_devices);
                notice_until = None;
                feed_shown = false;
                say(&status, "SteamVR closed. Waiting for it to start again.");
                continue;
            }
        }

        let mut out: Vec<News> = Vec::new();
        let mut quit = false;
        {
            let s = session.as_mut().unwrap();
            unsafe {
                let mut refresh = false;
                for (event, device) in s.events() {
                    match event {
                        EVENT_QUIT | EVENT_DRIVER_QUIT => {
                            if !closing {
                                closing = true;
                                if watch {
                                    out.push(alert("steamvr", "SteamVR", "SteamVR is closing".into(), "Anonymous VRX stays open on your desktop."));
                                }
                            }
                            // Only the quit itself is answered; a driver's request comes before it.
                            if event == EVENT_QUIT {
                                s.acknowledge_quit();
                                quit = true;
                            }
                        }
                        EVENT_DEACTIVATED | EVENT_WIRELESS_DISCONNECT => {
                            devices.switched_off(device);
                            refresh = true;
                        }
                        EVENT_ACTIVATED | EVENT_WIRELESS_RECONNECT => {
                            devices.switched_on(device);
                            refresh = true;
                        }
                        EVENT_PROPERTY_CHANGED => refresh = true,
                        _ => {}
                    }
                }
                // The devices that switched off go out together, keeping the gap between tracking
                // notices like lost tracking does.
                if let Some(notice) = devices.off_notice(Instant::now(), watch && !closing) {
                    out.push(notice);
                }
                if !quit {
                    let now = Instant::now();
                    // Battery and roles every five seconds, or half a second after SteamVR says
                    // something changed, so a burst of changes makes one read.
                    if refresh {
                        next_props = next_props.min(now + Duration::from_millis(500));
                    }
                    if now >= next_props {
                        read_devices(s, &mut devices, &mut out, watch && !closing);
                        next_props = now + Duration::from_secs(5);
                    }
                    if now >= next_poses {
                        read_poses(s, &mut devices, &mut out, watch && !closing);
                        next_poses = now + Duration::from_millis(500);
                    }
                    let list = devices.list();
                    if shown_devices.as_ref() != Some(&list) {
                        shown_devices = Some(list.clone());
                        out.push(News::Devices(list));
                    }
                }
            }
        }
        for item in out {
            news(item);
        }
        if quit {
            drop_session(&mut session, &connected, &mut shown_devices);
            notice_until = None;
            feed_shown = false;
            // SteamVR takes a moment to go; looking again at once would start it up again.
            next_look = Instant::now() + Duration::from_secs(20);
            say(&status, "SteamVR closed. Waiting for it to start again.");
            continue;
        }

        let s = session.as_mut().unwrap();
        unsafe {
            if notice_until.is_some_and(|t| Instant::now() >= t) {
                s.call(HIDE_OVERLAY, s.notice);
                notice_until = None;
            }
            match &feed {
                None => {
                    if feed_shown {
                        s.call(HIDE_OVERLAY, s.wrist);
                        feed_shown = false;
                    }
                    feed_changed = false;
                    // No feed, so nothing lines up with XSOverlay any more.
                    say(&status, "Connected to SteamVR.");
                }
                Some(panel) => {
                    // A controller can be switched on late, or swap hands.
                    let device = s.controller(panel.hand);
                    if device == NO_DEVICE {
                        if feed_shown {
                            s.call(HIDE_OVERLAY, s.wrist);
                            feed_shown = false;
                        }
                        feed_device = NO_DEVICE;
                        say(&status, "Connected to SteamVR. The arm feed waits for that controller to switch on.");
                    } else {
                        let width = panel.width.clamp(0.08, 0.4);
                        if feed_changed {
                            s.picture(s.wrist, &panel.picture);
                            s.call_f32(SET_WIDTH, s.wrist, width);
                            s.call_f32(SET_ALPHA, s.wrist, panel.alpha.clamp(0.2, 1.0));
                        }
                        // XSOverlay's wrist panel can be moved while both run; its settings file
                        // is read again every few seconds and the feed follows.
                        let mut moved = false;
                        if Instant::now() >= next_other {
                            next_other = Instant::now() + Duration::from_secs(5);
                            let now = xsoverlay_wrist();
                            if now != other {
                                other = now;
                                moved = true;
                            }
                        }
                        if feed_changed || moved || device != feed_device {
                            let height = width * panel.aspect.clamp(0.2, 3.0);
                            let beside = other.as_ref().filter(|o| o.hand == panel.hand);
                            s.place(s.wrist, device, &on_arm(panel.hand, panel.spot, height, panel.shift.clamp(-0.1, 0.2), beside));
                            feed_device = device;
                            say(
                                &status,
                                if beside.is_some() { "Connected to SteamVR. The arm feed lines up with XSOverlay's wrist panel." } else { "Connected to SteamVR." },
                            );
                        }
                        if !feed_shown {
                            s.call(SHOW_OVERLAY, s.wrist);
                            feed_shown = true;
                        }
                        // Only now: a picture that arrived while the controller was off still
                        // has to be handed over once it is on.
                        feed_changed = false;
                    }
                }
            }
            if dash_changed && Instant::now() >= dash_after {
                dash_changed = false;
                match &dash {
                    None => s.drop_dashboard(),
                    Some((main, aspect, thumb)) => {
                        if s.dash != 0 || s.create_dashboard() {
                            // The icon first: SteamVR asks for it the moment the tab exists.
                            if let (Some(thumb), false) = (thumb, thumb_given) {
                                thumb_given = s.dash_thumb != 0 && s.picture(s.dash_thumb, thumb);
                            }
                            s.picture(s.dash, main);
                            // Clicks come back in the picture's own pixels.
                            dash_height = PICTURE_WIDTH * aspect.clamp(0.2, 3.0);
                            s.mouse_scale(s.dash, PICTURE_WIDTH, dash_height);
                        } else {
                            // SteamVR said no; it is asked again in a few seconds.
                            dash_changed = true;
                            dash_after = Instant::now() + Duration::from_secs(5);
                        }
                    }
                }
            }
            if s.dash != 0 && dash_height > 0.0 {
                for event in s.overlay_events(s.dash) {
                    if let Some((x, y)) = click(&event, dash_height) {
                        news(News::DashClick { x, y });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet() -> NewsFn {
        Arc::new(|_| {})
    }

    /// XSOverlay's own settings as they were on my PC on 2026-11-27, trimmed to what is read.
    const XS_SETTINGS: &str = r#"{ "WristOffsets": { "x": -0.057381197810173038, "y": 0.010668456554412842, "z": -0.1349358707666397 },
        "WristRotation": { "x": 347.8140869140625, "y": 100.28219604492188, "z": 231.43423461914063 },
        "WristScale": 0.17000000178813935, "DominantHand": 0 }"#;

    fn col(m: &Matrix, c: usize) -> [f32; 3] {
        [m[0][c], m[1][c], m[2][c]]
    }

    fn close(a: [f32; 3], b: [f32; 3], by: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= by)
    }

    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }

    #[test]
    fn every_place_on_the_arm_is_a_proper_turn() {
        // The three columns are at right angles, one long, and x cross y gives z, so nothing
        // comes out mirrored, with and without XSOverlay's panel to line up with.
        let xs = xsoverlay_wrist_from(XS_SETTINGS).unwrap();
        for other in [None, Some(&xs)] {
            for hand in [Hand::Left, Hand::Right] {
                for spot in [Spot::Wrist, Spot::Forearm, Spot::Inside] {
                    let m = on_arm(hand, spot, 0.15, 0.0, other);
                    let (x, y, z) = (col(&m, 0), col(&m, 1), col(&m, 2));
                    for v in [x, y, z] {
                        assert!((dot(v, v) - 1.0).abs() < 1e-4, "{hand:?} {spot:?}");
                    }
                    assert!(dot(x, y).abs() < 1e-4 && dot(y, z).abs() < 1e-4 && dot(x, z).abs() < 1e-4);
                    let cross = [x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]];
                    assert!(close(cross, z, 1e-4), "{hand:?} {spot:?}");
                }
            }
        }
    }

    #[test]
    fn xsoverlay_s_wrist_settings_turn_into_steamvr_terms() {
        let xs = xsoverlay_wrist_from(XS_SETTINGS).unwrap();
        assert_eq!(xs.hand, Hand::Left);
        assert!((xs.width - 0.17).abs() < 1e-6);
        // Unity's z ahead is SteamVR's minus z: the panel is 13.5 cm back along the arm.
        assert!(close(xs.at, [-0.0574, 0.0107, 0.1349], 1e-3));
        // Worked out by hand from the same numbers: it faces out of the back of the hand
        // (minus x on the left) with its top towards the elbow (plus z).
        assert!(close(column(&xs.turn, 2), [-0.962, -0.211, -0.174], 2e-3));
        assert!(close(column(&xs.turn, 1), [-0.010, -0.609, 0.793], 2e-3));
        // Left-handed people keep it on the right.
        assert_eq!(xsoverlay_wrist_from(&XS_SETTINGS.replace("\"DominantHand\": 0", "\"DominantHand\": 1")).unwrap().hand, Hand::Right);
        // A file that says something odd is ignored rather than guessed at.
        assert!(xsoverlay_wrist_from("{}").is_none());
        // Hidden in XSOverlay: nothing to line up with.
        assert!(xsoverlay_wrist_from(&XS_SETTINGS.replace("\"DominantHand\": 0", "\"DominantHand\": 0, \"HideWristOverlay\": true")).is_none());
        assert!(xsoverlay_wrist_from(&XS_SETTINGS.replace("-0.1349358707666397", "7.5")).is_none());
        assert!(xsoverlay_wrist_from("not json").is_none());
    }

    #[test]
    fn on_xsoverlay_s_hand_the_feed_lines_up_with_its_wrist_panel() {
        let xs = xsoverlay_wrist_from(XS_SETTINGS).unwrap();
        let height = 0.15;
        let wrist = on_arm(Hand::Left, Spot::Wrist, height, 0.0, Some(&xs));
        // The back of the wrist is XSOverlay's own place and turn, half a centimetre in front.
        let face = column(&xs.turn, 2);
        assert!(close(col(&wrist, 3), std::array::from_fn(|i| xs.at[i] + face[i] * 0.005), 1e-6));
        for c in 0..3 {
            assert!(close(col(&wrist, c), column(&xs.turn, c), 1e-6));
        }
        // Up the arm: the same turn, moved along its top edge just past XSOverlay's panel.
        let up = on_arm(Hand::Left, Spot::Forearm, height, 0.0, Some(&xs));
        assert!(close(col(&up, 2), column(&xs.turn, 2), 1e-6));
        let step: [f32; 3] = std::array::from_fn(|i| up[i][3] - xs.at[i]);
        let along = dot(step, column(&xs.turn, 1));
        assert!((along - (xs.width * XS_WRIST_ASPECT / 2.0 + GAP + height / 2.0)).abs() < 1e-5);
        assert!(dot(step, column(&xs.turn, 0)).abs() < 1e-5 && dot(step, column(&xs.turn, 2)).abs() < 1e-5);
        // The slider moves it along the arm only.
        let moved = on_arm(Hand::Left, Spot::Forearm, height, 0.05, Some(&xs));
        let extra: [f32; 3] = std::array::from_fn(|i| moved[i][3] - up[i][3]);
        assert!(close(extra, column(&xs.turn, 1).map(|v| v * 0.05), 1e-6));
        // On the other hand XSOverlay's panel is not there, so it is not lined up with.
        let right = on_arm(Hand::Right, Spot::Wrist, height, 0.0, Some(&xs));
        assert_eq!(right, on_arm(Hand::Right, Spot::Wrist, height, 0.0, None));
    }

    #[test]
    fn without_xsoverlay_it_faces_out_of_the_back_of_the_hand_and_the_hands_mirror() {
        let left = on_arm(Hand::Left, Spot::Wrist, 0.15, 0.0, None);
        let right = on_arm(Hand::Right, Spot::Wrist, 0.15, 0.0, None);
        // The back of the left hand is minus x, of the right hand plus x; the top runs up the arm.
        assert!(col(&left, 2)[0] < -0.99 && col(&right, 2)[0] > 0.99);
        assert!(col(&left, 1)[2] > 0.7 && col(&right, 1)[2] > 0.7);
        assert!(close(col(&right, 3), [-left[0][3], left[1][3], left[2][3]], 1e-6));
        // The inside of the wrist faces out of the palm instead, across the wrist.
        let inside = on_arm(Hand::Left, Spot::Inside, 0.15, 0.0, None);
        assert!(col(&inside, 2)[0] > 0.99 && inside[0][3] > 0.0);
        assert!(close(col(&inside, 1), col(&left, 1), 1e-6));
        // Up the arm sits further towards the elbow than the wrist itself.
        let up = on_arm(Hand::Left, Spot::Forearm, 0.15, 0.0, None);
        assert!(up[2][3] > left[2][3] + 0.05);
    }

    #[test]
    fn a_notice_floats_in_front_and_upright_above_or_below_the_middle() {
        for above in [true, false] {
            let m = in_view(above);
            assert!(m[2][3] < -0.5, "in front of the eyes is minus z");
            assert_eq!([m[0][0], m[1][1], m[2][2]], [1.0, 1.0, 1.0]);
            assert_eq!(m[1][3] > 0.0, above);
        }
        // XSOverlay's notices sit 0.125 m under the middle; above keeps clear of them.
        assert!(in_view(true)[1][3] > 0.1);
    }

    #[test]
    fn steamvr_structs_have_the_sizes_its_header_gives() {
        assert_eq!(std::mem::size_of::<VrEvent>(), 64);
        assert_eq!(std::mem::size_of::<Pose>(), 80);
        assert_eq!(std::mem::offset_of!(Pose, result), 72);
        assert_eq!(std::mem::offset_of!(Pose, valid), 76);
        assert_eq!(std::mem::offset_of!(Pose, connected), 77);
        assert_eq!(std::mem::offset_of!(VrEvent, data), 16);
    }

    #[test]
    fn a_click_on_the_tab_comes_out_in_pixels_from_the_top() {
        let mut event: VrEvent = unsafe { std::mem::zeroed() };
        event.kind = EVENT_MOUSE_UP;
        let mut first = [0u8; 8];
        first[0..4].copy_from_slice(&100.0f32.to_le_bytes());
        first[4..8].copy_from_slice(&700.0f32.to_le_bytes());
        event.data[0] = u64::from_le_bytes(first);
        event.data[1] = u64::from(MOUSE_LEFT);
        // 700 up from the bottom of an 816 tall picture is 116 down from its top.
        assert_eq!(click(&event, 816.0), Some((100.0, 116.0)));
        event.data[1] = 2;
        assert_eq!(click(&event, 816.0), None, "the right button does nothing");
        event.data[1] = u64::from(MOUSE_LEFT);
        event.kind = 301;
        assert_eq!(click(&event, 816.0), None, "pressing alone is not a click");
    }

    #[test]
    fn a_battery_is_told_at_20_and_at_10_once_each() {
        let mut told = 0;
        assert_eq!(battery_step(0.80, false, &mut told), None);
        assert_eq!(battery_step(0.19, false, &mut told), Some(20));
        assert_eq!(battery_step(0.15, false, &mut told), None);
        assert_eq!(battery_step(0.09, false, &mut told), Some(10));
        assert_eq!(battery_step(0.05, false, &mut told), None);
        // Charging lets it be told again later.
        assert_eq!(battery_step(0.06, true, &mut told), None);
        assert_eq!(battery_step(0.06, false, &mut told), Some(10));
        // Coming on already low is told straight away.
        let mut fresh = 0;
        assert_eq!(battery_step(0.08, false, &mut fresh), Some(10));
        // A small rise over 20% does not count as charged.
        let mut wobble = 0;
        assert_eq!(battery_step(0.20, false, &mut wobble), Some(20));
        assert_eq!(battery_step(0.21, false, &mut wobble), None);
        assert_eq!(battery_step(0.20, false, &mut wobble), None);
    }

    #[test]
    fn tracking_counts_as_lost_after_three_seconds_and_once() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut w = Watched::default();
        // Never seen tracked: not lost, however long.
        assert!(!w.pose(false, at(0), true));
        assert!(!w.pose(false, at(5000), true));
        assert!(!w.pose(true, at(5100), true));
        assert!(!w.pose(false, at(6000), true));
        assert!(!w.pose(false, at(8900), true));
        assert!(w.pose(false, at(9000), true), "three seconds without tracking");
        assert!(!w.pose(false, at(12000), true), "told once");
        assert!(w.lost);
        // Back for less than a second still counts as lost.
        assert!(!w.pose(true, at(12100), true));
        assert!(!w.pose(true, at(12600), true));
        assert!(w.lost);
        assert!(!w.pose(true, at(13200), true));
        assert!(!w.lost);
        // A short drop is not a loss.
        assert!(!w.pose(false, at(14000), true));
        assert!(!w.pose(true, at(15000), true));
        assert!(!w.pose(false, at(16000), true));
        assert!(!w.pose(false, at(18500), true));
        assert!(w.pose(false, at(19000), true));
    }

    #[test]
    fn devices_are_named_by_hand_and_trackers_by_order() {
        let mut d = Devices::default();
        let w = |class, role| Watched { class, role, connected: true, has_battery: true, battery: Some(0.5), ..Watched::default() };
        d.by_index.insert(0, Watched { has_battery: false, ..w(CLASS_HMD, 0) });
        d.by_index.insert(3, w(CLASS_CONTROLLER, ROLE_RIGHT));
        d.by_index.insert(4, w(CLASS_CONTROLLER, ROLE_LEFT));
        d.by_index.insert(7, w(CLASS_TRACKER, 0));
        d.by_index.insert(5, w(CLASS_TRACKER, 0));
        d.trackers = vec![5, 7];
        assert_eq!(d.names(4).1, "Left controller");
        assert_eq!(d.names(3).0, "Right");
        assert_eq!(d.names(5).0, "Tracker 1");
        assert_eq!(d.names(7).1, "Tracker 2");
        let names: Vec<String> = d.list().into_iter().map(|s| s.name).collect();
        // The headset has no battery to show here, so it is left out.
        assert_eq!(names, ["Left", "Right", "Tracker 1", "Tracker 2"]);
        assert_eq!(d.list()[0].battery, Some(50));
    }

    #[test]
    fn switched_off_it_never_asks_about_steamvr() {
        fn never() -> bool {
            panic!("asked although the panels are off");
        }
        let overlay = Overlay::start(never, quiet());
        overlay.notice(PathBuf::from("nothing.png"), 3.0, 1.0, true);
        overlay.wrist(None);
        overlay.watch(true);
        std::thread::sleep(Duration::from_millis(1300));
        assert_eq!(overlay.status(), "Switched off.");
        assert!(!overlay.connected());
        assert!(overlay.try_native().starts_with("The app is not in SteamVR"));
    }

    fn pose(good: bool) -> Pose {
        let mut p: Pose = unsafe { std::mem::zeroed() };
        p.connected = true;
        p.valid = good;
        p.result = if good { TRACKING_OK } else { 101 };
        p
    }

    fn rig() -> Devices {
        let mut d = Devices::default();
        let w = |class, role| Watched { class, role, connected: true, ..Watched::default() };
        d.by_index.insert(0, w(CLASS_HMD, 0));
        d.by_index.insert(1, w(CLASS_CONTROLLER, ROLE_LEFT));
        d.by_index.insert(2, w(CLASS_CONTROLLER, ROLE_RIGHT));
        // A controller with no hand, like the hand-tracking stand-ins some headsets add.
        d.by_index.insert(3, w(CLASS_CONTROLLER, 0));
        d.by_index.insert(4, w(CLASS_TRACKER, 0));
        d.trackers = vec![4];
        d
    }

    /// Feeds the same poses every half second from `from` to `to` and gathers what is told.
    fn run_poses(d: &mut Devices, poses: &[Pose], start: Instant, from: u64, to: u64) -> Vec<News> {
        let mut out = Vec::new();
        for ms in (from..=to).step_by(500) {
            look_at_poses(poses, d, &mut out, true, start + Duration::from_millis(ms));
        }
        out
    }

    #[test]
    fn devices_lost_together_make_one_notice_and_stand_ins_none() {
        let start = Instant::now();
        let mut d = rig();
        let all_good = [pose(true), pose(true), pose(true), pose(true), pose(true)];
        assert!(run_poses(&mut d, &all_good, start, 0, 1000).is_empty());
        // Both controllers, the stand-in and the tracker drop while the headset keeps tracking.
        let dropped = [pose(true), pose(false), pose(false), pose(false), pose(false)];
        let told = run_poses(&mut d, &dropped, start, 1500, 8000);
        assert_eq!(told.len(), 1, "{told:?}");
        let News::Alert { title, body, .. } = &told[0] else { panic!() };
        assert_eq!(title, "3 devices lost tracking");
        assert_eq!(body, "Left, Right and Tracker 1. Check the base stations or cameras.");
    }

    #[test]
    fn a_short_headset_drop_does_not_count_against_the_controllers() {
        let start = Instant::now();
        let mut d = rig();
        let good = [pose(true), pose(true), pose(true), pose(true), pose(true)];
        run_poses(&mut d, &good, start, 0, 1000);
        // The headset is out from 1.5 to 3 s; the controllers stay bad until 5 s, under three
        // seconds once the headset is back.
        let asleep = [pose(false), pose(false), pose(false), pose(false), pose(false)];
        let mut told = run_poses(&mut d, &asleep, start, 1500, 3000);
        let back = [pose(true), pose(false), pose(false), pose(false), pose(true)];
        told.extend(run_poses(&mut d, &back, start, 3500, 5000));
        told.extend(run_poses(&mut d, &good, start, 5500, 9000));
        assert!(told.is_empty(), "{told:?}");
    }

    #[test]
    fn a_loss_waiting_while_the_headset_blinks_is_told_once_it_is_back() {
        let start = Instant::now();
        let mut d = rig();
        let good = [pose(true), pose(true), pose(true), pose(true), pose(true)];
        run_poses(&mut d, &good, start, 0, 1000);
        // Tracker 1 out from 1.5 s, lost at 4.5 s; the headset blinks out from 5 to 6 s, while
        // the notice is still waiting; the tracker stays out.
        let tracker_out = [pose(true), pose(true), pose(true), pose(true), pose(false)];
        let mut told = run_poses(&mut d, &tracker_out, start, 1500, 4500);
        let blink = [pose(false), pose(true), pose(true), pose(true), pose(false)];
        told.extend(run_poses(&mut d, &blink, start, 5000, 6000));
        told.extend(run_poses(&mut d, &tracker_out, start, 6500, 20000));
        assert!(matches!(told.as_slice(), [News::Alert { title, .. }] if title == "Tracker 1 lost tracking"), "{told:?}");
    }

    #[test]
    fn a_long_loss_inside_the_minute_is_told_when_the_minute_is_up() {
        let start = Instant::now();
        let mut d = rig();
        let good = [pose(true), pose(true), pose(true), pose(true), pose(true)];
        let out = [pose(true), pose(true), pose(true), pose(true), pose(false)];
        run_poses(&mut d, &good, start, 0, 1000);
        // Lost at 4.5 s, told at 6 s; back from 10 to 12 s; out again from 12.5 s for good.
        let mut told = run_poses(&mut d, &out, start, 1500, 9500);
        told.extend(run_poses(&mut d, &good, start, 10000, 12000));
        told.extend(run_poses(&mut d, &out, start, 12500, 90000));
        assert_eq!(told.len(), 2, "{told:?}");
    }

    #[test]
    fn a_headset_that_keeps_blinking_does_not_hide_a_real_loss() {
        let start = Instant::now();
        let mut d = rig();
        let good = [pose(true), pose(true), pose(true), pose(true), pose(true)];
        run_poses(&mut d, &good, start, 0, 1000);
        // Tracker 1 out for good from 1.5 s; the headset misses one look in four.
        let mut told = Vec::new();
        let mut ms = 1500;
        let mut n = 0;
        while ms <= 60_000 {
            let head = n % 4 != 3;
            let poses = [pose(head), pose(true), pose(true), pose(true), pose(false)];
            look_at_poses(&poses, &mut d, &mut told, true, start + Duration::from_millis(ms));
            ms += 500;
            n += 1;
        }
        assert!(matches!(told.as_slice(), [News::Alert { title, .. }] if title == "Tracker 1 lost tracking"), "{told:?}");
    }

    #[test]
    fn devices_dropping_one_after_another_share_notices() {
        let start = Instant::now();
        let mut d = rig();
        for i in 5..13 {
            d.by_index.insert(i, Watched { class: CLASS_TRACKER, connected: true, ..Watched::default() });
            d.trackers.push(i);
        }
        let good: Vec<Pose> = (0..13).map(|_| pose(true)).collect();
        run_poses(&mut d, &good, start, 0, 1000);
        // Both controllers and nine trackers, eleven in all, drop 2.5 s apart and stay out.
        let mut told = Vec::new();
        for ms in (1500..=60_000).step_by(500) {
            let out_now = (ms - 1500) / 2500 + 1;
            let poses: Vec<Pose> = (0..13usize)
                .map(|i| {
                    let order = match i {
                        1 | 2 => i,
                        4..=12 => i - 1,
                        _ => usize::MAX,
                    };
                    pose(order == usize::MAX || order as u64 > out_now)
                })
                .collect();
            look_at_poses(&poses, &mut d, &mut told, true, start + Duration::from_millis(ms));
        }
        // One notice every fifteen seconds at most, not one per device.
        assert!(told.len() <= 4, "{} notices: {told:?}", told.len());
        let named: usize = told
            .iter()
            .map(|n| match n {
                News::Alert { title, .. } => title.split_whitespace().next().and_then(|w| w.parse::<usize>().ok()).unwrap_or(1),
                _ => 0,
            })
            .sum();
        assert_eq!(named, 11, "every device is told about once: {told:?}");
    }

    #[test]
    fn switched_off_devices_make_one_notice_and_one_a_minute_each() {
        let start = Instant::now();
        let mut d = rig();
        for w in d.by_index.values_mut() {
            w.seen_good = true;
        }
        d.switched_off(1);
        d.switched_off(4);
        // A second word about the same switch-off adds nothing.
        d.switched_off(4);
        let first = d.off_notice(start, true).unwrap();
        assert!(matches!(&first, News::Alert { title, body, .. } if title == "2 devices switched off" && body == "Left and Tracker 1."), "{first:?}");
        // The same tracker again within the minute: held until the minute has passed.
        d.switched_on(4);
        d.by_index.get_mut(&4).unwrap().seen_good = true;
        d.switched_off(4);
        assert!(d.off_notice(start + Duration::from_secs(20), true).is_none());
        assert!(d.off_notice(start + Duration::from_secs(61), true).is_some());
        // With the alerts off, a switch-off waiting is dropped, not told later.
        d.switched_on(1);
        d.by_index.get_mut(&1).unwrap().seen_good = true;
        d.switched_off(1);
        assert!(d.off_notice(start + Duration::from_secs(200), false).is_none());
        assert!(d.off_notice(start + Duration::from_secs(201), true).is_none());
    }

    #[test]
    fn switch_offs_after_a_loss_wait_for_the_gap_and_go_out_together() {
        let start = Instant::now();
        let mut d = rig();
        for w in d.by_index.values_mut() {
            w.seen_good = true;
        }
        // Tracker 1 lost tracking and was told at `t`.
        d.by_index.get_mut(&4).unwrap().untold = true;
        assert!(d.lost_notice(start).is_none());
        let t = start + LOSS_BATCH;
        assert!(d.lost_notice(t).is_some());
        // Four seconds later the left controller switches off, and the right one at ten.
        d.switched_off(1);
        assert!(d.off_notice(t + Duration::from_secs(4), true).is_none());
        d.switched_off(2);
        assert!(d.off_notice(t + Duration::from_secs(10), true).is_none());
        // Once the gap has passed, both go out as one notice.
        let both = d.off_notice(t + NOTICE_GAP, true).unwrap();
        assert!(matches!(&both, News::Alert { title, body, .. } if title == "2 devices switched off" && body == "Left and Right."), "{both:?}");
        // A device that comes back before it is told is not reported as off.
        d.switched_off(4);
        d.switched_on(4);
        assert!(d.off_notice(t + Duration::from_secs(60), true).is_none());
    }

    #[test]
    fn a_controller_that_loses_its_hand_while_waiting_is_still_told_by_name() {
        let start = Instant::now();
        let mut d = rig();
        for w in d.by_index.values_mut() {
            w.seen_good = true;
        }
        // Tracker 1 lost tracking and was told at `t`.
        d.by_index.get_mut(&4).unwrap().untold = true;
        assert!(d.lost_notice(start).is_none());
        let t = start + LOSS_BATCH;
        assert!(d.lost_notice(t).is_some());
        // The left controller switches off four seconds later, and while its notice waits for the
        // gap, the next read of the roles finds it has no hand any more.
        d.switched_off(1);
        assert!(d.off_notice(t + Duration::from_secs(4), true).is_none());
        d.by_index.get_mut(&1).unwrap().role = 0;
        let told = d.off_notice(t + NOTICE_GAP, true).unwrap();
        assert!(matches!(&told, News::Alert { title, .. } if title == "Left controller switched off"), "{told:?}");
        // A controller that had no hand when it switched off is still never told about.
        d.switched_off(3);
        assert!(d.off_notice(t + Duration::from_secs(200), true).is_none());
    }

    #[test]
    fn a_device_that_keeps_dropping_is_told_about_once_a_minute() {
        let start = Instant::now();
        let mut d = rig();
        let good = [pose(true), pose(true), pose(true), pose(true), pose(true)];
        let drop = [pose(true), pose(true), pose(true), pose(true), pose(false)];
        let mut told = Vec::new();
        // Four seconds out, one and a half back, for a minute. A shorter drop of three and a
        // half seconds is told too, at most once a minute.
        for out_ms in [4000, 3500] {
            let mut d = rig();
            let mut told = Vec::new();
            // Ten minutes of it: once a minute at most, and at least once.
            let mut at = 0;
            while at < 600_000 {
                told.extend(run_poses(&mut d, &good, start, at, at + 1500));
                told.extend(run_poses(&mut d, &drop, start, at + 2000, at + 2000 + out_ms));
                at += 2500 + out_ms;
            }
            assert!((5..=10).contains(&told.len()), "out {out_ms} ms: {} notices", told.len());
        }
        let mut at = 0;
        while at < 60_000 {
            told.extend(run_poses(&mut d, &good, start, at, at + 1500));
            told.extend(run_poses(&mut d, &drop, start, at + 2000, at + 6000));
            at += 6500;
        }
        assert_eq!(told.len(), 1, "{told:?}");
    }

    #[test]
    fn nothing_is_told_when_the_headset_itself_drops() {
        let start = Instant::now();
        let mut d = rig();
        let all_good = [pose(true), pose(true), pose(true), pose(true), pose(true)];
        run_poses(&mut d, &all_good, start, 0, 1000);
        // A headset asleep or out of tracking takes everything with it: no news.
        let asleep = [pose(false), pose(false), pose(false), pose(false), pose(false)];
        assert!(run_poses(&mut d, &asleep, start, 1500, 10000).is_empty());
        // One device alone is still told by name.
        let mut d = rig();
        run_poses(&mut d, &all_good, start, 0, 1000);
        let one = [pose(true), pose(true), pose(true), pose(true), pose(false)];
        let told = run_poses(&mut d, &one, start, 1500, 8000);
        assert!(matches!(told.as_slice(), [News::Alert { title, .. }] if title == "Tracker 1 lost tracking"), "{told:?}");
    }

    #[test]
    fn stand_ins_without_a_battery_stay_off_the_list() {
        let mut d = rig();
        let names: Vec<String> = d.list().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["Left", "Right", "Tracker 1"]);
        d.by_index.get_mut(&3).unwrap().has_battery = true;
        assert!(d.list().iter().any(|s| s.name == "Controller"));
    }
}
