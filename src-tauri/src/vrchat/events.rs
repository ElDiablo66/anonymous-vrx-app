//! Turns pipeline websocket messages into friend changes and feed entries.

use std::collections::HashMap;

use serde_json::Value;

use super::model::{now_utc, FeedDraft, Friend, World};

#[derive(Debug, PartialEq)]
pub enum Effect {
    FriendChanged(Friend),
    FriendRemoved(String),
    Feed(FeedDraft),
    /// A world that came along with the event, saved so its name needs no request.
    WorldSeen(World),
}

/// Splits a raw websocket text frame into its type and content.
/// The content usually arrives as JSON packed inside a string.
pub fn read_frame(raw: &str) -> Option<(String, Value)> {
    let frame: Value = serde_json::from_str(raw).ok()?;
    if let Some(err) = frame.get("err").and_then(Value::as_str) {
        return Some(("error".into(), Value::String(err.to_string())));
    }
    let kind = frame.get("type")?.as_str()?.to_string();
    let content = match frame.get("content") {
        Some(Value::String(s)) => {
            serde_json::from_str::<Value>(s).unwrap_or_else(|_| Value::String(s.clone()))
        }
        Some(other) => other.clone(),
        None => Value::Null,
    };
    Some((kind, content))
}

fn user_id(content: &Value) -> Option<String> {
    content
        .get("userId")
        .or_else(|| content.get("userid"))
        .and_then(Value::as_str)
        .or_else(|| content.pointer("/user/id").and_then(Value::as_str))
        .map(str::to_string)
}

fn draft(f: &Friend, kind: &'static str, detail: &str, previous: &str) -> Effect {
    Effect::Feed(FeedDraft {
        user_id: f.id.clone(),
        display_name: f.display_name.clone(),
        kind,
        detail: detail.to_string(),
        previous: previous.to_string(),
        location: f.location.clone(),
    })
}

fn is_real_place(location: &str) -> bool {
    !matches!(location, "" | "offline" | "traveling")
}

/// Compares the friend list as last saved with a freshly fetched one, to catch
/// what happened while the app was closed: new friends, lost friends, new names.
pub fn list_changes(old: &HashMap<String, Friend>, new: &HashMap<String, Friend>) -> Vec<FeedDraft> {
    let mut out = Vec::new();
    // Nothing saved yet means a first run, not sixty new friends.
    if old.is_empty() {
        return out;
    }
    for (id, f) in new {
        match old.get(id) {
            None => out.push(FeedDraft {
                user_id: id.clone(),
                display_name: f.display_name.clone(),
                kind: "friendAdded",
                detail: String::new(),
                previous: String::new(),
                location: String::new(),
            }),
            Some(before) if !before.display_name.is_empty() && !f.display_name.is_empty() && before.display_name != f.display_name => {
                out.push(FeedDraft {
                    user_id: id.clone(),
                    display_name: f.display_name.clone(),
                    kind: "name",
                    detail: f.display_name.clone(),
                    previous: before.display_name.clone(),
                    location: String::new(),
                });
            }
            _ => {}
        }
    }
    // A list that came back much shorter is more likely a hiccup than a mass unfriending.
    if new.len() * 2 >= old.len() {
        for (id, f) in old {
            if !new.contains_key(id) {
                out.push(FeedDraft {
                    user_id: id.clone(),
                    display_name: f.display_name.clone(),
                    kind: "friendRemoved",
                    detail: String::new(),
                    previous: String::new(),
                    location: String::new(),
                });
            }
        }
    }
    out
}

