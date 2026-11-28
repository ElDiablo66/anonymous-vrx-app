//! Notifications: friend requests, invites, invite requests and the newer
//! group-style notifications, from the API and from the live websocket.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reply {
    /// What VRChat calls the reply, for example `accept` or `decline`.
    pub kind: String,
    pub data: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub id: String,
    /// 1 for the classic kind (friend request, invite), 2 for the newer kind.
    pub version: u8,
    /// `friendRequest`, `invite`, `requestInvite`, `inviteResponse`,
    /// `requestInviteResponse`, `boop`, `votetokick`, `message`, or a v2 type.
    pub kind: String,
    pub sender_id: String,
    pub sender_name: String,
    pub title: String,
    pub message: String,
    /// UTC, RFC 3339.
    pub at: String,
    pub seen: bool,
    /// For invites: the instance being offered.
    pub location: String,
    pub world_name: String,
    pub image: String,
    pub replies: Vec<Reply>,
    /// Where a newer notice points (`group:grp_...`), with whatever ids came along in its
    /// data. The window reads the group out of it to open from the row.
    #[serde(default)]
    pub link: String,
    /// The untouched v2 object, kept so later partial updates can be merged in.
    #[serde(skip)]
    raw: Value,
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// `details` arrives either as an object or as JSON packed into a string.
fn details(v: &Value) -> Value {
    match v.get("details") {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
        Some(other) => other.clone(),
        None => Value::Null,
    }
}

impl Notice {
    pub fn from_v1(v: &Value) -> Option<Notice> {
        let id = text(v, "id");
        if id.is_empty() {
            return None;
        }
        let d = details(v);
        let extra = ["inviteMessage", "requestMessage", "responseMessage"]
            .iter()
            .map(|k| text(&d, k))
            .find(|s| !s.is_empty())
            .unwrap_or_default();
        let message = if extra.is_empty() { text(v, "message") } else { extra };
        Some(Notice {
            id,
            version: 1,
            kind: text(v, "type"),
            sender_id: text(v, "senderUserId"),
            sender_name: text(v, "senderUsername"),
            title: String::new(),
            message,
            at: text(v, "created_at"),
            seen: v.get("seen").and_then(Value::as_bool).unwrap_or(false),
            location: text(&d, "worldId"),
            world_name: text(&d, "worldName"),
            image: text(&d, "imageUrl"),
            replies: Vec::new(),
            link: String::new(),
            raw: Value::Null,
        })
    }

    pub fn from_v2(v: &Value) -> Option<Notice> {
        let id = text(v, "id");
        if id.is_empty() {
            return None;
        }
        let replies = v
            .get("responses")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .map(|r| Reply {
                        kind: text(r, "type"),
                        data: text(r, "data"),
                        text: text(r, "text"),
                    })
                    .filter(|r| !r.kind.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        Some(Notice {
            id,
            version: 2,
            kind: text(v, "type"),
            sender_id: text(v, "senderUserId"),
            sender_name: text(v, "senderUsername"),
            title: text(v, "title"),
            message: text(v, "message"),
            at: text(v, "createdAt"),
            seen: v.get("seen").and_then(Value::as_bool).unwrap_or(false),
            location: String::new(),
            world_name: String::new(),
            image: text(v, "imageUrl"),
            replies,
            link: {
                let data = v.get("data").filter(|d| !d.is_null()).map(Value::to_string).unwrap_or_default();
                format!("{} {data}", text(v, "link")).trim().chars().take(600).collect()
            },
            raw: v.clone(),
        })
    }
}

fn put(list: &mut Vec<Notice>, notice: Notice) -> bool {
    let is_new = match list.iter().position(|n| n.id == notice.id) {
        Some(i) => {
            list[i] = notice;
            false
        }
        None => {
            list.push(notice);
            true
        }
    };
    list.sort_by(|a, b| b.at.cmp(&a.at));
    is_new
}

/// A notification the user dealt with, kept so they can look back at it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Handled {
    pub id: i64,
    /// When the user acted. UTC, RFC 3339.
    pub at: String,
    /// `accept`, `dismiss`, `reply` or `inviteBack`.
    pub action: String,
    /// For `reply`: the words on the button that was pressed.
    pub reply_text: String,
    pub notice: Notice,
}

