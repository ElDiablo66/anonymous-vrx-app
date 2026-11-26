//! The app's own panels in SteamVR: a notice in front of the eyes and a small feed on the wrist.
//!
//! The window draws each panel as a picture; this file only hands the picture to SteamVR and
//! says where it floats. It talks to SteamVR through `openvr_api.dll` from SteamVR's own
//! folder, loaded when SteamVR is running and let go of when SteamVR closes. Nothing is
//! bundled, and a PC without SteamVR never loads anything.
//!
//! The numbers below (which function sits where) belong to `IVROverlay_028` and
//! `IVRSystem_026` in Valve's `openvr_capi.h`. A SteamVR that does not know those versions
//! refuses the request, and the panels stay off.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const OVERLAY_TABLE: &str = "FnTable:IVROverlay_028";
const SYSTEM_TABLE: &str = "FnTable:IVRSystem_026";

// Places in the overlay table.
const CREATE_OVERLAY: usize = 1;
const DESTROY_OVERLAY: usize = 3;
const SET_ALPHA: usize = 16;
const SET_SORT_ORDER: usize = 20;
const SET_WIDTH: usize = 22;
const SET_TRANSFORM_DEVICE: usize = 35;
const SHOW_OVERLAY: usize = 43;
const HIDE_OVERLAY: usize = 44;
const SET_FROM_FILE: usize = 63;
// Places in the system table.
const CONTROLLER_FOR_ROLE: usize = 18;
const POLL_EVENT: usize = 30;
const ACKNOWLEDGE_QUIT: usize = 47;

const APP_OVERLAY: i32 = 2;
const EVENT_QUIT: u32 = 700;
const HMD: u32 = 0;
const NO_DEVICE: u32 = u32::MAX;

type Handle = u64;
/// Three rows of four: a turn, and in the last column a move in metres.
type Matrix = [[f32; 4]; 3];

type InitFn = unsafe extern "C" fn(*mut i32, i32) -> isize;
type ShutdownFn = unsafe extern "C" fn();
type InterfaceFn = unsafe extern "C" fn(*const c_char, *mut i32) -> isize;
type ErrorNameFn = unsafe extern "C" fn(i32) -> *const c_char;

/// Which hand wears the feed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Hand {
    Left,
    Right,
}

enum Cmd {
    /// Whether the app should be in SteamVR at all.
    Wanted(bool),
    Notice { picture: PathBuf, seconds: f32, alpha: f32 },
    /// `None` takes the feed off the wrist.
    Wrist(Option<(PathBuf, Hand, f32, f32)>),
}

/// The handle the rest of the app holds. Everything behind it runs on one thread of its own.
pub struct Overlay {
    tx: Mutex<Sender<Cmd>>,
    status: Arc<Mutex<String>>,
}

impl Overlay {
    /// `steamvr_running` is asked every few seconds while the panels are wanted.
    pub fn start(steamvr_running: fn() -> bool) -> Self {
        let (tx, rx) = channel();
        let status = Arc::new(Mutex::new("Switched off.".to_string()));
        let shared = status.clone();
        std::thread::spawn(move || run(rx, shared, steamvr_running));
        Self { tx: Mutex::new(tx), status }
    }

    pub fn wanted(&self, on: bool) {
        let _ = self.tx.lock().unwrap().send(Cmd::Wanted(on));
    }

    pub fn notice(&self, picture: PathBuf, seconds: f32, alpha: f32) {
        let _ = self.tx.lock().unwrap().send(Cmd::Notice { picture, seconds, alpha });
    }

    pub fn wrist(&self, feed: Option<(PathBuf, Hand, f32, f32)>) {
        let _ = self.tx.lock().unwrap().send(Cmd::Wrist(feed));
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

/// Floats a panel in front of the eyes: a little under the middle, an arm's length away.
pub fn in_view() -> Matrix {
    [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, -0.16], [0.0, 0.0, 1.0, -0.85]]
}

/// Lays a panel flat over the back of the hand, like a watch: its face up, its top edge
/// pointing where the controller points.
pub fn on_wrist(hand: Hand) -> Matrix {
    // A little towards the thumb side, so the controller's ring is not in the way.
    let side = if hand == Hand::Left { 0.03 } else { -0.03 };
    [[1.0, 0.0, 0.0, side], [0.0, 0.0, 1.0, 0.07], [0.0, -1.0, 0.0, 0.12]]
}

/// One stay in SteamVR, from signing in to letting go.
struct Session {
    // Kept so the DLL stays loaded for as long as the tables below are used.
    _lib: libloading::Library,
    shutdown: ShutdownFn,
    overlay: *const *const c_void,
    system: *const *const c_void,
    notice: Handle,
    wrist: Handle,
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
        let mut session = Self { _lib: lib, shutdown, overlay, system, notice: 0, wrist: 0 };
        session.notice = session.create("anonymous-vrx.notice", "Anonymous VRX notice");
        session.wrist = session.create("anonymous-vrx.wrist", "Anonymous VRX wrist feed");
        if session.notice == 0 || session.wrist == 0 {
            // Dropping the session lets go of SteamVR again.
            return Err("SteamVR did not make the panels. Another copy of the app may be holding them.".to_string());
        }
        // Notices sit above other panels, the wrist feed among them.
        session.call_u32(SET_SORT_ORDER, session.notice, 50);
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
        f(if hand == Hand::Left { 1 } else { 2 })
    }

