//! The shapes the window works with, built from VRChat's looser JSON.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    /// `world`, `private`, `traveling` or `none`.
    pub kind: String,
    pub world_id: Option<String>,
    pub instance: Option<String>,
    /// `public`, `friendsPlus`, `friends`, `invitePlus`, `invite`, `group`, `groupPlus`, `groupPublic`.
    pub access: Option<String>,
    pub region: Option<String>,
    pub group_id: Option<String>,
}

/// Reads a location string such as `wrld_x:12345~hidden(usr_y)~region(eu)`.
pub fn parse_location(raw: &str) -> Place {
    let raw = raw.trim();
    match raw {
        "" | "offline" => {
            return Place {
                kind: "none".into(),
                ..Default::default()
            }
        }
        "private" => {
            return Place {
                kind: "private".into(),
                ..Default::default()
            }
        }
        "traveling" => {
            return Place {
                kind: "traveling".into(),
                ..Default::default()
            }
        }
        _ => {}
    }
    let (world, rest) = match raw.split_once(':') {
        Some((w, r)) => (w, Some(r)),
        None => (raw, None),
    };
    if !world.starts_with("wrld_") && !world.starts_with("wld_") {
        return Place {
            kind: "none".into(),
            ..Default::default()
        };
    }
    let mut place = Place {
        kind: "world".into(),
        world_id: Some(world.to_string()),
        ..Default::default()
    };
    let Some(rest) = rest else { return place };
    let mut parts = rest.split('~');
    place.instance = parts.next().map(str::to_string);
    let mut access = "public".to_string();
    let mut can_request = false;
    let mut group_access: Option<String> = None;
    for part in parts {
        let (key, value) = match part.split_once('(') {
            Some((k, v)) => (k, Some(v.trim_end_matches(')'))),
            None => (part, None),
        };
        match key {
            "hidden" => access = "friendsPlus".into(),
            "friends" => access = "friends".into(),
            "private" => access = "invite".into(),
            "canRequestInvite" => can_request = true,
            "group" => {
                access = "group".into();
                place.group_id = value.map(str::to_string);
            }
            "groupAccessType" => group_access = value.map(str::to_string),
            "region" => place.region = value.map(str::to_string),
            _ => {}
        }
    }
    if access == "invite" && can_request {
        access = "invitePlus".into();
    }
    if access == "group" {
        match group_access.as_deref() {
            Some("plus") => access = "groupPlus".into(),
            Some("public") => access = "groupPublic".into(),
            _ => {}
        }
    }
    place.access = Some(access);
    place
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Friend {
    pub id: String,
    pub display_name: String,
    /// `online` (in VRChat), `active` (website or phone) or `offline`.
    pub state: String,
    /// VRChat's own status: `join me`, `active`, `ask me`, `busy`, `offline`.
    pub status: String,
    pub status_description: String,
    pub location: String,
    pub traveling_to: String,
    pub place: Place,
    pub image: String,
    pub platform: String,
    pub avatar_image: String,
    /// When this friend was first seen at their current location. UTC, RFC 3339.
    /// Empty when they were already there before the app could notice.
    #[serde(default)]
    pub since: String,
    /// True when `since` is only the moment the app first noticed them there,
    /// so the real stay is at least that long.
    #[serde(default)]
    pub since_at_least: bool,
    /// Trust rank in plain words, from the account tags. Empty until VRChat sends tags.
    #[serde(default)]
    pub rank: String,
    #[serde(default)]
    pub vrc_plus: bool,
    /// Age verified as 18+.
    #[serde(default)]
    pub adult: bool,
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn first_text(v: &Value, keys: &[&str]) -> String {
    keys.iter()
        .map(|k| text(v, k))
        .find(|s| !s.is_empty())
        .unwrap_or_default()
}

impl Friend {
    /// Builds a friend from a user object. `state` comes from the caller,
    /// because the user object alone does not tell website from in-game.
    pub fn from_user(user: &Value, state: &str) -> Option<Friend> {
        let id = text(user, "id");
        if id.is_empty() {
            return None;
        }
        let mut f = Friend {
            id,
            state: state.to_string(),
            ..Default::default()
        };
        f.merge_user(user);
        Some(f)
    }

    /// Copies over whatever fields the given user object carries.
    pub fn merge_user(&mut self, user: &Value) {
        let set = |target: &mut String, key: &str| {
            if let Some(v) = user.get(key).and_then(Value::as_str) {
                *target = v.to_string();
            }
        };
        set(&mut self.display_name, "displayName");
        set(&mut self.status, "status");
        set(&mut self.status_description, "statusDescription");
        if let Some(p) = user
            .get("platform")
            .or_else(|| user.get("last_platform"))
            .and_then(Value::as_str)
        {
            self.platform = p.to_string();
        }
        if let Some(loc) = user.get("location").and_then(Value::as_str) {
            self.set_location(loc);
        }
        if let Some(t) = user.get("travelingToLocation").and_then(Value::as_str) {
            self.traveling_to = t.to_string();
        }
        if let Some(tags) = user.get("tags").and_then(Value::as_array) {
            let tags: Vec<String> = tags.iter().filter_map(Value::as_str).map(str::to_string).collect();
            self.rank = crate::vrchat::people::rank(&tags).to_string();
            self.vrc_plus = tags.iter().any(|t| t == "system_supporter");
        }
        if let Some(age) = user.get("ageVerificationStatus").and_then(Value::as_str) {
            self.adult = age == "18+";
        } else if let Some(verified) = user.get("ageVerified").and_then(Value::as_bool) {
            self.adult = verified;
        }
        let avatar = first_text(
            user,
            &["currentAvatarThumbnailImageUrl", "currentAvatarImageUrl"],
        );
        if !avatar.is_empty() {
            self.avatar_image = avatar;
        }
        let image = first_text(user, &["iconUrl", "userIcon", "profilePicOverrideThumbnail", "profilePicOverride"]);
        if !image.is_empty() {
            self.image = image;
        } else if user.get("userIcon").is_some() || self.image.is_empty() {
            self.image = self.avatar_image.clone();
        }
    }

    pub fn set_location(&mut self, location: &str) {
        self.location = location.to_string();
        self.place = parse_location(location);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FeedItem {
    pub id: i64,
    /// UTC, RFC 3339.
    pub at: String,
    pub user_id: String,
    pub display_name: String,
    /// `online`, `offline`, `location`, `status`, `statusText`, `name`, `avatar`, `friendAdded`, `friendRemoved`.
    pub kind: String,
    pub detail: String,
    pub previous: String,
    pub location: String,
    pub world_id: Option<String>,
}

/// A feed entry before the database gives it an id and a time.
#[derive(Clone, Debug, PartialEq)]
pub struct FeedDraft {
    pub user_id: String,
    pub display_name: String,
    pub kind: &'static str,
    pub detail: String,
    pub previous: String,
    pub location: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct World {
    pub id: String,
    pub name: String,
    pub image: String,
}

impl World {
    pub fn from_json(v: &Value) -> Option<World> {
        let id = text(v, "id");
        let name = text(v, "name");
        if id.is_empty() || name.is_empty() {
            return None;
        }
        Some(World {
            id,
            name,
            image: first_text(v, &["thumbnailImageUrl", "imageUrl"]),
        })
    }
}

/// How busy one instance is.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Instance {
    pub location: String,
    pub users: i64,
    pub capacity: i64,
    pub full: bool,
}

impl Instance {
    pub fn from_json(location: &str, v: &Value) -> Option<Instance> {
        let users = v
            .get("n_users")
            .or_else(|| v.get("userCount"))
            .and_then(Value::as_i64)?;
        Some(Instance {
            location: location.to_string(),
            users,
            capacity: v.get("capacity").and_then(Value::as_i64).unwrap_or(0),
            full: v.get("full").and_then(Value::as_bool).unwrap_or(false),
        })
    }
}

/// The address of VRChat's own page for joining an instance.
pub fn join_page(location: &str) -> Option<String> {
    let place = parse_location(location);
    let world = place.world_id?;
    let rest = location.split_once(':')?.1;
    let instance: String = percent_encoding::utf8_percent_encode(rest, percent_encoding::NON_ALPHANUMERIC).to_string();
    Some(format!(
        "https://vrchat.com/home/launch?worldId={world}&instanceId={instance}"
    ))
}

/// The link that makes VRChat itself start in, or move to, an instance.
pub fn launch_link(location: &str) -> Option<String> {
    let place = parse_location(location);
    place.world_id.as_ref()?;
    place.instance.as_ref()?;
    // Only the characters a location is made of are let through.
    let safe = location
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_-:~().".contains(c));
    safe.then(|| format!("vrchat://launch?ref=vrchat.com&id={location}"))
}

/// Picks the first `"...exe"` out of a registry command line.
pub fn first_quoted_exe(text: &str) -> Option<String> {
    let mut parts = text.split('"');
    parts.next()?;
    parts
        .step_by(2)
        .find(|p| p.to_ascii_lowercase().ends_with(".exe"))
        .map(str::to_string)
}

/// The current time the way the rest of the history stores it.
pub fn now_utc() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteGroup {
    /// VRChat's internal name, for example `group_0`.
    pub key: String,
    /// The name the user gave it.
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Favorites {
    pub groups: Vec<FavoriteGroup>,
    /// Friend user id to group key.
    pub friends: std::collections::HashMap<String, String>,
}

impl Favorites {
    /// Builds the favourite list from the two API answers.
    pub fn from_api(favorites: &[Value], groups: &[Value]) -> Favorites {
        let mut out = Favorites::default();
        for f in favorites {
            if text(f, "type") != "friend" {
                continue;
            }
            let id = text(f, "favoriteId");
            let key = f
                .get("tags")
                .and_then(Value::as_array)
                .and_then(|t| t.first())
                .and_then(Value::as_str)
                .unwrap_or("group_0")
                .to_string();
            if !id.is_empty() {
                out.friends.insert(id, key);
            }
        }
        let mut keys: Vec<String> = out.friends.values().cloned().collect();
        keys.sort();
        keys.dedup();
        for key in keys {
            let named = groups
                .iter()
                .find(|g| text(g, "type") == "friend" && text(g, "name") == key)
                .map(|g| text(g, "displayName"))
                .filter(|n| !n.is_empty());
            let number = key
                .rsplit('_')
                .next()
                .and_then(|n| n.parse::<u32>().ok())
                .map_or(1, |n| n + 1);
            out.groups.push(FavoriteGroup {
                name: named.unwrap_or_else(|| format!("Favourites {number}")),
                key,
            });
        }
        out
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Me {
    pub id: String,
    pub display_name: String,
    pub status: String,
    pub status_description: String,
    pub image: String,
}

impl Me {
    pub fn from_user(user: &Value) -> Me {
        Me {
            id: text(user, "id"),
            display_name: text(user, "displayName"),
            status: text(user, "status"),
            status_description: text(user, "statusDescription"),
            image: first_text(
                user,
                &[
                    "iconUrl",
                    "userIcon",
                    "profilePicOverrideThumbnail",
                    "profilePicOverride",
                    "currentAvatarThumbnailImageUrl",
                    "currentAvatarImageUrl",
                ],
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favourites_map_friends_to_named_groups() {
        let favorites = vec![
            serde_json::json!({ "id": "fvrt_1", "type": "friend", "favoriteId": "usr_1", "tags": ["group_1"] }),
            serde_json::json!({ "id": "fvrt_2", "type": "friend", "favoriteId": "usr_2", "tags": ["group_0"] }),
            serde_json::json!({ "id": "fvrt_3", "type": "world", "favoriteId": "wrld_9", "tags": ["worlds1"] }),
        ];
        let groups = vec![
            serde_json::json!({ "type": "friend", "name": "group_1", "displayName": "Dance crew" }),
            serde_json::json!({ "type": "world", "name": "group_0", "displayName": "Not this one" }),
        ];
        let f = Favorites::from_api(&favorites, &groups);
        assert_eq!(f.friends.len(), 2);
        assert_eq!(f.friends["usr_1"], "group_1");
        assert_eq!(
            f.groups,
            vec![
                FavoriteGroup { key: "group_0".into(), name: "Favourites 1".into() },
                FavoriteGroup { key: "group_1".into(), name: "Dance crew".into() },
            ]
        );
    }

    #[test]
    fn instance_reads_either_count_field() {
        let a = Instance::from_json("wrld_a:1", &serde_json::json!({ "n_users": 12, "capacity": 32, "full": false })).unwrap();
        assert_eq!((a.users, a.capacity, a.full), (12, 32, false));
        let b = Instance::from_json("wrld_a:2", &serde_json::json!({ "userCount": 3 })).unwrap();
        assert_eq!((b.users, b.capacity), (3, 0));
        assert!(Instance::from_json("wrld_a:3", &serde_json::json!({ "error": "nope" })).is_none());
    }

    #[test]
    fn old_saved_friends_without_since_still_load() {
        let json = r#"{"id":"usr_1","displayName":"Mossy","state":"online","status":"active","statusDescription":"","location":"private","travelingTo":"","place":{"kind":"private","worldId":null,"instance":null,"access":null,"region":null,"groupId":null},"image":"","platform":"","avatarImage":""}"#;
        let f: Friend = serde_json::from_str(json).unwrap();
        assert_eq!(f.since, "");
    }

    #[test]
    fn join_page_only_for_real_instances() {
        assert_eq!(
            join_page("wrld_a:123~hidden(usr_9)~region(eu)").as_deref(),
            Some("https://vrchat.com/home/launch?worldId=wrld_a&instanceId=123%7Ehidden%28usr%5F9%29%7Eregion%28eu%29")
        );
        assert!(join_page("private").is_none());
        assert!(join_page("wrld_a").is_none());
        assert!(join_page("https://example.test/x:1").is_none());
    }

    #[test]
    fn launch_link_only_for_clean_instances() {
        assert_eq!(
            launch_link("wrld_a:123~hidden(usr_9)~region(eu)").as_deref(),
            Some("vrchat://launch?ref=vrchat.com&id=wrld_a:123~hidden(usr_9)~region(eu)")
        );
        assert!(launch_link("wrld_a").is_none());
        assert!(launch_link("private").is_none());
        assert!(launch_link("wrld_a:1&evil=1").is_none());
        assert!(launch_link("wrld_a:1 --flag").is_none());
    }

    #[test]
    fn finds_the_launcher_in_a_registry_line() {
        let text = "\r\nHKEY_CURRENT_USER\\Software\\Classes\\VRChat\\shell\\open\\command\r\n    (Default)    REG_SZ    \"D:\\Games\\VRChat\\launch.exe\" \"%1\" %*\r\n";
        assert_eq!(first_quoted_exe(text).as_deref(), Some("D:\\Games\\VRChat\\launch.exe"));
        assert_eq!(first_quoted_exe("ERROR: The system was unable to find the specified registry key"), None);
        assert_eq!(first_quoted_exe("    (Default)    REG_SZ    \"%1\" %*"), None);
    }

    #[test]
    fn newer_icon_field_wins_for_pictures() {
        let user = serde_json::json!({ "id": "usr_1", "displayName": "Mossy",
            "iconUrl": "https://example.test/icon-new.png", "userIcon": "https://example.test/icon-old.png",
            "currentAvatarImageUrl": "https://example.test/avatar.png" });
        assert_eq!(Friend::from_user(&user, "online").unwrap().image, "https://example.test/icon-new.png");
        assert_eq!(Me::from_user(&user).image, "https://example.test/icon-new.png");
    }

    #[test]
    fn friend_marks_come_from_tags_and_age() {
        let user = serde_json::json!({ "id": "usr_1", "displayName": "Mossy",
            "tags": ["system_trust_basic", "system_trust_known", "system_trust_trusted", "system_supporter"],
            "ageVerificationStatus": "18+" });
        let f = Friend::from_user(&user, "online").unwrap();
        assert_eq!((f.rank.as_str(), f.vrc_plus, f.adult), ("Known User", true, true));
        // An update without tags keeps what was known.
        let mut g = f.clone();
        g.merge_user(&serde_json::json!({ "status": "busy" }));
        assert_eq!((g.rank.as_str(), g.vrc_plus, g.adult), ("Known User", true, true));
    }

    #[test]
    fn reads_plain_states() {
        assert_eq!(parse_location("offline").kind, "none");
        assert_eq!(parse_location("").kind, "none");
        assert_eq!(parse_location("private").kind, "private");
        assert_eq!(parse_location("traveling").kind, "traveling");
    }

    #[test]
    fn reads_public_instance() {
        let p = parse_location("wrld_aaa:12345~region(eu)");
        assert_eq!(p.kind, "world");
        assert_eq!(p.world_id.as_deref(), Some("wrld_aaa"));
        assert_eq!(p.instance.as_deref(), Some("12345"));
        assert_eq!(p.access.as_deref(), Some("public"));
        assert_eq!(p.region.as_deref(), Some("eu"));
    }

    #[test]
    fn reads_access_types() {
        let access = |s: &str| parse_location(s).access.unwrap();
        assert_eq!(access("wrld_a:1~hidden(usr_b)~region(us)"), "friendsPlus");
        assert_eq!(access("wrld_a:1~friends(usr_b)~region(us)"), "friends");
        assert_eq!(access("wrld_a:1~private(usr_b)~region(us)"), "invite");
        assert_eq!(
            access("wrld_a:1~private(usr_b)~canRequestInvite~region(us)"),
            "invitePlus"
        );
        assert_eq!(
            access("wrld_a:1~group(grp_c)~groupAccessType(plus)~region(jp)"),
            "groupPlus"
        );
        assert_eq!(
            access("wrld_a:1~group(grp_c)~groupAccessType(public)"),
            "groupPublic"
        );
        assert_eq!(
            access("wrld_a:1~group(grp_c)~groupAccessType(members)"),
            "group"
        );
        assert_eq!(
            parse_location("wrld_a:1~group(grp_c)~groupAccessType(members)")
                .group_id
                .as_deref(),
            Some("grp_c")
        );
    }

    #[test]
    fn friend_picks_icon_before_avatar() {
        let user = serde_json::json!({
            "id": "usr_1", "displayName": "Mossy",
            "currentAvatarThumbnailImageUrl": "https://example.test/avatar.png",
            "userIcon": "https://example.test/icon.png",
            "status": "join me", "location": "private"
        });
        let f = Friend::from_user(&user, "online").unwrap();
        assert_eq!(f.image, "https://example.test/icon.png");
        assert_eq!(f.place.kind, "private");

        let user = serde_json::json!({
            "id": "usr_2", "displayName": "Pip", "userIcon": "",
            "currentAvatarThumbnailImageUrl": "https://example.test/avatar.png"
        });
        let f = Friend::from_user(&user, "offline").unwrap();
        assert_eq!(f.image, "https://example.test/avatar.png");
    }
}