pub struct Outcome {
    pub changed: bool,
    /// Set when a notification arrived that was not in the list before.
    pub arrived: Option<Notice>,
    /// Notifications VRChat cleared on its side, for example because the game showed them.
    pub removed: Vec<Notice>,
}

/// Takes the matching notifications out of the list and hands them back.
fn take(list: &mut Vec<Notice>, keep: impl Fn(&Notice) -> bool) -> Vec<Notice> {
    let (gone, kept): (Vec<Notice>, Vec<Notice>) = list.drain(..).partition(|n| !keep(n));
    *list = kept;
    gone
}

/// Applies one websocket event to the list. Events of other kinds change nothing.
pub fn apply(list: &mut Vec<Notice>, kind: &str, content: &Value) -> Outcome {
    let mut out = Outcome {
        changed: false,
        arrived: None,
        removed: Vec::new(),
    };
    match kind {
        "notification" | "notification-v2" => {
            let built = if kind == "notification" {
                Notice::from_v1(content)
            } else {
                Notice::from_v2(content)
            };
            if let Some(notice) = built {
                if put(list, notice.clone()) {
                    out.arrived = Some(notice);
                }
                out.changed = true;
            }
        }
        "notification-v2-update" => {
            let id = text(content, "id");
            let Some(i) = list.iter().position(|n| n.id == id && n.version == 2) else {
                return out;
            };
            let mut raw = list[i].raw.clone();
            if let (Some(target), Some(updates)) = (
                raw.as_object_mut(),
                content.get("updates").and_then(Value::as_object),
            ) {
                for (k, v) in updates {
                    target.insert(k.clone(), v.clone());
                }
            }
            if let Some(notice) = Notice::from_v2(&raw) {
                list[i] = notice;
                out.changed = true;
            }
        }
        "notification-v2-delete" => {
            let ids: Vec<&str> = content
                .get("ids")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            out.removed = take(list, |n| !ids.contains(&n.id.as_str()));
            out.changed = !out.removed.is_empty();
        }
        "hide-notification" | "see-notification" => {
            let Some(id) = content.as_str() else { return out };
            if kind == "hide-notification" {
                out.removed = take(list, |n| n.id != id);
                out.changed = !out.removed.is_empty();
            } else if let Some(n) = list.iter_mut().find(|n| n.id == id) {
                out.changed = !n.seen;
                n.seen = true;
            }
        }
        "response-notification" => {
            // Someone answered the notification somewhere else (in the game, or on the website).
            let id = text(content, "notificationId");
            if !id.is_empty() {
                out.removed = take(list, |n| n.id != id);
                out.changed = !out.removed.is_empty();
            }
        }
        "clear-notification" => {
            out.removed = take(list, |n| n.version != 1);
            out.changed = !out.removed.is_empty();
        }
        _ => {}
    }
    out
}

/// Replaces the list with what the two API calls returned.
pub fn from_api(v1: &[Value], v2: &[Value]) -> Vec<Notice> {
    let mut list: Vec<Notice> = v1
        .iter()
        .filter_map(Notice::from_v1)
        .chain(v2.iter().filter_map(Notice::from_v2))
        .collect();
    list.sort_by(|a, b| b.at.cmp(&a.at));
    list
}

/// Which alert switch a notification belongs to.
pub fn alert_group(n: &Notice) -> &'static str {
    match n.kind.as_str() {
        "friendRequest" => "friendRequests",
        "invite" | "requestInvite" | "inviteResponse" | "requestInviteResponse" => "invites",
        "boop" => "boops",
        _ => "everythingElse",
    }
}