    /// True when SteamVR said it is closing. It is told the app is on its way out.
    unsafe fn told_to_quit(&self) -> bool {
        let poll: unsafe extern "system" fn(*mut c_void, u32) -> bool = self.slot(self.system, POLL_EVENT);
        // Room for any event SteamVR has, lined up the way it expects.
        let mut event = [0u64; 32];
        let size = std::mem::size_of_val(&event) as u32;
        while poll(event.as_mut_ptr().cast(), size) {
            if (event[0] & 0xffff_ffff) as u32 == EVENT_QUIT {
                let acknowledge: unsafe extern "system" fn() = self.slot(self.system, ACKNOWLEDGE_QUIT);
                acknowledge();
                return true;
            }
        }
        false
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
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

fn run(rx: Receiver<Cmd>, status: Arc<Mutex<String>>, steamvr_running: fn() -> bool) {
    let mut wanted = false;
    let mut session: Option<Session> = None;
    let mut next_look = Instant::now();
    // What the wrist should show, kept so it is put back after SteamVR restarts.
    let mut feed: Option<(PathBuf, Hand, f32, f32)> = None;
    let mut feed_shown = false;
    let mut feed_device = NO_DEVICE;
    let mut feed_changed = false;
    let mut notice_until: Option<Instant> = None;

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
                    session = None;
                    feed_shown = false;
                    notice_until = None;
                    say(&status, "Switched off.");
                }
            }
            Ok(Cmd::Notice { picture, seconds, alpha }) => {
                if let Some(s) = &session {
                    unsafe {
                        if s.picture(s.notice, &picture) {
                            s.call_f32(SET_WIDTH, s.notice, 0.46);
                            s.call_f32(SET_ALPHA, s.notice, alpha.clamp(0.2, 1.0));
                            s.place(s.notice, HMD, &in_view());
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
                    feed_shown = false;
                    feed_device = NO_DEVICE;
                    feed_changed = true;
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
                session = None;
                notice_until = None;
                feed_shown = false;
                say(&status, "SteamVR closed. Waiting for it to start again.");
                continue;
            }
        }

        let s = session.as_ref().unwrap();
        unsafe {
            if s.told_to_quit() {
                session = None;
                notice_until = None;
                feed_shown = false;
                // SteamVR takes a moment to go; looking again at once would start it up again.
                next_look = Instant::now() + Duration::from_secs(20);
                say(&status, "SteamVR closed. Waiting for it to start again.");
                continue;
            }
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
                }
                Some((picture, hand, width, alpha)) => {
                    // A controller can be switched on late, or swap hands.
                    let device = s.controller(*hand);
                    if device == NO_DEVICE {
                        if feed_shown {
                            s.call(HIDE_OVERLAY, s.wrist);
                            feed_shown = false;
                        }
                        feed_device = NO_DEVICE;
                    } else {
                        if feed_changed {
                            s.picture(s.wrist, picture);
                            s.call_f32(SET_WIDTH, s.wrist, width.clamp(0.08, 0.4));
                            s.call_f32(SET_ALPHA, s.wrist, alpha.clamp(0.2, 1.0));
                        }
                        if feed_changed || device != feed_device {
                            s.place(s.wrist, device, &on_wrist(*hand));
                            feed_device = device;
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wrist_panel_lies_face_up_with_its_top_pointing_forward() {
        for hand in [Hand::Left, Hand::Right] {
            let m = on_wrist(hand);
            // Where the panel's own "towards the reader" ends up: straight up from the hand.
            assert_eq!([m[0][2], m[1][2], m[2][2]], [0.0, 1.0, 0.0]);
            // Where the panel's own "up" ends up: forward, which is minus z for a controller.
            assert_eq!([m[0][1], m[1][1], m[2][1]], [0.0, 0.0, -1.0]);
        }
        assert!(on_wrist(Hand::Left)[0][3] > 0.0 && on_wrist(Hand::Right)[0][3] < 0.0);
    }

    #[test]
    fn a_notice_floats_in_front_and_upright() {
        let m = in_view();
        assert!(m[2][3] < -0.5, "in front of the eyes is minus z");
        assert_eq!([m[0][0], m[1][1], m[2][2]], [1.0, 1.0, 1.0]);
    }

    #[test]
    fn switched_off_it_never_asks_about_steamvr() {
        fn never() -> bool {
            panic!("asked although the panels are off");
        }
        let overlay = Overlay::start(never);
        overlay.notice(PathBuf::from("nothing.png"), 3.0, 1.0);
        overlay.wrist(None);
        std::thread::sleep(Duration::from_millis(1300));
        assert_eq!(overlay.status(), "Switched off.");
    }
}
