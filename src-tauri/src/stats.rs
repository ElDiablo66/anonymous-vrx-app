//! My own numbers from the game-log history on this PC: how long I played, when, where and
//! with whom. The sums are worked out here, from the history alone.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Datelike, Duration, Local, NaiveDateTime, TimeZone, Timelike, Utc};

/// A stay longer than this was a stay the game never closed; only this much of it counts.
pub const CAP_SECS: f64 = 24.0 * 3600.0;

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatWorld {
    pub world_id: String,
    pub name: String,
    pub times: i64,
    pub seconds: i64,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatPerson {
    pub user_id: String,
    pub name: String,
    pub friend: bool,
    pub times: i64,
    pub seconds: i64,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatStay {
    pub visit_id: i64,
    pub world_id: String,
    pub world_name: String,
    pub joined_at: String,
    pub seconds: i64,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MyStats {
    pub from: String,
    pub to: String,
    pub total_seconds: i64,
    pub stays: i64,
    pub play_days: i64,
    pub worlds_count: i64,
    pub people_count: i64,
    pub longest: Option<StatStay>,
    /// Local date (YYYY-MM-DD) and seconds played that day, oldest first; days without play are left out.
    pub days: Vec<(String, i64)>,
    /// Seconds by hour of the day, 0 to 23, in the PC's own time.
    pub hours: Vec<i64>,
    /// Seconds by weekday, 0 is Sunday.
    pub weekdays: Vec<i64>,
    pub worlds: Vec<StatWorld>,
    pub people: Vec<StatPerson>,
    /// A stay is still going and counts up to now.
    pub open_now: bool,
    /// Stays that ran past a day and were cut there.
    pub capped: i64,
}

/// One of my stays as the history holds it, times as Julian days.
pub struct StayRow {
    pub visit_id: i64,
    pub world_id: String,
    pub world_name: String,
    pub joined_at: String,
    pub start: f64,
    pub left: Option<f64>,
    /// The last thing seen in the stay, for a stay the game never closed.
    pub last_seen: Option<f64>,
}

/// One time a person was in one of my stays, times as Julian days.
pub struct SightRow {
    pub key: String,
    pub user_id: String,
    pub name: String,
    pub visit_id: i64,
    pub start: f64,
    pub end: f64,
    pub friend: bool,
}

fn to_utc(jd: f64) -> DateTime<Utc> {
    let ms = ((jd - 2_440_587.5) * 86_400_000.0).round() as i64;
    Utc.timestamp_millis_opt(ms).single().unwrap_or_default()
}

/// Adds one piece of play time to the day, hour and weekday it fell in, hour by hour in the
/// PC's own time, so an evening that runs past midnight counts on both days.
fn spread(from: f64, to: f64, days: &mut BTreeMap<String, i64>, hours: &mut [i64], weekdays: &mut [i64]) {
    let mut at = to_utc(from);
    let end = to_utc(to);
    let mut guard = 0;
    while at < end && guard < 100_000 {
        guard += 1;
        let local = at.with_timezone(&Local).naive_local();
        let hour_start = local.date().and_hms_opt(local.hour(), 0, 0).unwrap_or(local);
        let next_local: NaiveDateTime = hour_start + Duration::hours(1);
        // A local hour that does not exist (the clock jumps forward) is passed over to the
        // first hour that does.
        let next = (0..4)
            .find_map(|skip| Local.from_local_datetime(&(next_local + Duration::hours(skip))).earliest())
            .map(|t| t.with_timezone(&Utc))
            .filter(|t| *t > at)
            .unwrap_or(at + Duration::hours(1));
        let step_end = next.min(end);
        let secs = (step_end - at).num_milliseconds() as f64 / 1000.0;
        let secs = secs.round() as i64;
        *days.entry(local.format("%Y-%m-%d").to_string()).or_insert(0) += secs;
        hours[local.hour() as usize] += secs;
        weekdays[local.weekday().num_days_from_sunday() as usize] += secs;
        at = step_end;
    }
}

/// Works the numbers out of my stays and the people in them, between two Julian days.
/// `running` says whether VRChat runs now: only then may a stay that was never closed count
/// up to now; otherwise it ends at the last thing seen in it.
pub fn work_out(from: f64, now: f64, stays: &[StayRow], sights: &[SightRow], running: bool) -> MyStats {
    let mut out = MyStats { hours: vec![0; 24], weekdays: vec![0; 7], ..MyStats::default() };
    let mut days: BTreeMap<String, i64> = BTreeMap::new();
    let newest_open = stays.iter().filter(|s| s.left.is_none()).map(|s| s.visit_id).max();
    let mut stay_end: HashMap<i64, f64> = HashMap::new();
    let mut worlds: HashMap<String, StatWorld> = HashMap::new();
    let mut reach = f64::MIN;
    for s in stays {
        let mut end = match s.left {
            Some(left) => left,
            None if running && Some(s.visit_id) == newest_open => {
                out.open_now = true;
                now
            }
            None => s.last_seen.unwrap_or(s.start).max(s.start),
        };
        if (end - s.start) * 86400.0 > CAP_SECS {
            // Only a stay whose cut part reaches into the time span is worth a word.
            if end.min(now) > from {
                out.capped += 1;
            }
            end = s.start + CAP_SECS / 86400.0;
        }
        stay_end.insert(s.visit_id, end);
        let a = s.start.max(from);
        let b = end.min(now);
        if b <= a {
            continue;
        }
        let whole = ((b - a) * 86400.0).round() as i64;
        if whole > out.longest.as_ref().map_or(0, |l| l.seconds) {
            out.longest = Some(StatStay { visit_id: s.visit_id, world_id: s.world_id.clone(), world_name: s.world_name.clone(), joined_at: s.joined_at.clone(), seconds: whole });
        }
        // Two records of the same time (one from VRCX, one from this app) count once.
        let piece_from = a.max(reach);
        reach = reach.max(b);
        if b <= piece_from {
            continue;
        }
        let secs = ((b - piece_from) * 86400.0).round() as i64;
        out.stays += 1;
        out.total_seconds += secs;
        spread(piece_from, b, &mut days, &mut out.hours, &mut out.weekdays);
        let key = if s.world_id.is_empty() { format!("name:{}", s.world_name) } else { s.world_id.clone() };
        let w = worlds.entry(key).or_insert_with(|| StatWorld { world_id: s.world_id.clone(), ..StatWorld::default() });
        if !s.world_name.is_empty() {
            // Stays come oldest first, so the newest name the world had wins.
            w.name = s.world_name.clone();
        }
        w.times += 1;
        w.seconds += secs;
    }
    out.play_days = days.values().filter(|s| **s >= 60).count() as i64;
    out.days = days.into_iter().collect();
    out.worlds_count = worlds.len() as i64;
    let mut worlds: Vec<StatWorld> = worlds.into_values().collect();
    worlds.sort_by(|a, b| b.seconds.cmp(&a.seconds).then_with(|| a.name.cmp(&b.name)));
    worlds.truncate(10);
    out.worlds = worlds;

    // People: rows of the same person that overlap count once, also across two records of the
    // same stay, and nobody counts past the end of my own stay. A stay counts toward "times"
    // when it adds time that was not counted yet.
    let mut people: HashMap<String, (StatPerson, i64, f64)> = HashMap::new();
    for r in sights {
        let Some(end_of_stay) = stay_end.get(&r.visit_id).copied() else { continue };
        let a = r.start.max(from);
        let b = r.end.min(end_of_stay).min(now);
        let entry = people.entry(r.key.clone()).or_insert_with(|| {
            (StatPerson { user_id: r.user_id.clone(), friend: r.friend, ..StatPerson::default() }, i64::MIN, f64::MIN)
        });
        // Rows come sorted by person, then by start time.
        entry.0.name = r.name.clone();
        let from_here = a.max(entry.2);
        if b > from_here {
            entry.0.seconds += ((b - from_here) * 86400.0).round() as i64;
            if entry.1 != r.visit_id && a >= entry.2 {
                entry.0.times += 1;
                entry.1 = r.visit_id;
            }
            entry.2 = entry.2.max(b);
        }
    }
    let mut people: Vec<StatPerson> = people.into_values().map(|p| p.0).filter(|p| p.seconds > 0).collect();
    out.people_count = people.len() as i64;
    people.sort_by(|a, b| b.seconds.cmp(&a.seconds).then_with(|| a.name.cmp(&b.name)));
    let friends: Vec<StatPerson> = people.iter().filter(|p| p.friend).take(12).cloned().collect();
    let others: Vec<StatPerson> = people.iter().filter(|p| !p.friend).take(6).cloned().collect();
    out.people = friends.into_iter().chain(others).collect();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jd(text: &str) -> f64 {
        let t = DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc);
        t.timestamp_millis() as f64 / 86_400_000.0 + 2_440_587.5
    }

    fn stay(id: i64, world: &str, from: &str, to: Option<&str>) -> StayRow {
        StayRow { visit_id: id, world_id: world.into(), world_name: format!("World {world}"), joined_at: from.into(), start: jd(from), left: to.map(jd), last_seen: None }
    }

    #[test]
    fn counts_time_once_and_cuts_open_stays() {
        let now = jd("2026-01-10T12:00:00Z");
        let from = jd("2026-01-01T00:00:00Z");
        let stays = vec![
            stay(1, "wrld_a", "2026-01-05T20:00:00Z", Some("2026-01-05T21:00:00Z")),
            // The same hour again, as an import and the app's own log both kept it.
            stay(2, "wrld_a", "2026-01-05T20:00:00Z", Some("2026-01-05T21:00:00Z")),
            stay(3, "wrld_b", "2026-01-06T10:00:00Z", Some("2026-01-06T10:30:00Z")),
            // The game crashed and the app never saw it close; it ends at its last sight.
            StayRow { last_seen: Some(jd("2026-01-07T10:15:00Z")), ..stay(4, "wrld_b", "2026-01-07T10:00:00Z", None) },
        ];
        let sights = vec![
            SightRow { key: "usr_pip".into(), user_id: "usr_pip".into(), name: "Pip".into(), visit_id: 1, start: jd("2026-01-05T20:10:00Z"), end: jd("2026-01-05T20:40:00Z"), friend: true },
            // A doubled log line for the same time: counted once.
            SightRow { key: "usr_pip".into(), user_id: "usr_pip".into(), name: "Pip".into(), visit_id: 1, start: jd("2026-01-05T20:20:00Z"), end: jd("2026-01-05T20:40:00Z"), friend: true },
            SightRow { key: " Bo".into(), user_id: String::new(), name: "Bo".into(), visit_id: 3, start: jd("2026-01-06T10:00:00Z"), end: jd("2026-01-06T12:00:00Z"), friend: false },
        ];
        let s = work_out(from, now, &stays, &sights, false);
        assert_eq!(s.total_seconds, 3600 + 1800 + 900);
        assert_eq!(s.stays, 3);
        assert!(!s.open_now);
        assert_eq!(s.worlds[0].world_id, "wrld_a");
        assert_eq!(s.worlds[0].seconds, 3600);
        assert_eq!(s.hours.iter().sum::<i64>(), s.total_seconds);
        assert_eq!(s.weekdays.iter().sum::<i64>(), s.total_seconds);
        assert_eq!(s.days.iter().map(|d| d.1).sum::<i64>(), s.total_seconds);
        let pip = s.people.iter().find(|p| p.user_id == "usr_pip").unwrap();
        assert_eq!((pip.seconds, pip.times), (1800, 1));
        // Bo stayed past my own stay; only my half hour counts.
        assert_eq!(s.people.iter().find(|p| p.name == "Bo").unwrap().seconds, 1800);
        assert_eq!(s.longest.as_ref().unwrap().seconds, 3600);
    }

    #[test]
    fn a_person_in_both_records_of_one_stay_counts_once() {
        let now = jd("2026-01-10T12:00:00Z");
        let stays = vec![
            stay(1, "wrld_a", "2026-01-05T20:00:00Z", Some("2026-01-05T21:00:00Z")),
            stay(2, "wrld_a", "2026-01-05T20:00:20Z", Some("2026-01-05T21:00:00Z")),
            stay(3, "wrld_b", "2026-01-06T20:00:00Z", Some("2026-01-06T20:30:00Z")),
        ];
        let pip = |visit: i64, from: &str, to: &str| SightRow { key: "usr_pip".into(), user_id: "usr_pip".into(), name: "Pip".into(), visit_id: visit, start: jd(from), end: jd(to), friend: true };
        let sights = vec![
            pip(1, "2026-01-05T20:00:00Z", "2026-01-05T21:00:00Z"),
            pip(2, "2026-01-05T20:00:20Z", "2026-01-05T21:00:00Z"),
            pip(3, "2026-01-06T20:00:00Z", "2026-01-06T20:30:00Z"),
        ];
        let s = work_out(jd("2026-01-01T00:00:00Z"), now, &stays, &sights, false);
        let p = &s.people[0];
        assert_eq!((p.seconds, p.times), (3600 + 1800, 2));
        assert!(p.seconds <= s.total_seconds);
    }

    #[test]
    fn the_cut_is_only_mentioned_for_stays_in_the_span() {
        let now = jd("2026-01-10T12:00:00Z");
        let old = vec![StayRow { last_seen: Some(jd("2025-06-02T18:00:00Z")), ..stay(1, "wrld_a", "2025-06-01T10:00:00Z", None) }];
        let s = work_out(jd("2026-01-04T00:00:00Z"), now, &old, &[], false);
        assert_eq!((s.capped, s.total_seconds), (0, 0));
    }

    #[test]
    fn an_open_stay_counts_to_now_only_while_the_game_runs_and_never_past_a_day() {
        let now = jd("2026-01-10T12:00:00Z");
        let open = vec![stay(1, "wrld_a", "2026-01-10T11:00:00Z", None)];
        let s = work_out(jd("2026-01-01T00:00:00Z"), now, &open, &[], true);
        assert!(s.open_now);
        assert_eq!(s.total_seconds, 3600);
        let old = vec![stay(1, "wrld_a", "2026-01-02T11:00:00Z", Some("2026-01-05T11:00:00Z"))];
        let s = work_out(jd("2026-01-01T00:00:00Z"), now, &old, &[], false);
        assert_eq!((s.total_seconds, s.capped), (24 * 3600, 1));
    }

    #[test]
    fn a_range_cuts_a_stay_that_started_before_it() {
        let now = jd("2026-01-10T12:00:00Z");
        let stays = vec![stay(1, "wrld_a", "2026-01-09T23:00:00Z", Some("2026-01-10T01:00:00Z"))];
        let s = work_out(jd("2026-01-10T00:00:00Z"), now, &stays, &[], false);
        assert_eq!(s.total_seconds, 3600);
    }
}