/// Title and body for a Windows notification.
pub fn popup_text(n: &Notice) -> (String, String) {
    let who = if n.sender_name.is_empty() { "Someone".to_string() } else { n.sender_name.clone() };
    let place = if n.world_name.is_empty() { String::new() } else { format!(" to {}", n.world_name) };
    match n.kind.as_str() {
        "friendRequest" => (format!("{who} wants to be friends"), "Open the Inbox to accept or decline.".into()),
        "invite" => (format!("{who} invited you{place}"), n.message.clone()),
        "requestInvite" => (format!("{who} asks for an invite"), n.message.clone()),
        "inviteResponse" => (format!("{who} answered your invite"), n.message.clone()),
        "requestInviteResponse" => (format!("{who} answered your invite request"), n.message.clone()),
        "boop" => {
            // The newer boop keeps its words in the title, like "Boop!".
            let words = if n.message.is_empty() { n.title.clone() } else { n.message.clone() };
            (format!("{who} booped you"), words)
        }
        "votetokick" => ("Vote to kick".into(), n.message.clone()),
        _ => {
            let title = if !n.title.is_empty() {
                n.title.clone()
            } else if !n.sender_name.is_empty() {
                format!("From {}", n.sender_name)
            } else {
                "VRChat notification".into()
            };
            (title, n.message.clone())
        }
    }
}

/// The colour and the word a notice gets on the app's own panel in the headset.
pub fn popup_kind(n: &Notice) -> (&'static str, &'static str) {
    match n.kind.as_str() {
        "friendRequest" => ("invite", "Friend request"),
        "invite" => ("invite", "Invite"),
        "requestInvite" => ("invite", "Invite request"),
        "inviteResponse" | "requestInviteResponse" => ("invite", "Answer"),
        "boop" => ("invite", "Boop"),
        _ => ("invite", "From VRChat"),
    }
}

pub fn remove(list: &mut Vec<Notice>, id: &str) {
    list.retain(|n| n.id != id);
}

