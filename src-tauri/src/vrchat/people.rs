//! People and worlds as shown in profiles and search results.

use serde::Serialize;
use serde_json::Value;

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn first_text(v: &Value, keys: &[&str]) -> String {
    keys.iter()
        .map(|k| text(v, k))
        .find(|s| !s.is_empty())
        .unwrap_or_default()
}

fn tags(v: &Value) -> Vec<String> {
    v.get("tags")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

/// VRChat's trust rank in the words the game uses, from the account's tags.
pub fn rank(tags: &[String]) -> &'static str {
    let has = |t: &str| tags.iter().any(|x| x == t);
    if has("system_troll") || has("system_probable_troll") {
        "Nuisance"
    } else if has("system_trust_veteran") {
        "Trusted User"
    } else if has("system_trust_trusted") {
        "Known User"
    } else if has("system_trust_known") {
        "User"
    } else if has("system_trust_basic") {
        "New User"
    } else {
        "Visitor"
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub id: String,
    pub display_name: String,
    pub image: String,
    pub bio: String,
    pub bio_links: Vec<String>,
    pub pronouns: String,
    pub status: String,
    pub status_description: String,
    pub rank: String,
    /// The day the account was made, `YYYY-MM-DD`.
    pub joined: String,
    pub last_login: String,
    pub last_platform: String,
    pub is_friend: bool,
    pub location: String,
    /// The note kept on VRChat's side for this person, if any.
    pub vrchat_note: String,
}

impl Person {
    pub fn from_json(v: &Value) -> Option<Person> {
        let id = text(v, "id");
        if id.is_empty() {
            return None;
        }
        let tags = tags(v);
        Some(Person {
            id,
            display_name: text(v, "displayName"),
            image: first_text(
                v,
                &[
                    "iconUrl",
                    "userIcon",
                    "profilePicOverrideThumbnail",
                    "profilePicOverride",
                    "currentAvatarThumbnailImageUrl",
                    "currentAvatarImageUrl",
                ],
            ),
            bio: text(v, "bio"),
            bio_links: v
                .get("bioLinks")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .filter(|l| l.starts_with("https://") || l.starts_with("http://"))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            pronouns: text(v, "pronouns"),
            status: text(v, "status"),
            status_description: text(v, "statusDescription"),
            rank: rank(&tags).to_string(),
            joined: text(v, "date_joined"),
            last_login: text(v, "last_login"),
            last_platform: text(v, "last_platform"),
            is_friend: v.get("isFriend").and_then(Value::as_bool).unwrap_or(false),
            location: text(v, "location"),
            vrchat_note: text(v, "note"),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldCard {
    pub id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub image: String,
    pub capacity: i64,
    pub visits: i64,
    pub favorites: i64,
    /// People in public instances of it right now.
    pub occupants: i64,
    /// The tags the author chose, without VRChat's prefix.
    pub tags: Vec<String>,
}

impl WorldCard {
    pub fn from_json(v: &Value) -> Option<WorldCard> {
        let id = text(v, "id");
        let name = text(v, "name");
        if id.is_empty() || name.is_empty() {
            return None;
        }
        let number = |k: &str| v.get(k).and_then(Value::as_i64).unwrap_or(0);
        Some(WorldCard {
            id,
            name,
            author: text(v, "authorName"),
            description: text(v, "description"),
            image: first_text(v, &["thumbnailImageUrl", "imageUrl"]),
            capacity: number("capacity"),
            visits: number("visits"),
            favorites: number("favorites"),
            occupants: number("occupants"),
            tags: tags(v)
                .into_iter()
                .filter_map(|t| t.strip_prefix("author_tag_").map(str::to_string))
                .collect(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarCard {
    pub id: String,
    pub name: String,
    pub author: String,
    pub author_id: String,
    pub description: String,
    pub image: String,
    /// VRChat's performance rank on PC and on Quest or phone, when the database knows it.
    pub pc: String,
    pub mobile: String,
}

impl AvatarCard {
    pub fn from_json(v: &Value) -> Option<AvatarCard> {
        let id = first_text(v, &["id", "Id"]);
        if !id.starts_with("avtr_") {
            return None;
        }
        let perf = v.get("performance").cloned().unwrap_or(Value::Null);
        Some(AvatarCard {
            id,
            name: first_text(v, &["name", "avatarName"]),
            author: text(v, "authorName"),
            author_id: text(v, "authorId"),
            description: text(v, "description"),
            image: first_text(v, &["thumbnailImageUrl", "imageUrl"]),
            pc: text(&perf, "pc_rating"),
            mobile: text(&perf, "android_rating"),
        })
    }
}

/// How much time the history shows the signed-in user spent with someone.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Together {
    /// Visits in which both were in the same instance.
    pub times: i64,
    pub seconds: i64,
    pub first: Option<String>,
    pub last: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rank_follows_the_highest_trust_tag() {
        let t = |list: &[&str]| rank(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(t(&[]), "Visitor");
        assert_eq!(t(&["system_trust_basic"]), "New User");
        assert_eq!(t(&["system_trust_basic", "system_trust_known"]), "User");
        assert_eq!(t(&["system_trust_basic", "system_trust_known", "system_trust_trusted"]), "Known User");
        assert_eq!(t(&["system_trust_veteran", "system_trust_trusted"]), "Trusted User");
    }

    #[test]
    fn person_keeps_only_web_links() {
        let p = Person::from_json(&json!({
            "id": "usr_1", "displayName": "Mossy", "bio": "pier person", "pronouns": "they/them",
            "bioLinks": ["https://example.test/mossy", "javascript:alert(1)", ""],
            "tags": ["system_trust_known"], "date_joined": "2021-04-02", "isFriend": true,
            "userIcon": "", "currentAvatarThumbnailImageUrl": "https://example.test/a.png", "note": "met at the pier"
        }))
        .unwrap();
        assert_eq!(p.bio_links, ["https://example.test/mossy"]);
        assert_eq!(p.rank, "User");
        assert_eq!(p.image, "https://example.test/a.png");
        assert!(p.is_friend);
        assert_eq!(p.vrchat_note, "met at the pier");
        assert!(Person::from_json(&json!({ "displayName": "no id" })).is_none());
    }

    #[test]
    fn avatar_card_needs_a_real_avatar_id() {
        let a = AvatarCard::from_json(&json!({
            "id": "avtr_1", "name": "Robot", "authorName": "Rook", "authorId": "usr_5",
            "thumbnailImageUrl": "https://example.test/t.png",
            "performance": { "pc_rating": "Good", "android_rating": "Medium" }
        }))
        .unwrap();
        assert_eq!((a.pc.as_str(), a.mobile.as_str(), a.author.as_str()), ("Good", "Medium", "Rook"));
        assert!(AvatarCard::from_json(&json!({ "id": "something else", "name": "x" })).is_none());
    }

    #[test]
    fn world_card_strips_tag_prefix() {
        let w = WorldCard::from_json(&json!({
            "id": "wrld_a", "name": "Quiet Pier", "authorName": "Rook", "capacity": 32, "visits": 1200,
            "favorites": 80, "occupants": 5, "tags": ["author_tag_chill", "system_approved", "author_tag_sunset"]
        }))
        .unwrap();
        assert_eq!(w.tags, ["chill", "sunset"]);
        assert_eq!((w.capacity, w.visits, w.occupants), (32, 1200, 5));
    }
}