/// Applies one event to the friend map and says what changed.
pub fn apply(friends: &mut HashMap<String, Friend>, kind: &str, content: &Value) -> Vec<Effect> {
    let mut out = Vec::new();
    let Some(id) = user_id(content) else {
        return out;
    };
    let empty = Value::Null;
    let user = content.get("user").unwrap_or(&empty);
    if let Some(world) = content.get("world").and_then(World::from_json) {
        out.push(Effect::WorldSeen(world));
    }

    match kind {
        "friend-online" => {
            let before = friends.get(&id).cloned();
            let f = friends
                .entry(id.clone())
                .or_insert_with(|| Friend { id: id.clone(), ..Default::default() });
            let place_before = f.location.clone();
            f.merge_user(user);
            f.state = "online".into();
            if let Some(loc) = content.get("location").and_then(Value::as_str) {
                f.set_location(loc);
            }
            if f.location != place_before {
                f.since = now_utc();
                f.since_at_least = false;
            }
            if let Some(t) = content.get("travelingToLocation").and_then(Value::as_str) {
                f.traveling_to = t.to_string();
            }
            if let Some(p) = content.get("platform").and_then(Value::as_str) {
                f.platform = p.to_string();
            }
            if before.as_ref().map(|b| b.state.as_str()) != Some("online") {
                out.push(draft(f, "online", "", ""));
            }
            out.push(Effect::FriendChanged(f.clone()));
        }
        "friend-active" | "friend-offline" => {
            let Some(f) = friends.get_mut(&id) else {
                if let Some(mut f) = Friend::from_user(user, "offline") {
                    f.state = if kind == "friend-active" { "active" } else { "offline" }.into();
                    f.set_location("offline");
                    friends.insert(id, f.clone());
                    out.push(Effect::FriendChanged(f));
                }
                return out;
            };
            let was_online = f.state == "online";
            let last_place = f.location.clone();
            f.merge_user(user);
            f.state = if kind == "friend-active" { "active" } else { "offline" }.into();
            f.set_location("offline");
            f.traveling_to.clear();
            f.before_travel.clear();
            f.since.clear();
            f.since_at_least = false;
            if let Some(p) = content.get("platform").and_then(Value::as_str) {
                f.platform = p.to_string();
            }
            if was_online {
                let mut e = draft(f, "offline", "", "");
                if let Effect::Feed(d) = &mut e {
                    d.location = last_place;
                }
                out.push(e);
            }
            out.push(Effect::FriendChanged(f.clone()));
        }
        "friend-location" => {
            let f = friends
                .entry(id.clone())
                .or_insert_with(|| Friend { id: id.clone(), ..Default::default() });
            let before = f.location.clone();
            f.merge_user(user);
            f.state = "online".into();
            if let Some(loc) = content.get("location").and_then(Value::as_str) {
                f.set_location(loc);
            }
            f.traveling_to = content
                .get("travelingToLocation")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if f.location != before {
                f.since = now_utc();
                f.since_at_least = false;
            }
            // VRChat sends "traveling" between two places. The place they left is kept over
            // that step, so the feed says where they really came from, not "traveling".
            if f.location == "traveling" {
                if is_real_place(&before) {
                    f.before_travel = before.clone();
                }
            } else if is_real_place(&f.location) && f.location != before {
                let came_from = if before == "traveling" { std::mem::take(&mut f.before_travel) } else { before.clone() };
                f.before_travel.clear();
                out.push(draft(f, "location", "", &came_from));
            } else {
                f.before_travel.clear();
            }
            out.push(Effect::FriendChanged(f.clone()));
        }
        "friend-update" => {
            let Some(f) = friends.get_mut(&id) else {
                return out;
            };
            let before = f.clone();
            // The user object here has no trustworthy location, so keep ours.
            let (location, place, traveling) =
                (f.location.clone(), f.place.clone(), f.traveling_to.clone());
            f.merge_user(user);
            f.location = location;
            f.place = place;
            f.traveling_to = traveling;
            if f.display_name != before.display_name && !before.display_name.is_empty() {
                out.push(draft(f, "name", &f.display_name, &before.display_name));
            }
            if f.status != before.status && !before.status.is_empty() {
                out.push(draft(f, "status", &f.status, &before.status));
            }
            if f.status_description != before.status_description {
                out.push(draft(
                    f,
                    "statusText",
                    &f.status_description,
                    &before.status_description,
                ));
            }
            if f.avatar_image != before.avatar_image && !before.avatar_image.is_empty() {
                out.push(draft(f, "avatar", "", ""));
            }
            if *f != before {
                out.push(Effect::FriendChanged(f.clone()));
            }
        }
        "friend-add" => {
            if let Some(f) = Friend::from_user(user, "offline") {
                let mut f = f;
                if is_real_place(&f.location) {
                    f.state = "online".into();
                }
                friends.insert(id, f.clone());
                out.push(draft(&f, "friendAdded", "", ""));
                out.push(Effect::FriendChanged(f));
            }
        }
        "friend-delete" => {
            if let Some(f) = friends.remove(&id) {
                out.push(draft(&f, "friendRemoved", "", ""));
            }
            out.push(Effect::FriendRemoved(id));
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn feed_kinds(effects: &[Effect]) -> Vec<&'static str> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::Feed(d) => Some(d.kind),
                _ => None,
            })
            .collect()
    }

    fn frame(kind: &str, content: Value) -> String {
        json!({ "type": kind, "content": content.to_string() }).to_string()
    }

    #[test]
    fn unpacks_string_content() {
        let raw = frame("friend-offline", json!({ "userId": "usr_1", "platform": "" }));
        let (kind, content) = read_frame(&raw).unwrap();
        assert_eq!(kind, "friend-offline");
        assert_eq!(content["userId"], "usr_1");
    }

    #[test]
    fn keeps_plain_string_content() {
        let raw = json!({ "type": "see-notification", "content": "not_123" }).to_string();
        let (kind, content) = read_frame(&raw).unwrap();
        assert_eq!(kind, "see-notification");
        assert_eq!(content, Value::String("not_123".into()));
    }

    #[test]
    fn reports_error_frames() {
        let (kind, content) = read_frame(r#"{"err":"authToken doesn't correspond with an active session"}"#).unwrap();
        assert_eq!(kind, "error");
        assert!(content.as_str().unwrap().contains("authToken"));
    }

    #[test]
    fn online_then_move_then_leave() {
        let mut friends = HashMap::new();
        friends.insert(
            "usr_1".to_string(),
            Friend::from_user(&json!({ "id": "usr_1", "displayName": "Mossy", "location": "offline" }), "offline").unwrap(),
        );

        let (k, c) = read_frame(&frame(
            "friend-online",
            json!({ "userId": "usr_1", "platform": "standalonewindows", "location": "private",
                    "user": { "id": "usr_1", "displayName": "Mossy", "status": "active" } }),
        ))
        .unwrap();
        let fx = apply(&mut friends, &k, &c);
        assert_eq!(feed_kinds(&fx), vec!["online"]);
        assert_eq!(friends["usr_1"].state, "online");
        assert_eq!(friends["usr_1"].place.kind, "private");

        let loc = "wrld_aaa:777~hidden(usr_9)~region(eu)";
        let (k, c) = read_frame(&frame(
            "friend-location",
            json!({ "userId": "usr_1", "location": loc, "travelingToLocation": "", "worldId": "wrld_aaa",
                    "world": { "id": "wrld_aaa", "name": "Quiet Pier", "thumbnailImageUrl": "https://example.test/t.png" },
                    "user": { "id": "usr_1", "displayName": "Mossy" } }),
        ))
        .unwrap();
        let fx = apply(&mut friends, &k, &c);
        assert_eq!(feed_kinds(&fx), vec!["location"]);
        assert!(fx.iter().any(|e| matches!(e, Effect::WorldSeen(w) if w.name == "Quiet Pier")));
        assert_eq!(friends["usr_1"].place.access.as_deref(), Some("friendsPlus"));

        let arrived = friends["usr_1"].since.clone();
        assert!(!arrived.is_empty(), "arrival time is stamped on a move");

        // The same location again is not news, and does not restart the clock.
        let fx = apply(&mut friends, &k, &c);
        assert!(feed_kinds(&fx).is_empty());
        assert_eq!(friends["usr_1"].since, arrived);

        let (k, c) = read_frame(&frame("friend-active", json!({ "userid": "usr_1", "platform": "web",
            "user": { "id": "usr_1", "displayName": "Mossy" } }))).unwrap();
        let fx = apply(&mut friends, &k, &c);
        assert_eq!(feed_kinds(&fx), vec!["offline"]);
        assert_eq!(friends["usr_1"].state, "active");
        assert_eq!(friends["usr_1"].since, "");
        assert_eq!(friends["usr_1"].platform, "web");
        match fx.iter().find(|e| matches!(e, Effect::Feed(_))).unwrap() {
            Effect::Feed(d) => assert_eq!(d.location, loc),
            _ => unreachable!(),
        }

        // Going from the website to fully offline adds nothing to the feed.
        let (k, c) = read_frame(&frame("friend-offline", json!({ "userId": "usr_1" }))).unwrap();
        let fx = apply(&mut friends, &k, &c);
        assert!(feed_kinds(&fx).is_empty());
        assert_eq!(friends["usr_1"].state, "offline");
    }

    #[test]
    fn traveling_is_not_a_place() {
        let mut friends = HashMap::new();
        let (k, c) = read_frame(&frame(
            "friend-location",
            json!({ "userId": "usr_2", "location": "traveling", "travelingToLocation": "wrld_b:1",
                    "user": { "id": "usr_2", "displayName": "Pip" } }),
        ))
        .unwrap();
        let fx = apply(&mut friends, &k, &c);
        assert!(feed_kinds(&fx).is_empty());
        assert_eq!(friends["usr_2"].traveling_to, "wrld_b:1");
    }

    #[test]
    fn the_place_before_travelling_is_where_they_came_from() {
        let mut friends = HashMap::new();
        let mut moved = |content: Value| {
            let (k, c) = read_frame(&frame("friend-location", content)).unwrap();
            apply(&mut friends, &k, &c)
        };
        let previous = |fx: &[Effect]| {
            fx.iter()
                .find_map(|e| match e {
                    Effect::Feed(d) if d.kind == "location" => Some(d.previous.clone()),
                    _ => None,
                })
                .expect("a location entry")
        };
        moved(json!({ "userId": "usr_5", "location": "private", "user": { "id": "usr_5", "displayName": "Ash" } }));
        let fx = moved(json!({ "userId": "usr_5", "location": "traveling", "travelingToLocation": "wrld_x:1",
            "user": { "id": "usr_5", "displayName": "Ash" } }));
        assert!(feed_kinds(&fx).is_empty());
        // The same "traveling" word twice does not lose the place they left.
        moved(json!({ "userId": "usr_5", "location": "traveling", "travelingToLocation": "wrld_x:1",
            "user": { "id": "usr_5", "displayName": "Ash" } }));
        let fx = moved(json!({ "userId": "usr_5", "location": "wrld_x:1", "user": { "id": "usr_5", "displayName": "Ash" } }));
        assert_eq!(previous(&fx), "private");

        // A move without the travelling step names the place before, as always.
        let fx = moved(json!({ "userId": "usr_5", "location": "wrld_y:2", "user": { "id": "usr_5", "displayName": "Ash" } }));
        assert_eq!(previous(&fx), "wrld_x:1");
        // Nothing is carried over into the next trip.
        let fx = moved(json!({ "userId": "usr_5", "location": "traveling", "user": { "id": "usr_5", "displayName": "Ash" } }));
        assert!(feed_kinds(&fx).is_empty());
        let fx = moved(json!({ "userId": "usr_5", "location": "wrld_z:3", "user": { "id": "usr_5", "displayName": "Ash" } }));
        assert_eq!(previous(&fx), "wrld_y:2");
    }

    #[test]
    fn update_reports_status_changes_and_keeps_location() {
        let mut friends = HashMap::new();
        let mut f = Friend::from_user(
            &json!({ "id": "usr_3", "displayName": "Juno", "status": "active", "statusDescription": "hi" }),
            "online",
        )
        .unwrap();
        f.set_location("wrld_c:5");
        friends.insert("usr_3".to_string(), f);
        let (k, c) = read_frame(&frame(
            "friend-update",
            json!({ "userId": "usr_3", "user": { "id": "usr_3", "displayName": "Juno",
                    "status": "busy", "statusDescription": "building", "location": "" } }),
        ))
        .unwrap();
        let fx = apply(&mut friends, &k, &c);
        assert_eq!(feed_kinds(&fx), vec!["status", "statusText"]);
        assert_eq!(friends["usr_3"].location, "wrld_c:5");
    }

    #[test]
    fn list_changes_catch_what_happened_while_closed() {
        let person = |id: &str, name: &str| {
            (id.to_string(), Friend { id: id.into(), display_name: name.into(), ..Default::default() })
        };
        let old: HashMap<String, Friend> = [person("usr_1", "Mossy"), person("usr_2", "Pip"), person("usr_3", "Rook")].into();
        let new: HashMap<String, Friend> = [person("usr_1", "Mossy"), person("usr_2", "Pippin"), person("usr_4", "Juno")].into();
        let mut kinds: Vec<(&str, String)> = list_changes(&old, &new).into_iter().map(|d| (d.kind, d.display_name)).collect();
        kinds.sort();
        assert_eq!(kinds, [("friendAdded", "Juno".to_string()), ("friendRemoved", "Rook".to_string()), ("name", "Pippin".to_string())]);

        // A first run records nothing, and a list that shrank by more than half is not trusted.
        assert!(list_changes(&HashMap::new(), &new).is_empty());
        let tiny: HashMap<String, Friend> = [person("usr_1", "Mossy")].into();
        assert!(list_changes(&old, &tiny).iter().all(|d| d.kind != "friendRemoved"));
    }

    #[test]
    fn add_and_delete() {
        let mut friends = HashMap::new();
        let (k, c) = read_frame(&frame("friend-add", json!({ "userId": "usr_4",
            "user": { "id": "usr_4", "displayName": "Rook", "location": "offline" } }))).unwrap();
        assert_eq!(feed_kinds(&apply(&mut friends, &k, &c)), vec!["friendAdded"]);
        let (k, c) = read_frame(&frame("friend-delete", json!({ "userId": "usr_4" }))).unwrap();
        let fx = apply(&mut friends, &k, &c);
        assert_eq!(feed_kinds(&fx), vec!["friendRemoved"]);
        assert!(friends.is_empty());
    }
}