/// When the newer notifications could not be read, the ones already listed stay: a failed
/// read says nothing about whether they are gone. Keeps the list newest first.
pub fn keep_newer_kind(fresh: &mut Vec<Notice>, before: &[Notice]) {
    for n in before.iter().filter(|n| n.version == 2) {
        if !fresh.iter().any(|f| f.id == n.id) {
            fresh.push(n.clone());
        }
    }
    fresh.sort_by(|a, b| b.at.cmp(&a.at));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn invite() -> Value {
        json!({
            "id": "not_1", "type": "invite", "senderUserId": "usr_1", "senderUsername": "Mossy",
            "message": "This is a generated invite to Quiet Pier", "created_at": "2026-01-05T20:00:00.000Z", "seen": false,
            "details": "{\"worldId\":\"wrld_a:1~hidden(usr_1)~region(eu)\",\"worldName\":\"Quiet Pier\",\"inviteMessage\":\"come sit\"}"
        })
    }

    #[test]
    fn reads_an_invite_with_packed_details() {
        let n = Notice::from_v1(&invite()).unwrap();
        assert_eq!(n.kind, "invite");
        assert_eq!(n.sender_name, "Mossy");
        assert_eq!(n.world_name, "Quiet Pier");
        assert!(n.location.starts_with("wrld_a:1"));
        assert_eq!(n.message, "come sit");
    }

    #[test]
    fn reads_details_given_as_an_object() {
        let v = json!({ "id": "not_2", "type": "requestInvite", "senderUserId": "usr_2", "senderUsername": "Pip",
            "message": "", "created_at": "2026-01-05T20:01:00.000Z", "details": { "requestMessage": "room for one more?" } });
        assert_eq!(Notice::from_v1(&v).unwrap().message, "room for one more?");
    }

    #[test]
    fn live_events_add_mark_and_remove() {
        let mut list = Vec::new();
        let first = apply(&mut list, "notification", &invite());
        assert!(first.changed);
        assert_eq!(first.arrived.unwrap().id, "not_1");
        // The same one again is an update, not an arrival.
        assert!(apply(&mut list, "notification", &invite()).arrived.is_none());

        let request = json!({ "id": "not_0", "type": "friendRequest", "senderUserId": "usr_3", "senderUsername": "Rook",
            "message": "", "created_at": "2026-01-05T21:00:00.000Z" });
        apply(&mut list, "notification", &request);
        assert_eq!(list[0].id, "not_0", "newest first");

        assert!(apply(&mut list, "see-notification", &json!("not_1")).changed);
        assert!(list.iter().find(|n| n.id == "not_1").unwrap().seen);
        assert!(apply(&mut list, "hide-notification", &json!("not_1")).changed);
        assert_eq!(list.len(), 1);
        assert!(apply(&mut list, "clear-notification", &Value::Null).changed);
        assert!(list.is_empty());
        assert!(!apply(&mut list, "friend-online", &json!({ "userId": "usr_1" })).changed);
    }

    #[test]
    fn a_boop_the_game_clears_is_handed_back_and_can_be_announced() {
        let mut list = Vec::new();
        let boop = json!({ "id": "not_b", "type": "boop", "senderUserId": "usr_9", "senderUsername": "Ash",
            "title": "Boop!", "message": "", "createdAt": "2026-01-05T22:00:00.000Z", "seen": false,
            "responses": [ { "type": "boop", "data": "usr_9", "text": "Boop back" } ] });
        let got = apply(&mut list, "notification-v2", &boop);
        let arrived = got.arrived.unwrap();
        assert_eq!(alert_group(&arrived), "boops");
        assert_eq!(popup_text(&arrived), ("Ash booped you".to_string(), "Boop!".to_string()));
        // The game shows the boop and VRChat deletes it a moment later.
        let gone = apply(&mut list, "notification-v2-delete", &json!({ "ids": ["not_b"], "version": 2 }));
        assert_eq!(gone.removed.len(), 1);
        assert_eq!(gone.removed[0].id, "not_b");
        assert!(list.is_empty());
    }

    #[test]
    fn an_answer_given_elsewhere_removes_the_notification() {
        let mut list = Vec::new();
        apply(&mut list, "notification", &invite());
        let out = apply(&mut list, "response-notification", &json!({ "notificationId": "not_1", "receiverId": "usr_me", "responseId": "not_r" }));
        assert!(out.changed);
        assert_eq!(out.removed[0].id, "not_1");
        assert!(list.is_empty());
        assert_eq!(alert_group(&out.removed[0]), "invites");
        assert_eq!(popup_text(&out.removed[0]).0, "Mossy invited you to Quiet Pier");
    }

    #[test]
    fn newer_kind_updates_and_deletes() {
        let mut list = Vec::new();
        let group = json!({
            "id": "not_g", "type": "group.invite", "senderUserId": "grp_1", "senderUsername": "",
            "title": "Group invite", "message": "Night Owls invited you", "createdAt": "2026-01-05T22:00:00.000Z",
            "seen": false, "imageUrl": "https://example.test/g.png",
            "responses": [ { "type": "accept", "data": "grp_1,usr_me", "text": "Accept" },
                           { "type": "decline", "data": "grp_1,usr_me", "text": "Decline" } ]
        });
        let got = apply(&mut list, "notification-v2", &group);
        assert_eq!(got.arrived.unwrap().replies.len(), 2);
        assert_eq!(list[0].replies[0].text, "Accept");

        let update = json!({ "id": "not_g", "version": 2, "updates": { "seen": true, "message": "Still waiting" } });
        assert!(apply(&mut list, "notification-v2-update", &update).changed);
        assert!(list[0].seen);
        assert_eq!(list[0].message, "Still waiting");
        assert_eq!(list[0].replies.len(), 2, "untouched fields survive an update");

        assert!(apply(&mut list, "notification-v2-delete", &json!({ "ids": ["not_g"], "version": 2 })).changed);
        assert!(list.is_empty());
    }

    #[test]
    fn a_failed_read_of_the_newer_kind_keeps_the_ones_listed() {
        let group = json!({ "id": "not_g", "type": "group.announcement", "title": "News", "message": "",
            "createdAt": "2026-01-05T23:00:00.000Z", "seen": false });
        let before = from_api(&[invite()], &[group]);
        assert_eq!(before.len(), 2);
        // The newer kind could not be read this time: only the classic ones came back.
        let mut fresh = from_api(&[invite()], &[]);
        keep_newer_kind(&mut fresh, &before);
        let ids: Vec<&str> = fresh.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["not_g", "not_1"], "kept, once each, newest first");
        // A classic one that is gone stays gone.
        let mut fresh = from_api(&[], &[]);
        keep_newer_kind(&mut fresh, &before);
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].id, "not_g");
    }
}
