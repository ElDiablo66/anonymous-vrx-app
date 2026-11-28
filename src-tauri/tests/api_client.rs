//! The API client against a local stand-in server. No request leaves the machine
//! and every account detail here is invented.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use anonymous_vrx_lib::error::AppError;
use anonymous_vrx_lib::hub::Hub;
use anonymous_vrx_lib::vrchat::client::{ApiClient, LoginOutcome, Session, Timing};
use wiremock::matchers::{body_json, header, header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn quick() -> Timing {
    Timing {
        gap: Duration::from_millis(1),
        backoff: Duration::from_millis(20),
        max_wait: Duration::from_millis(200),
        attempts: 3,
        action_wait: Duration::from_millis(100),
        action_hold: Duration::from_secs(60),
    }
}

fn client(server: &MockServer) -> ApiClient {
    ApiClient::with_base(&server.uri(), quick()).unwrap()
}

fn user() -> Value {
    json!({
        "id": "usr_me", "displayName": "Tester", "status": "active",
        "onlineFriends": ["usr_1"], "activeFriends": ["usr_2"], "offlineFriends": ["usr_3"]
    })
}

#[tokio::test]
async fn login_without_code_stores_cookie() {
    let server = MockServer::start().await;
    // "tester:p@ss word" with the password percent-encoded, then base64.
    Mock::given(method("GET"))
        .and(path("/auth/user"))
        .and(header("authorization", "Basic dGVzdGVyOnAlNDBzcyUyMHdvcmQ="))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("set-cookie", "auth=authcookie_test; Path=/; HttpOnly")
                .set_body_json(user()),
        )
        .mount(&server)
        .await;
    let api = client(&server);
    let outcome = api.login("tester", "p@ss word").await.unwrap();
    assert!(matches!(outcome, LoginOutcome::SignedIn(u) if u["id"] == "usr_me"));
    assert_eq!(api.auth_token().as_deref(), Some("authcookie_test"));
}

#[tokio::test]
async fn login_sends_a_descriptive_user_agent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/auth/user"))
        .and(header_exists("user-agent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(user()))
        .mount(&server)
        .await;
    let api = client(&server);
    api.login("tester", "x").await.unwrap();
    let seen = &server.received_requests().await.unwrap()[0];
    let agent = seen.headers.get("user-agent").unwrap().to_str().unwrap();
    assert!(agent.starts_with("anonymous-vrx/"), "got {agent}");
    assert!(!agent.contains("reqwest"));
}

#[tokio::test]
async fn login_with_code_then_verify() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/auth/user"))
        .and(header_exists("authorization"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("set-cookie", "auth=authcookie_pending; Path=/")
                .set_body_json(json!({ "requiresTwoFactorAuth": ["totp", "otp"] })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/auth/twofactorauth/totp/verify"))
        .and(header("cookie", "auth=authcookie_pending"))
        .respond_with(|req: &Request| {
            let body: Value = serde_json::from_slice(&req.body).unwrap();
            if body["code"] == "123456" {
                ResponseTemplate::new(200)
                    .insert_header("set-cookie", "twoFactorAuth=remember_me; Path=/")
                    .set_body_json(json!({ "verified": true }))
            } else {
                ResponseTemplate::new(400)
                    .set_body_json(json!({ "error": { "message": "Invalid code", "status_code": 400 } }))
            }
        })
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/auth/user"))
        .and(header("cookie", "auth=authcookie_pending; twoFactorAuth=remember_me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(user()))
        .mount(&server)
        .await;

    let api = client(&server);
    match api.login("tester", "x").await.unwrap() {
        LoginOutcome::NeedsCode(methods) => assert_eq!(methods, ["totp", "otp"]),
        _ => panic!("expected a code request"),
    }
    assert!(matches!(api.verify_code("totp", "000000").await, Err(AppError::BadCode)));
    api.verify_code("totp", " 123456 ").await.unwrap();
    assert_eq!(api.session().two_factor.as_deref(), Some("remember_me"));
    assert_eq!(api.current_user().await.unwrap()["id"], "usr_me");
}

#[tokio::test]
async fn email_code_uses_its_own_path() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/twofactorauth/emailotp/verify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "verified": true })))
        .expect(1)
        .mount(&server)
        .await;
    let api = client(&server);
    api.verify_code("emailOtp", "654321").await.unwrap();
    assert!(matches!(api.verify_code("carrier-pigeon", "1").await, Err(AppError::Parse(_))));
}

#[tokio::test]
async fn wrong_password_is_unauthorized_and_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/auth/user"))
        .respond_with(ResponseTemplate::new(401).set_body_json(
            json!({ "error": { "message": "\"Invalid Username/Email or Password\"", "status_code": 401 } }),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let api = client(&server);
    assert!(matches!(api.login("tester", "nope").await, Err(AppError::Unauthorized)));
}

#[tokio::test]
async fn expired_cookie_is_unauthorized() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/auth/user"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({ "error": { "message": "Missing Credentials" } })))
        .mount(&server)
        .await;
    let api = client(&server);
    assert!(matches!(api.current_user().await, Err(AppError::NotSignedIn)));
    api.set_session(Session { auth: Some("authcookie_old".into()), two_factor: None });
    assert!(matches!(api.current_user().await, Err(AppError::Unauthorized)));
}

#[tokio::test]
async fn waits_after_429_then_succeeds() {
    let server = MockServer::start().await;
    let hits = Arc::new(Mutex::new(0u32));
    let counter = hits.clone();
    Mock::given(method("GET"))
        .and(path("/worlds/wrld_a"))
        .respond_with(move |_: &Request| {
            let mut n = counter.lock().unwrap();
            *n += 1;
            if *n == 1 {
                ResponseTemplate::new(429)
            } else {
                ResponseTemplate::new(200).set_body_json(json!({ "id": "wrld_a", "name": "Quiet Pier" }))
            }
        })
        .mount(&server)
        .await;
    let api = client(&server);
    let started = std::time::Instant::now();
    let world = api.world("wrld_a").await.unwrap();
    assert_eq!(world["name"], "Quiet Pier");
    assert_eq!(*hits.lock().unwrap(), 2);
    assert!(started.elapsed() >= Duration::from_millis(20));
}

#[tokio::test]
async fn gives_up_after_repeated_429() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/worlds/wrld_a"))
        .respond_with(ResponseTemplate::new(429))
        .expect(3)
        .mount(&server)
        .await;
    let api = client(&server);
    assert!(matches!(api.world("wrld_a").await, Err(AppError::RateLimited(_))));
}

#[tokio::test]
async fn a_boop_vrchat_turns_down_is_sent_once_says_how_long_and_holds_nothing_else() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/usr_ash/boop"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "45"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/users/usr_kit/boop"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": { "message": "Booped" } })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/worlds/wrld_a"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "wrld_a", "name": "Quiet Pier" })))
        .expect(1)
        .mount(&server)
        .await;
    let api = client(&server);
    let started = std::time::Instant::now();
    let first = api.boop("usr_ash", None).await;
    assert!(matches!(first, Err(AppError::RateLimited(Some(45)))), "{first:?}");
    assert!(first.unwrap_err().to_string().contains("45 seconds"));
    // Asking again inside the wait is answered here, without a second request.
    assert!(matches!(api.boop("usr_ash", None).await, Err(AppError::RateLimited(Some(s))) if s > 40 && s <= 45));
    // Another friend and a read go through at once.
    api.boop("usr_kit", None).await.unwrap();
    assert_eq!(api.world("wrld_a").await.unwrap()["name"], "Quiet Pier");
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn avatar_switches_share_one_hold_and_are_never_sent_again_by_themselves() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/avatars/avtr_a/select"))
        .respond_with(ResponseTemplate::new(429))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/avatars/avtr_b/select"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "usr_me" })))
        .expect(0)
        .mount(&server)
        .await;
    let api = client(&server);
    // No Retry-After: the action's own hold of a minute is used.
    assert!(matches!(api.select_avatar("avtr_a").await, Err(AppError::RateLimited(Some(60)))));
    assert!(matches!(api.select_avatar("avtr_b").await, Err(AppError::RateLimited(Some(_)))));
}

#[tokio::test]
async fn an_action_behind_a_long_slowdown_says_so_instead_of_waiting() {
    let server = MockServer::start().await;
    // A read meets a slowdown of three seconds. Its first answer shuts the gate for that long and
    // its second try waits it out.
    Mock::given(method("GET"))
        .and(path("/worlds/wrld_a"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "3"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/users/usr_ash/boop"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let api = ApiClient::with_base(&server.uri(), Timing { attempts: 2, max_wait: Duration::from_secs(3), ..quick() }).unwrap();
    let reading = api.world("wrld_a");
    let boop = async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let started = std::time::Instant::now();
        let got = api.boop("usr_ash", None).await;
        (got, started.elapsed())
    };
    let (_, (got, took)) = tokio::join!(reading, boop);
    assert!(matches!(got, Err(AppError::RateLimited(Some(s))) if (2..=3).contains(&s)), "{got:?}");
    assert!(took < Duration::from_millis(500));
}

fn page(from: usize, count: usize, location: &str) -> Value {
    Value::Array(
        (from..from + count)
            .map(|i| json!({ "id": format!("usr_{i}"), "displayName": format!("Friend {i}"), "location": location, "status": "active" }))
            .collect(),
    )
}

async fn mount_friends(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/auth/user/friends"))
        .and(query_param("offline", "false"))
        .and(query_param("offset", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(0, 100, "wrld_a:1~region(eu)")))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/auth/user/friends"))
        .and(query_param("offline", "false"))
        .and(query_param("offset", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(100, 5, "offline")))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/auth/user/friends"))
        .and(query_param("offline", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(200, 7, "offline")))
        .mount(server)
        .await;
}

#[tokio::test]
async fn friends_are_fetched_page_by_page() {
    let server = MockServer::start().await;
    mount_friends(&server).await;
    let api = client(&server);
    assert_eq!(api.friends(false).await.unwrap().len(), 105);
    assert_eq!(api.friends(true).await.unwrap().len(), 7);
}

#[tokio::test]
async fn hub_loads_favourite_friends_with_group_names() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/favorites"))
        .and(query_param("type", "friend"))
        .and(query_param("offset", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": "fvrt_1", "type": "friend", "favoriteId": "usr_1", "tags": ["group_0"] },
            { "id": "fvrt_2", "type": "friend", "favoriteId": "usr_7", "tags": ["group_2"] }
        ])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/favorite/groups"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "type": "friend", "name": "group_2", "displayName": "Night owls" },
            { "type": "avatar", "name": "avatars1", "displayName": "Mine" }
        ])))
        .expect(1)
        .mount(&server)
        .await;
    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    let events: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = events.clone();
    let hub = Hub::new(
        api,
        std::env::temp_dir(),
        Arc::new(move |name, _| sink.lock().unwrap().push(name.to_string())),
        false,
        None,
    );
    hub.sync_favorites().await.unwrap();
    let f = hub.favorites();
    assert_eq!(f.friends["usr_7"], "group_2");
    assert_eq!(f.groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Favourites 1", "Night owls"]);
    assert!(events.lock().unwrap().contains(&"favorites:reset".to_string()));
}

#[tokio::test]
async fn hub_lists_notifications_and_acts_on_them() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/auth/user/notifications"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": "frq_1", "type": "friendRequest", "senderUserId": "usr_5", "senderUsername": "Rook",
              "message": "", "created_at": "2026-01-05T20:00:00.000Z" },
            { "id": "not_2", "type": "requestInvite", "senderUserId": "usr_6", "senderUsername": "Pip",
              "message": "", "created_at": "2026-01-05T21:00:00.000Z", "details": "{}" }
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/notifications"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": "not_g", "type": "group.invite", "title": "Group invite", "message": "Night Owls invited you",
              "createdAt": "2026-01-05T22:00:00.000Z",
              "responses": [ { "type": "accept", "data": "grp_1", "text": "Accept" } ] }
        ])))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/auth/user/notifications/frq_1/accept"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": { "message": "ok", "status_code": 200 } })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/notifications/not_g/respond"))
        .respond_with(|req: &Request| {
            let body: Value = serde_json::from_slice(&req.body).unwrap();
            assert_eq!(body["responseType"], "accept");
            assert_eq!(body["responseData"], "grp_1");
            ResponseTemplate::new(200).set_body_json(json!({}))
        })
        .expect(1)
        .mount(&server)
        .await;

    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    let events: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = events.clone();
    let hub = Hub::new(
        api,
        std::env::temp_dir(),
        Arc::new(move |name, _| sink.lock().unwrap().push(name.to_string())),
        false,
        None,
    );
    hub.sync_notices(false).await.unwrap();
    assert!(!events.lock().unwrap().contains(&"popup".to_string()), "the first load at sign-in pops nothing up");
    let ids: Vec<String> = hub.notices().iter().map(|n| n.id.clone()).collect();
    assert_eq!(ids, ["not_g", "not_2", "frq_1"], "newest first");

    hub.notice_action("frq_1", "accept", "", "").await.unwrap();
    hub.notice_action("not_g", "reply", "accept", "grp_1").await.unwrap();
    // A reply the notification never offered is refused before any request is made.
    assert!(matches!(hub.notice_action("not_2", "reply", "accept", "x").await, Err(AppError::CannotDo(_))));
    // Not signed in to a history file here, so there is no current world to invite to.
    assert!(hub.notice_action("not_2", "inviteBack", "", "").await.is_err());
    assert!(matches!(hub.notice_action("frq_1", "accept", "", "").await, Err(AppError::CannotDo(_))));
    assert_eq!(hub.notices().len(), 1);

    let frame = json!({ "type": "notification", "content": json!({ "id": "not_9", "type": "invite",
        "senderUserId": "usr_1", "senderUsername": "Mossy", "message": "", "created_at": "2026-01-06T00:00:00.000Z",
        "details": { "worldId": "wrld_a:1", "worldName": "Quiet Pier" } }).to_string() }).to_string();
    assert!(!hub.handle_frame(&frame));
    assert_eq!(hub.notices()[0].world_name, "Quiet Pier");
    assert!(events.lock().unwrap().contains(&"notice:new".to_string()));
    assert!(events.lock().unwrap().contains(&"popup".to_string()), "a live invite pops up");

    // A boop with boop pop-ups switched off shows in the window but makes no pop-up.
    hub.set_alerts(anonymous_vrx_lib::hub::Alerts { invites: true, friend_requests: true, boops: false, everything_else: true });
    events.lock().unwrap().clear();
    let boop = json!({ "type": "notification-v2", "content": json!({ "id": "not_b", "type": "boop", "senderUserId": "usr_7",
        "senderUsername": "Ash", "title": "Boop!", "message": "", "createdAt": "2026-01-06T00:01:00.000Z" }).to_string() }).to_string();
    assert!(!hub.handle_frame(&boop));
    assert!(events.lock().unwrap().contains(&"notice:new".to_string()));
    assert!(!events.lock().unwrap().contains(&"popup".to_string()));
    // The game shows it and VRChat clears it: it leaves the waiting list.
    let cleared = json!({ "type": "notification-v2-delete", "content": json!({ "ids": ["not_b"], "version": 2 }).to_string() }).to_string();
    assert!(!hub.handle_frame(&cleared));
    assert!(hub.notices().iter().all(|n| n.id != "not_b"));
}

#[tokio::test]
async fn instance_count_and_invites_use_the_right_addresses() {
    let server = MockServer::start().await;
    let location = "wrld_a:123~hidden(usr_9)~region(eu)";
    Mock::given(method("GET"))
        .and(path("/instances/wrld_a:123~hidden(usr_9)~region(eu)"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "n_users": 14, "capacity": 32, "full": false })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/invite/myself/to/wrld_a:123~hidden(usr_9)~region(eu)"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/requestInvite/usr_0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    mount_friends(&server).await;

    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    assert_eq!(api.instance(location).await.unwrap()["n_users"], 14);

    let hub = Hub::new(api, std::env::temp_dir(), Arc::new(|_, _| {}), false, None);
    hub.invite_myself(location).await.unwrap();
    assert!(matches!(hub.invite_myself("private").await, Err(AppError::CannotDo(_))));
    // Only people on the friend list can be asked.
    assert!(matches!(hub.request_invite("usr_0", None).await, Err(AppError::CannotDo(_))));
    let me = json!({ "id": "usr_me", "onlineFriends": ["usr_0"], "activeFriends": [], "offlineFriends": [] });
    hub.sync_friends(Some(me)).await.unwrap();
    hub.request_invite("usr_0", None).await.unwrap();
}

#[tokio::test]
async fn avatar_search_sends_no_account_details_to_the_third_party() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v3/avatar/search/vrcx"))
        .and(query_param("search", "robot"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": "avtr_1", "name": "Robot", "authorName": "Rook", "authorId": "usr_5" },
            { "id": "not an avatar" }
        ])))
        .mount(&server)
        .await;
    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_secret".into()), two_factor: Some("remember_me".into()) });
    // Only https addresses are accepted for the provider.
    assert!(matches!(api.search_avatars("http://plain.example.test/x", "robot").await, Err(AppError::CannotDo(_))));
    assert!(matches!(api.search_avatars("https://x.example.test/a?b=1", "robot").await, Err(AppError::CannotDo(_))));
    let provider = format!("{}/v3/avatar/search/vrcx", server.uri());
    let found = api.search_avatars(&provider, "robot").await.unwrap();
    assert_eq!(found.len(), 2);
    let seen = &server.received_requests().await.unwrap()[0];
    assert!(seen.headers.get("cookie").is_none(), "the VRChat cookie must never reach a third party");
    assert!(seen.headers.get("authorization").is_none());
}

#[tokio::test]
async fn profile_parts_use_the_right_addresses() {
    let server = MockServer::start().await;
    for (p, body) in [
        ("/users/usr_1/mutuals/friends", json!([{ "id": "usr_9", "displayName": "Juno" }])),
        ("/users/usr_1/groups", json!([{ "groupId": "grp_1", "name": "Night Owls", "memberCount": 12 }])),
        ("/users/usr_1/groups/represented", json!({ "groupId": "grp_1", "name": "Night Owls" })),
        ("/worlds", json!([{ "id": "wrld_a", "name": "Quiet Pier" }])),
    ] {
        Mock::given(method("GET")).and(path(p)).respond_with(ResponseTemplate::new(200).set_body_json(body)).mount(&server).await;
    }
    Mock::given(method("GET"))
        .and(path("/db"))
        .and(query_param("authorId", "usr_1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "id": "avtr_1", "name": "Robot" }])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/db"))
        .and(query_param("fileId", "file_abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "avtr_2", "name": "Fox" })))
        .mount(&server)
        .await;
    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_secret".into()), two_factor: None });
    let hub = Hub::new(api, std::env::temp_dir(), Arc::new(|_, _| {}), false, None);
    let db = format!("{}/db", server.uri());
    assert_eq!(hub.profile_part("usr_1", "mutualFriends", "", "").await.unwrap()[0]["displayName"], "Juno");
    assert_eq!(hub.profile_part("usr_1", "groups", "", "").await.unwrap()[0]["memberCount"], 12);
    assert_eq!(hub.profile_part("usr_1", "represented", "", "").await.unwrap()["name"], "Night Owls");
    assert_eq!(hub.profile_part("usr_1", "worlds", "", "").await.unwrap()[0]["name"], "Quiet Pier");
    assert_eq!(hub.profile_part("usr_1", "avatars", "", &db).await.unwrap()[0]["name"], "Robot");
    assert_eq!(hub.profile_part("usr_1", "wearing", "file_abc", &db).await.unwrap()["name"], "Fox");
    assert!(hub.profile_part("usr_1", "wearing", "something", &db).await.unwrap().is_null());
    assert!(matches!(hub.profile_part("usr_1", "passwords", "", "").await, Err(AppError::CannotDo(_))));
    // Nothing sent to the avatar database carried the VRChat cookie.
    for req in server.received_requests().await.unwrap() {
        if req.url.path() == "/db" {
            assert!(req.headers.get("cookie").is_none());
        }
    }
}

#[tokio::test]
async fn favourites_add_list_and_remove() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/favorites"))
        .respond_with(|req: &Request| {
            let body: Value = serde_json::from_slice(&req.body).unwrap();
            assert_eq!(body, json!({ "type": "avatar", "favoriteId": "avtr_1", "tags": ["avatars1"] }));
            ResponseTemplate::new(200).set_body_json(json!({ "id": "fvrt_new" }))
        })
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/favorite/groups")).respond_with(ResponseTemplate::new(200).set_body_json(json!([
        { "type": "avatar", "name": "avatars1", "displayName": "Main" }, { "type": "world", "name": "worlds1", "displayName": "W" }
    ]))).mount(&server).await;
    Mock::given(method("GET")).and(path("/favorites")).and(query_param("type", "avatar")).respond_with(ResponseTemplate::new(200).set_body_json(json!([
        { "id": "fvrt_1", "favoriteId": "avtr_1", "tags": ["avatars1"] }, { "id": "fvrt_2", "favoriteId": "avtr_gone", "tags": ["avatars1"] }
    ]))).mount(&server).await;
    // Items are only handed out per favourite group, so the group tag must be asked for.
    Mock::given(method("GET")).and(path("/avatars/favorites")).and(query_param("tag", "avatars1")).respond_with(ResponseTemplate::new(200).set_body_json(json!([
        { "id": "avtr_1", "name": "Robot", "releaseStatus": "public" }
    ]))).mount(&server).await;
    Mock::given(method("DELETE")).and(path("/favorites/fvrt_2")).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&server).await;

    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    let hub = Hub::new(api, std::env::temp_dir(), Arc::new(|_, _| {}), false, None);
    hub.add_favorite("avatar", "avtr_1", "avatars1").await.unwrap();
    assert!(matches!(hub.add_favorite("avatar", "wrld_1", "avatars1").await, Err(AppError::CannotDo(_))));
    assert!(hub.favorites_cached("avatar").is_none());
    let all = hub.favorites_of("avatar").await.unwrap();
    assert_eq!(all["capacity"], json!(50), "the usual limit when VRChat names none");
    assert_eq!(hub.favorites_cached("avatar").unwrap(), all);
    assert_eq!(all["groups"].as_array().unwrap().len(), 1);
    assert_eq!(all["entries"].as_array().unwrap().len(), 2);
    assert_eq!(all["items"].as_array().unwrap().len(), 1);
    // Ids that are not favourite entries are skipped, never sent.
    let removed = hub.remove_favorites(&["fvrt_2".to_string(), "avtr_1".to_string()]).await.unwrap();
    assert_eq!(removed, 1);
    assert!(hub.favorites_cached("avatar").is_none(), "a change drops the remembered answer");
    assert!(matches!(hub.group_action("usr_1", "join").await, Err(AppError::CannotDo(_))));
    assert!(matches!(hub.group_action("grp_1", "burn").await, Err(AppError::CannotDo(_))));
    assert!(matches!(hub.thing("groupPhotos", "grp_1").await, Err(AppError::CannotDo(_))));
    assert!(matches!(hub.thing("password", "x").await, Err(AppError::CannotDo(_))));
}

#[tokio::test]
async fn clean_up_only_names_what_vrchat_confirms_gone() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/favorite/groups")).respond_with(ResponseTemplate::new(200).set_body_json(json!([]))).mount(&server).await;
    Mock::given(method("GET")).and(path("/favorites")).respond_with(ResponseTemplate::new(200).set_body_json(json!([
        { "id": "fvrt_1", "favoriteId": "avtr_listed", "tags": ["avatars1"] },
        { "id": "fvrt_2", "favoriteId": "avtr_fine", "tags": ["avatars2"] },
        { "id": "fvrt_3", "favoriteId": "avtr_deleted", "tags": ["avatars2"] },
        { "id": "fvrt_4", "favoriteId": "avtr_private", "tags": ["avatars2"] },
        { "id": "fvrt_5", "favoriteId": "avtr_broken", "tags": ["avatars2"] }
    ]))).mount(&server).await;
    Mock::given(method("GET")).and(path("/avatars/favorites")).and(query_param("tag", "avatars1")).respond_with(ResponseTemplate::new(200).set_body_json(json!([
        { "id": "avtr_listed", "name": "Robot", "releaseStatus": "public", "authorId": "usr_a" }
    ]))).mount(&server).await;
    // The second group fails to load: nothing in it may be called gone because of that.
    Mock::given(method("GET")).and(path("/avatars/favorites")).and(query_param("tag", "avatars2")).respond_with(ResponseTemplate::new(500).set_body_json(json!({}))).mount(&server).await;
    Mock::given(method("GET")).and(path("/avatars/avtr_fine")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "avtr_fine", "name": "Fox", "releaseStatus": "public", "authorId": "usr_a" }))).mount(&server).await;
    Mock::given(method("GET")).and(path("/avatars/avtr_deleted")).respond_with(ResponseTemplate::new(404).set_body_json(json!({ "error": { "message": "Avatar not found", "status_code": 404 } }))).mount(&server).await;
    Mock::given(method("GET")).and(path("/avatars/avtr_private")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "avtr_private", "name": "Owl", "releaseStatus": "private", "authorId": "usr_a" }))).mount(&server).await;
    Mock::given(method("GET")).and(path("/avatars/avtr_broken")).respond_with(ResponseTemplate::new(500).set_body_json(json!({}))).mount(&server).await;

    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    let hub = Hub::new(api, std::env::temp_dir(), Arc::new(|_, _| {}), false, None);
    let all = hub.favorites_of("avatar").await.unwrap();
    assert_eq!(all["complete"], json!(false));
    let found = hub.check_favorites("avatar").await.unwrap();
    let dead: Vec<&str> = found["dead"].as_array().unwrap().iter().map(|d| d["entryId"].as_str().unwrap()).collect();
    assert_eq!(dead, vec!["fvrt_3", "fvrt_4"]);
    assert_eq!(found["checked"], json!(4));
    assert!(matches!(hub.check_favorites("friend").await, Err(AppError::CannotDo(_))));
}

#[tokio::test]
async fn status_change_is_checked_and_service_status_is_read() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/status.json")).respond_with(|req: &Request| {
        assert!(req.headers.get("cookie").is_none(), "the status page gets no cookie");
        ResponseTemplate::new(200).set_body_json(json!({ "status": { "indicator": "minor", "description": "Partial outage" } }))
    }).mount(&server).await;
    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    let hub = Hub::new(api, std::env::temp_dir(), Arc::new(|_, _| {}), false, None);
    assert!(matches!(hub.set_status("party", "").await, Err(AppError::CannotDo(_))));
    assert!(matches!(hub.set_status("busy", "this text is far too long for vrchat to take").await, Err(AppError::CannotDo(_))));
    assert!(matches!(hub.set_status("busy", "brb").await, Err(AppError::NotSignedIn)));
    let status = hub.service_status().await.unwrap();
    assert_eq!(status, json!({ "indicator": "minor", "description": "Partial outage" }));
    // Nothing on this PC before sign-in.
    assert!(matches!(hub.avatar_history(10), Err(AppError::NotSignedIn)));
    assert!(hub.add_local_favorite(&serde_json::from_value(json!({ "kind": "avatar", "itemId": "usr_1", "name": "", "authorName": "", "image": "" })).unwrap()).is_err());
}


#[tokio::test]
async fn my_uploads_can_be_changed_and_deleted_but_nobody_elses() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/avatars/avtr_mine")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "avtr_mine", "authorId": "usr_me" }))).mount(&server).await;
    Mock::given(method("GET")).and(path("/avatars/avtr_theirs")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "avtr_theirs", "authorId": "usr_other" }))).mount(&server).await;
    Mock::given(method("GET")).and(path("/worlds/wrld_mine")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "wrld_mine", "authorId": "usr_me" }))).mount(&server).await;
    Mock::given(method("PUT")).and(path("/avatars/avtr_mine")).respond_with(|req: &Request| {
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(body, json!({ "releaseStatus": "private" }));
        ResponseTemplate::new(200).set_body_json(json!({}))
    }).expect(1).mount(&server).await;
    Mock::given(method("DELETE")).and(path("/avatars/avtr_mine")).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&server).await;
    Mock::given(method("PUT")).and(path("/worlds/wrld_mine/publish")).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&server).await;
    Mock::given(method("DELETE")).and(path("/worlds/wrld_mine/publish")).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&server).await;
    Mock::given(method("DELETE")).and(path("/worlds/wrld_mine")).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&server).await;
    // Nothing may ever be sent for someone else's avatar.
    Mock::given(method("PUT")).and(path("/avatars/avtr_theirs")).respond_with(ResponseTemplate::new(200)).expect(0).mount(&server).await;
    Mock::given(method("DELETE")).and(path("/avatars/avtr_theirs")).respond_with(ResponseTemplate::new(200)).expect(0).mount(&server).await;

    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    let dir = std::env::temp_dir().join(format!("anonymous-vrx-uploads-test-{}", std::process::id()));
    let hub = Hub::new(api, dir.clone(), Arc::new(|_, _| {}), false, None);
    assert!(matches!(hub.upload_action("avatar", "avtr_mine", "private").await, Err(AppError::NotSignedIn)));
    hub.begin(json!({ "id": "usr_me", "displayName": "Tester" })).unwrap();
    assert!(matches!(hub.upload_action("avatar", "avtr_theirs", "delete").await, Err(AppError::CannotDo(_))));
    assert!(matches!(hub.upload_action("avatar", "avtr_theirs", "private").await, Err(AppError::CannotDo(_))));
    assert!(matches!(hub.upload_action("avatar", "wrld_mine", "delete").await, Err(AppError::CannotDo(_))));
    assert!(matches!(hub.upload_action("avatar", "avtr_mine", "explode").await, Err(AppError::CannotDo(_))));
    hub.upload_action("avatar", "avtr_mine", "private").await.unwrap();
    hub.upload_action("world", "wrld_mine", "public").await.unwrap();
    hub.upload_action("world", "wrld_mine", "private").await.unwrap();
    hub.upload_action("avatar", "avtr_mine", "delete").await.unwrap();
    hub.upload_action("world", "wrld_mine", "delete").await.unwrap();
    hub.sign_out().await.ok();
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn blocks_and_mutes_are_listed_set_and_taken_back() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/auth/user/playermoderations")).respond_with(ResponseTemplate::new(200).set_body_json(json!([
        { "id": "pmod_1", "type": "mute", "sourceUserId": "usr_me", "targetUserId": "usr_2", "targetDisplayName": "Loud One", "created": "2026-01-02T00:00:00.000Z" },
        { "id": "pmod_2", "type": "block", "sourceUserId": "usr_me", "targetUserId": "usr_3", "targetDisplayName": "Rook", "created": "2026-03-04T00:00:00.000Z" },
        { "id": "pmod_3", "type": "showAvatar", "sourceUserId": "usr_me", "targetUserId": "usr_4", "targetDisplayName": "Pal", "created": "2026-05-06T00:00:00.000Z" }
    ]))).expect(2).mount(&server).await;
    Mock::given(method("POST")).and(path("/auth/user/playermoderations")).and(body_json(json!({ "moderated": "usr_5", "type": "block" }))).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&server).await;
    Mock::given(method("PUT")).and(path("/auth/user/unplayermoderate")).and(body_json(json!({ "moderated": "usr_2", "type": "mute" }))).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&server).await;

    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    let hub = Hub::new(api, std::env::temp_dir(), Arc::new(|_, _| {}), false, None);
    // Nothing is asked or changed before sign-in.
    assert!(matches!(hub.moderations(false).await, Err(AppError::NotSignedIn)));
    assert!(matches!(hub.set_moderation("usr_5", "block", true).await, Err(AppError::NotSignedIn)));
    hub.begin(json!({ "id": "usr_me", "displayName": "Tester" })).unwrap();

    let list = hub.moderations(false).await.unwrap();
    // Only blocks and mutes, newest first.
    assert_eq!(list, json!([
        { "kind": "block", "userId": "usr_3", "name": "Rook", "at": "2026-03-04T00:00:00.000Z" },
        { "kind": "mute", "userId": "usr_2", "name": "Loud One", "at": "2026-01-02T00:00:00.000Z" }
    ]));
    // Asked once; the second look comes from memory.
    hub.moderations(false).await.unwrap();

    assert!(matches!(hub.set_moderation("usr_me", "block", true).await, Err(AppError::CannotDo(_))));
    assert!(matches!(hub.set_moderation("avtr_1", "block", true).await, Err(AppError::CannotDo(_))));
    assert!(matches!(hub.set_moderation("usr_5", "report", true).await, Err(AppError::CannotDo(_))));
    hub.set_moderation("usr_5", "block", true).await.unwrap();
    hub.set_moderation("usr_2", "mute", false).await.unwrap();
    // A change empties the memory, so the list is asked for again.
    hub.moderations(false).await.unwrap();

    // A row's picture and rank are asked for once per person.
    Mock::given(method("GET")).and(path("/users/usr_3")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "id": "usr_3", "displayName": "Rook", "userIcon": "https://example.test/rook.png", "status": "busy", "state": "offline",
        "tags": ["system_trust_basic", "system_supporter"]
    }))).expect(1).mount(&server).await;
    let brief = hub.person_brief("usr_3").await.unwrap();
    assert_eq!((brief["displayName"].as_str(), brief["vrcPlus"].as_bool(), brief["state"].as_str()), (Some("Rook"), Some(true), Some("offline")));
    hub.person_brief("usr_3").await.unwrap();
    assert!(matches!(hub.person_brief("avtr_1").await, Err(AppError::CannotDo(_))));
}

#[tokio::test]
async fn tags_are_cleaned_and_kept_per_item() {
    let server = MockServer::start().await;
    let api = Arc::new(client(&server));
    let dir = std::env::temp_dir().join(format!("anonymous-vrx-tags-test-{}", std::process::id()));
    let hub = Hub::new(api, dir.clone(), Arc::new(|_, _| {}), false, None);
    assert!(matches!(hub.set_tags("avtr_1", &["club".to_string()]), Err(AppError::NotSignedIn)));
    hub.begin(json!({ "id": "usr_me", "displayName": "Tester" })).unwrap();
    let saved = hub.set_tags("avtr_1", &["  Club ".to_string(), "club".to_string(), "".to_string(), "dance   night".to_string()]).unwrap();
    assert_eq!(saved, vec!["Club".to_string(), "dance night".to_string()]);
    assert_eq!(hub.tags().unwrap().get("avtr_1"), Some(&vec!["Club".to_string(), "dance night".to_string()]));
    assert!(matches!(hub.set_tags("usr_1", &["x".to_string()]), Err(AppError::CannotDo(_))));
    assert!(matches!(hub.set_tags("avtr_1", &["this tag is far too long to keep".to_string()]), Err(AppError::CannotDo(_))));
    hub.set_tags("avtr_1", &[]).unwrap();
    assert!(hub.tags().unwrap().is_empty());
    hub.sign_out().await.ok();
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn calendar_is_read_once_and_events_can_be_followed() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/calendar/following")).and(query_param("n", "100")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "hasNext": false, "totalCount": 4, "results": [
            { "id": "cal_1", "ownerId": "grp_1", "ownerName": "Night Owls", "title": "Friday meet-up", "startsAt": "2026-11-27T20:00:00.000Z", "endsAt": "2026-11-27T22:00:00.000Z",
              "accessType": "public", "category": "hangout", "interestedUserCount": 12, "userInterest": { "isFollowing": true } },
            { "id": "cal_2", "ownerId": "grp_1", "title": "A draft", "isDraft": true },
            { "id": "cal_4", "ownerId": "grp_2", "title": "Pier walk", "startsAt": "2026-11-28T20:00:00.000Z", "endsAt": "2026-11-28T21:00:00.000Z" },
            { "id": "cal_3", "ownerId": "grp_1", "title": "Called off", "deletedAt": "2026-11-20T00:00:00.000Z" }
        ]
    }))).expect(2).mount(&server).await;
    // An event that comes without its group's name: the group is asked once, however often the list is read.
    Mock::given(method("GET")).and(path("/groups/grp_2")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "grp_2", "name": "Pier Regulars" }))).expect(1).mount(&server).await;
    Mock::given(method("POST")).and(path("/calendar/grp_1/cal_1/follow")).and(body_json(json!({ "isFollowing": false }))).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&server).await;

    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    let hub = Hub::new(api, std::env::temp_dir(), Arc::new(|_, _| {}), false, None);
    assert!(matches!(hub.calendar("following", "2026-11", false).await, Err(AppError::NotSignedIn)));
    hub.begin(json!({ "id": "usr_me", "displayName": "Tester" })).unwrap();
    assert!(matches!(hub.calendar("everything", "2026-11", false).await, Err(AppError::CannotDo(_))));
    assert!(matches!(hub.calendar("following", "November", false).await, Err(AppError::CannotDo(_))));

    let page = hub.calendar("following", "2026-11", false).await.unwrap();
    // Drafts and deleted events are left out.
    assert_eq!(page["events"].as_array().unwrap().len(), 2);
    assert_eq!(page["events"][1]["groupName"], "Pier Regulars");
    assert_eq!(page["events"][0]["groupName"], "Night Owls");
    assert_eq!(page["events"][0]["following"], true);
    // An event on the followed list counts as followed even when the answer does not say so.
    assert_eq!(page["events"][1]["following"], true);
    // The second look comes from memory.
    hub.calendar("following", "2026-11", false).await.unwrap();

    assert!(matches!(hub.follow_event("usr_1", "cal_1", true).await, Err(AppError::CannotDo(_))));
    assert!(matches!(hub.follow_event("grp_1", "evt_1", true).await, Err(AppError::CannotDo(_))));
    hub.follow_event("grp_1", "cal_1", false).await.unwrap();
    // A change empties the memory, so the list is asked for again.
    hub.calendar("following", "2026-11", false).await.unwrap();

    // Events across VRChat have no month: on now and coming up, one request.
    Mock::given(method("GET")).and(path("/calendar/discover")).and(query_param("scope", "all")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "nextCursor": "", "results": [
            { "id": "cal_9", "ownerId": "grp_1", "ownerName": "Night Owls", "title": "Open stage", "startsAt": "2026-11-25T20:00:00.000Z", "endsAt": "2026-11-25T23:00:00.000Z", "featured": true }
        ]
    }))).expect(1).mount(&server).await;
    let wide = hub.calendar("discover", "", false).await.unwrap();
    assert_eq!(wide["events"][0]["title"], "Open stage");
    assert_eq!(wide["more"], false);
    hub.calendar("discover", "2031-01", false).await.unwrap();
}

#[tokio::test]
async fn boop_and_picture() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/user/usr_2/friendRequest")).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&server).await;
    Mock::given(method("DELETE")).and(path("/auth/user/friends/usr_3")).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&server).await;
    // A picture on the API's own address may carry the sign-in; nothing else does.
    Mock::given(method("GET")).and(path("/file/photo")).and(header_exists("cookie")).respond_with(ResponseTemplate::new(200).insert_header("content-type", "image/jpeg").set_body_bytes(vec![9u8; 4])).mount(&server).await;
    Mock::given(method("POST")).and(path("/users/usr_1/boop")).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&server).await;
    // Another host stands in for a picture that is not on VRChat's API.
    let elsewhere = MockServer::start().await;
    Mock::given(method("GET")).and(path("/pic.png")).respond_with(|req: &Request| {
        assert!(req.headers.get("cookie").is_none(), "pictures from other hosts are fetched without the sign-in cookie");
        ResponseTemplate::new(200).insert_header("content-type", "image/png").set_body_bytes(vec![1u8, 2, 3])
    }).mount(&elsewhere).await;
    Mock::given(method("GET")).and(path("/page")).respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/html").set_body_string("hello")).mount(&server).await;

    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    let hub = Hub::new(api, std::env::temp_dir(), Arc::new(|_, _| {}), false, None);
    hub.boop("usr_1", None).await.unwrap();
    // Friend requests and group names are checked before anything is sent.
    assert!(matches!(hub.friend_action("avtr_1", "add").await, Err(AppError::CannotDo(_))));
    assert!(matches!(hub.friend_action("usr_1", "poke").await, Err(AppError::CannotDo(_))));
    assert!(matches!(hub.rename_favorite_group("avatar", "avatars1", "  ").await, Err(AppError::CannotDo(_))));
    assert!(matches!(hub.rename_favorite_group("avatar", "avatars1", "Mine").await, Err(AppError::NotSignedIn)));
    assert!(matches!(hub.boop("avtr_1", None).await, Err(AppError::CannotDo(_))));
    let (kind, bytes) = hub.picture(&format!("{}/pic.png", elsewhere.uri())).await.unwrap();
    assert_eq!((kind.as_str(), bytes.len()), ("image/png", 3));
    let (kind, bytes) = hub.picture(&format!("{}/file/photo", server.uri())).await.unwrap();
    assert_eq!((kind.as_str(), bytes.len()), ("image/jpeg", 4));
    assert!(matches!(hub.picture(&format!("{}/page", server.uri())).await, Err(AppError::CannotDo(_))));
    assert!(matches!(hub.picture("file:///c:/secret.txt").await, Err(AppError::CannotDo(_))));
    hub.friend_action("usr_2", "add").await.unwrap();
    hub.friend_action("usr_3", "remove").await.unwrap();
}

#[tokio::test]
async fn search_asks_the_right_addresses() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users"))
        .and(query_param("search", "moss y"))
        .and(query_param("n", "24"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": "usr_1", "displayName": "Mossy", "tags": ["system_trust_known"], "isFriend": true },
            { "displayName": "broken entry" }
        ])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/worlds"))
        .and(query_param("search", "pier"))
        .and(query_param("sort", "relevance"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": "wrld_a", "name": "Quiet Pier", "authorName": "Rook", "capacity": 32 }
        ])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/users/usr_1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "usr_1", "displayName": "Mossy", "bio": "pier person" })))
        .mount(&server)
        .await;
    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    assert_eq!(api.user("usr_1").await.unwrap()["bio"], "pier person");
    let hub = Hub::new(api, std::env::temp_dir(), Arc::new(|_, _| {}), false, None);
    let people = hub.search_users(" moss y ").await.unwrap();
    assert_eq!(people.len(), 1);
    assert_eq!(people[0].rank, "User");
    let worlds = hub.search_worlds("pier").await.unwrap();
    assert_eq!(worlds[0].author, "Rook");
    // Strangers cannot be invited from here.
    assert!(matches!(hub.invite_here("usr_1", None).await, Err(AppError::CannotDo(_))));
}

#[tokio::test]
async fn hub_sorts_friends_into_states_and_reports_live_changes() {
    let server = MockServer::start().await;
    mount_friends(&server).await;
    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });

    let events: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = events.clone();
    let dir = std::env::temp_dir().join(format!("anonymous-vrx-test-{}", std::process::id()));
    let hub = Hub::new(
        api,
        dir.clone(),
        Arc::new(move |name, _| sink.lock().unwrap().push(name.to_string())),
        false,
        None,
    );
    let me = json!({ "id": "usr_me", "displayName": "Tester",
        "onlineFriends": ["usr_0", "usr_1"], "activeFriends": ["usr_100"], "offlineFriends": [] });
    hub.sync_friends(Some(me)).await.unwrap();

    let friends = hub.friends();
    assert_eq!(friends.len(), 112);
    let state = |id: &str| friends.iter().find(|f| f.id == id).unwrap().state.clone();
    assert_eq!(state("usr_0"), "online");
    assert_eq!(state("usr_50"), "online");
    assert_eq!(state("usr_100"), "active");
    assert_eq!(state("usr_203"), "offline");
    assert!(events.lock().unwrap().contains(&"friends:reset".to_string()));

    let frame = json!({ "type": "friend-offline", "content": json!({ "userId": "usr_0" }).to_string() }).to_string();
    assert!(!hub.handle_frame(&frame));
    assert_eq!(hub.friends().iter().find(|f| f.id == "usr_0").unwrap().state, "offline");
    assert!(events.lock().unwrap().contains(&"friend:changed".to_string()));
    assert!(hub.handle_frame(r#"{"err":"authToken doesn't correspond with an active session"}"#));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn new_instance_is_owned_by_me_and_checked_before_asking() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/instances"))
        .and(wiremock::matchers::body_json(json!({
            "worldId": "wrld_a",
            "type": "private",
            "region": "eu",
            "ownerId": "usr_me",
            "canRequestInvite": true
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "location": "wrld_a:41872~private(usr_me)~canRequestInvite~region(eu)"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    let hub = Hub::new(api, std::env::temp_dir(), Arc::new(|_, _| {}), false, None);
    // Not signed in yet: nothing is sent.
    assert!(matches!(hub.create_instance("wrld_a", "invitePlus", "eu").await, Err(AppError::NotSignedIn)));
    hub.begin(json!({ "id": "usr_me", "displayName": "Tester" })).unwrap();
    // Wrong world, region or kind never reaches VRChat.
    assert!(matches!(hub.create_instance("wrld_a:1", "invite", "eu").await, Err(AppError::CannotDo(_))));
    assert!(matches!(hub.create_instance("wrld_a", "invite", "mars").await, Err(AppError::CannotDo(_))));
    assert!(matches!(hub.create_instance("wrld_a", "secret", "eu").await, Err(AppError::CannotDo(_))));
    let location = hub.create_instance("wrld_a", "invitePlus", "eu").await.unwrap();
    assert_eq!(location, "wrld_a:41872~private(usr_me)~canRequestInvite~region(eu)");
}

#[tokio::test]
async fn frames_and_effects_are_listed_once_by_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/cosmetics/index/iconFrame")).respond_with(ResponseTemplate::new(200).set_body_json(json!([
        { "id": "frame_fire", "metadata": { "assets": [
            { "type": "base", "url": "https://assets.example/fire.png" },
            { "type": "mainAnimation", "url": "https://assets.example/fire.webp" },
            { "type": "introAnimation", "url": "https://assets.example/fire-in.webp", "totalDurationMs": 900 }
        ] } },
        { "id": "frame_plain", "metadata": { "assets": [ { "type": "base", "url": "https://assets.example/plain.png" } ] } },
        { "id": "frame_odd", "metadata": { "assets": [ { "type": "base", "url": "javascript:alert(1)" } ] } },
        { "metadata": {} }
    ]))).expect(1).mount(&server).await;
    // One list failing does not hide the others.
    Mock::given(method("GET")).and(path("/cosmetics/index/profileEffect")).respond_with(ResponseTemplate::new(500)).expect(1).mount(&server).await;
    Mock::given(method("GET")).and(path("/cosmetics/index/nameplateEffect")).respond_with(ResponseTemplate::new(200).set_body_json(json!([
        { "id": "plate_ember", "metadata": { "gradientStart": "7a1010", "gradientEnd": "f97316", "assets": [] } }
    ]))).expect(1).mount(&server).await;

    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    let hub = Hub::new(api, std::env::temp_dir(), Arc::new(|_, _| {}), false, None);
    assert!(matches!(hub.cosmetics().await, Err(AppError::NotSignedIn)));
    hub.begin(json!({ "id": "usr_me", "displayName": "Tester" })).unwrap();

    let all = hub.cosmetics().await.unwrap();
    assert_eq!(all["frame_fire"]["picture"], "https://assets.example/fire.webp");
    assert_eq!(all["frame_fire"]["still"], "https://assets.example/fire.png");
    assert_eq!(all["frame_plain"]["picture"], "https://assets.example/plain.png");
    // Only plain web addresses are passed on to the window.
    assert_eq!(all["frame_odd"]["picture"], "");
    assert_eq!(all["plate_ember"]["from"], "7a1010");
    assert_eq!(all["plate_ember"]["to"], "f97316");
    assert_eq!(all.as_object().unwrap().len(), 4);
    // Kept: a second profile asks VRChat nothing.
    hub.cosmetics().await.unwrap();
}

#[tokio::test]
async fn friends_of_friends_are_asked_one_friend_at_a_time() {
    let server = MockServer::start().await;
    // Asked for a page at a time, with the page size named once.
    Mock::given(method("GET")).and(path("/users/usr_a/mutuals/friends")).and(query_param("n", "100")).and(query_param("offset", "0")).respond_with(ResponseTemplate::new(200).set_body_json(json!([ { "id": "usr_b" }, { "id": "usr_c" } ]))).expect(1).mount(&server).await;
    // Someone who does not let others see their friends.
    Mock::given(method("GET")).and(path("/users/usr_b/mutuals/friends")).respond_with(ResponseTemplate::new(403).set_body_json(json!({ "error": { "message": "hidden", "status_code": 403 } }))).expect(2).mount(&server).await;
    Mock::given(method("GET")).and(path("/users/usr_c/mutuals/friends")).respond_with(ResponseTemplate::new(200).set_body_json(json!([ { "id": "usr_a" } ]))).expect(1).mount(&server).await;

    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    let hub = Hub::new(api, std::env::temp_dir(), Arc::new(|_, _| {}), false, None);
    assert!(matches!(hub.fetch_mutuals(std::time::Duration::ZERO, true).await, Err(AppError::NotSignedIn)));
    hub.begin(json!({ "id": "usr_me_mutuals", "displayName": "Tester" })).unwrap();
    for id in ["usr_a", "usr_b", "usr_c"] {
        hub.put_friend_for_test(id);
    }

    let ran = hub.fetch_mutuals(std::time::Duration::ZERO, true).await.unwrap();
    assert_eq!((ran["done"].as_i64(), ran["ended"].as_str()), (Some(3), Some("done")));
    let known = hub.mutuals().unwrap();
    // a-b and a-c, each once, whoever reported it.
    assert_eq!(known["pairs"].as_array().unwrap().len(), 2);
    assert_eq!(known["hidden"], json!(["usr_b"]));
    assert_eq!(known["running"], false);
    // Asked a moment ago: a second run that is not told to ask everyone only asks the one who gave no answer.
    let again = hub.fetch_mutuals(std::time::Duration::ZERO, false).await;
    assert_eq!(again.unwrap()["total"], 1);

    // A refusal that is not about one friend ends the run and is reported, and nobody is marked as hiding.
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/users/usr_a/mutuals/friends")).respond_with(ResponseTemplate::new(400).set_body_json(json!({ "error": { "message": "bad question", "status_code": 400 } }))).expect(1).mount(&server).await;
    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    let hub = Hub::new(api, std::env::temp_dir(), Arc::new(|_, _| {}), false, None);
    hub.begin(json!({ "id": "usr_me_mutuals_two", "displayName": "Tester" })).unwrap();
    hub.put_friend_for_test("usr_a");
    hub.put_friend_for_test("usr_b");
    let ran = hub.fetch_mutuals(std::time::Duration::ZERO, true).await.unwrap();
    assert_eq!((ran["done"].as_i64(), ran["ended"].as_str()), (Some(0), Some("refused")));
    assert!(ran["problem"].as_str().unwrap().contains("bad question"));
    assert_eq!(hub.mutuals().unwrap()["hidden"], json!([]));
}

#[tokio::test]
async fn a_full_instance_is_watched_until_it_has_room() {
    let server = MockServer::start().await;
    // Full at the first look, one place free at the second.
    Mock::given(method("GET")).and(path("/instances/wrld_a:1~region(eu)")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "n_users": 0, "userCount": 16, "capacity": 16 }))).up_to_n_times(1).expect(1).mount(&server).await;
    Mock::given(method("GET")).and(path("/instances/wrld_a:1~region(eu)")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "n_users": 15, "capacity": 16, "full": false }))).expect(1).mount(&server).await;
    // A third answers with no head count at all, both times: nothing is known, so it stays waited for.
    Mock::given(method("GET")).and(path("/instances/wrld_c:3~region(eu)")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "capacity": 16 }))).expect(2).mount(&server).await;
    // The other one has closed.
    Mock::given(method("GET")).and(path("/instances/wrld_b:2~region(eu)")).respond_with(ResponseTemplate::new(404).set_body_json(json!({ "error": { "message": "gone", "status_code": 404 } }))).expect(1).mount(&server).await;

    let heard: Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let keep = heard.clone();
    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    let hub = Hub::new(api, std::env::temp_dir(), Arc::new(move |name, payload| keep.lock().unwrap().push((name.to_string(), payload))), false, None);
    assert!(matches!(hub.wait_for_room("wrld_a:1~region(eu)", "Pier"), Err(AppError::NotSignedIn)));
    hub.begin(json!({ "id": "usr_me_queue", "displayName": "Tester" })).unwrap();
    assert!(hub.wait_for_room("not a place", "Nowhere").is_err());

    hub.wait_for_room("wrld_a:1~region(eu)", "Pier").unwrap();
    hub.wait_for_room("wrld_b:2~region(eu)", "Arcade").unwrap();
    hub.wait_for_room("wrld_c:3~region(eu)", "Loft").unwrap();
    assert!(hub.wait_for_room("wrld_d:4~region(eu)", "Fourth").is_err());
    assert_eq!(hub.waiting().as_array().unwrap().len(), 3);

    // First look: the pier is still full, the arcade is gone.
    assert_eq!(hub.look_for_room().await, 2);
    assert!(heard.lock().unwrap().iter().any(|(n, p)| n == "queue:gone" && p["worldName"] == "Arcade"));
    assert!(!heard.lock().unwrap().iter().any(|(n, _)| n == "queue:room"));

    // Second look: room, said once, and nothing is waited for any more.
    assert_eq!(hub.look_for_room().await, 1);
    hub.stop_waiting("wrld_c:3~region(eu)");
    assert_eq!(hub.waiting().as_array().unwrap().len(), 0);
    let all = heard.lock().unwrap();
    assert!(!all.iter().any(|(_, p)| p["worldName"] == "Loft"));
    let room = all.iter().find(|(n, _)| n == "queue:room").expect("room is announced");
    assert_eq!((room.1["worldName"].as_str(), room.1["free"].as_i64()), (Some("Pier"), Some(1)));
    assert!(all.iter().any(|(n, p)| n == "popup" && p["title"] == "Room in Pier"));
}

#[tokio::test]
async fn boops_carry_an_emoji_and_invites_a_saved_message() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/users/usr_a/boop")).and(body_json(json!({ "emojiId": "default_heart" }))).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&server).await;
    Mock::given(method("POST")).and(path("/users/usr_a/boop")).and(body_json(json!({}))).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&server).await;
    Mock::given(method("POST")).and(path("/requestInvite/usr_a")).and(body_json(json!({ "requestSlot": 3 }))).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&server).await;
    Mock::given(method("POST")).and(path("/invite/usr_a")).and(body_json(json!({ "instanceId": "wrld_a:1", "messageSlot": 0 }))).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&server).await;
    Mock::given(method("GET")).and(path("/message/usr_me_msg/request")).respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "slot": 0, "message": "Can I join?", "canBeUpdated": true, "remainingCooldownMinutes": 0 }]))).expect(1).mount(&server).await;
    Mock::given(method("PUT")).and(path("/message/usr_me_msg/message/2")).and(body_json(json!({ "message": "Come over" }))).respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "slot": 2, "message": "Come over" }]))).expect(1).mount(&server).await;
    // The bio and its links go to the profile record, which answers with the record as kept.
    Mock::given(method("PUT")).and(path("/profile/usr_me_msg")).and(body_json(json!({ "bio": "Hello", "bioLinks": ["https://example.org/me"] }))).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "usr_me_msg", "bio": "Hello", "bioLinks": ["https://example.org/me"] }))).expect(1).mount(&server).await;
    // Pronouns alone go to the account.
    Mock::given(method("PUT")).and(path("/users/usr_me_msg")).and(body_json(json!({ "pronouns": "they/them" }))).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&server).await;

    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    api.request_invite("usr_a", Some(3)).await.unwrap();
    api.invite_user("usr_a", "wrld_a:1", Some(0)).await.unwrap();

    let hub = Hub::new(api, std::env::temp_dir(), Arc::new(|_, _| {}), false, None);
    hub.begin(json!({ "id": "usr_me_msg", "displayName": "Tester" })).unwrap();
    hub.boop("usr_a", Some("default_heart")).await.unwrap();
    hub.boop("usr_a", None).await.unwrap();
    // Anything that is not one of VRChat's emoji names or a file id never leaves the app.
    assert!(hub.boop("usr_a", Some("../admin")).await.is_err());
    assert!(hub.boop("usr_a", Some("default_heart\"}")).await.is_err());

    assert_eq!(hub.invite_messages("request").await.unwrap()[0]["message"], "Can I join?");
    assert!(hub.invite_messages("response/../x").await.is_err());
    assert_eq!(hub.set_invite_message("message", 2, " Come over ").await.unwrap()[0]["slot"], 2);
    assert!(hub.set_invite_message("message", 12, "x").await.is_err());
    assert!(hub.set_invite_message("message", 1, "").await.is_err());
    assert!(hub.set_invite_message("message", 1, &"x".repeat(65)).await.is_err());

    let links: Vec<String> = vec!["https://example.org/me".into(), "  ".into()];
    hub.update_profile(Some(("Hello", &links)), None).await.unwrap();
    hub.update_profile(None, Some(" they/them ")).await.unwrap();
    // Nothing changed, nothing sent.
    hub.update_profile(None, None).await.unwrap();
    assert!(hub.update_profile(Some(("Hello", &["javascript:alert(1)".to_string()])), None).await.is_err());
    assert!(hub.update_profile(Some((&"x".repeat(513), &[])), None).await.is_err());
    assert!(hub.update_profile(Some(("line one\nline\u{1b}two", &[])), None).await.is_err());
    assert!(hub.update_profile(Some(("", &["https://a.org".into(), "https://b.org".into(), "https://c.org".into(), "https://d.org".into()])), None).await.is_err());
    assert!(hub.update_profile(None, Some(&"x".repeat(33))).await.is_err());
}

#[tokio::test]
async fn profile_look_moderation_and_group_invites_are_checked_before_they_go_out() {
    let server = MockServer::start().await;
    Mock::given(method("PUT")).and(path("/profile/usr_me_look")).and(body_json(json!({ "backgroundType": "gradient", "backgroundGradientTop": "1a0f35", "backgroundGradientBottom": "FF4FA3", "iconFrame": "" }))).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&server).await;
    Mock::given(method("POST")).and(path("/users/usr_me_look/removeTags")).and(body_json(json!({ "tags": ["language_deu"] }))).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&server).await;
    Mock::given(method("POST")).and(path("/users/usr_me_look/addTags")).and(body_json(json!({ "tags": ["language_eng", "language_spa"] }))).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&server).await;
    Mock::given(method("GET")).and(path("/user/usr_me_look/cosmetics")).respond_with(ResponseTemplate::new(200).set_body_json(json!([
        { "itemType": "iconFrame", "templateId": "invt_00000000-0000-0000-0000-000000000001" },
        { "itemType": "iconFrame", "templateId": "invt_00000000-0000-0000-0000-000000000001" },
        { "itemType": "nameplateEffect", "templateId": "invt_00000000-0000-0000-0000-000000000002" }
    ]))).expect(1).mount(&server).await;
    Mock::given(method("GET")).and(path("/auth/user/playermoderations")).respond_with(ResponseTemplate::new(200).set_body_json(json!([
        { "type": "muteChat", "targetUserId": "usr_a" }, { "type": "hideAvatar", "targetUserId": "usr_a" }, { "type": "block", "targetUserId": "usr_b" }
    ]))).expect(1).mount(&server).await;
    Mock::given(method("POST")).and(path("/groups/grp_a/invites")).and(body_json(json!({ "userId": "usr_a", "confirmOverrideBlock": false }))).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&server).await;

    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    let hub = Hub::new(api, std::env::temp_dir(), Arc::new(|_, _| {}), false, None);
    hub.begin(json!({ "id": "usr_me_look", "displayName": "Tester" })).unwrap();

    hub.update_look(&json!({ "backgroundType": "gradient", "backgroundGradientTop": "1a0f35", "backgroundGradientBottom": "FF4FA3", "iconFrame": "" })).await.unwrap();
    hub.update_languages(&["eng".into(), "spa".into()], &["deu".into()]).await.unwrap();
    assert!(hub.update_languages(&["eng".into(), "spa".into(), "deu".into(), "fra".into()], &[]).await.is_err());
    assert!(hub.update_languages(&["EN".into()], &[]).await.is_err());
    for bad in [
        json!({ "languages": ["eng"] }),
        json!({ "backgroundGradientTop": "#1a0f35" }),
        json!({ "backgroundTextureId": "../../x" }),
        json!({ "backgroundType": "inventory" }),
        json!({ "iconFrame": "invt_x" }),
        json!({ "bio": "sneaked in" }),
    ] {
        assert!(hub.update_look(&bad).await.is_err(), "{bad}");
    }
    let mine = hub.my_cosmetics().await.unwrap();
    assert_eq!(mine["iconFrame"].as_array().unwrap().len(), 1);
    assert_eq!(mine["nameplateEffect"][0], "invt_00000000-0000-0000-0000-000000000002");

    let kinds = hub.moderations_for("usr_a").await.unwrap();
    assert_eq!(kinds, json!(["muteChat", "hideAvatar"]));
    // A second look within the minute asks VRChat nothing.
    assert_eq!(hub.moderations_for("usr_b").await.unwrap(), json!(["block"]));
    assert!(hub.set_moderation("usr_a", "report", true).await.is_err());

    hub.invite_to_group("grp_a", "usr_a").await.unwrap();
    assert!(hub.invite_to_group("grp_a/../x", "usr_a").await.is_err());
    assert!(hub.invite_to_group("grp_a", "usr_me_look").await.is_err());
}

#[tokio::test]
async fn a_bio_vrchat_did_not_keep_is_an_error_and_the_record_wins_on_the_profile() {
    let server = MockServer::start().await;
    // The record answers without the bio, and reading it back shows the old one: not kept.
    Mock::given(method("PUT")).and(path("/profile/usr_me_bio")).and(body_json(json!({ "bio": "New words", "bioLinks": [] }))).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "usr_me_bio" }))).expect(1).mount(&server).await;
    Mock::given(method("PUT")).and(path("/profile/usr_me_bio")).and(body_json(json!({ "bio": "Kept\r\nwords ", "bioLinks": [] }))).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "usr_me_bio", "bio": "Kept\nwords" }))).expect(1).mount(&server).await;
    Mock::given(method("GET")).and(path("/profile/usr_me_bio")).and(query_param("asSelf", "true")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "usr_me_bio", "bio": "Old words", "bioLinks": ["https://example.org/a"] }))).mount(&server).await;
    // The account record still carries an old bio and more links; the profile record wins.
    Mock::given(method("GET")).and(path("/users/usr_me_bio")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "usr_me_bio", "displayName": "Tester", "bio": "Stale words", "bioLinks": ["https://example.org/a", "https://example.org/b"] }))).mount(&server).await;
    Mock::given(method("GET")).and(path("/auth/user")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "usr_me_bio", "displayName": "Tester" }))).mount(&server).await;
    // Someone whose record cannot be read: nothing marks their bio as real.
    Mock::given(method("GET")).and(path("/users/usr_other")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "usr_other", "displayName": "Other" }))).mount(&server).await;
    Mock::given(method("GET")).and(path("/profile/usr_other")).respond_with(ResponseTemplate::new(500).set_body_json(json!({ "error": { "message": "busy" } }))).mount(&server).await;

    let api = Arc::new(client(&server));
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    let hub = Hub::new(api, std::env::temp_dir().join(format!("avrx-bio-{}", std::process::id())), Arc::new(|_, _| {}), false, None);
    hub.begin(json!({ "id": "usr_me_bio", "displayName": "Tester" })).unwrap();

    let err = hub.update_profile(Some(("New words", &[])), None).await.unwrap_err();
    assert!(err.to_string().contains("did not keep"), "{err}");
    // Line ends and outer spaces tidied by VRChat still count as kept.
    hub.update_profile(Some(("Kept\r\nwords ", &[])), None).await.unwrap();

    let mine = hub.profile("usr_me_bio").await.unwrap();
    assert!(mine.record_read);
    assert_eq!(mine.raw["bio"], "Old words");
    assert_eq!(mine.raw["bioLinks"], json!(["https://example.org/a"]));
    let other = hub.profile("usr_other").await.unwrap();
    assert!(!other.record_read);
}


#[tokio::test]
async fn a_slowdown_holds_the_next_read_even_after_the_last_try_and_sign_out_still_goes() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/worlds/wrld_a")).respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1")).expect(1).mount(&server).await;
    Mock::given(method("GET")).and(path("/worlds/wrld_b")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "wrld_b", "name": "Calm Dock" }))).expect(1).mount(&server).await;
    Mock::given(method("PUT")).and(path("/logout")).respond_with(ResponseTemplate::new(200)).expect(1).mount(&server).await;
    let api = Arc::new(ApiClient::with_base(&server.uri(), Timing { attempts: 1, max_wait: Duration::from_secs(5), action_wait: Duration::from_millis(100), ..quick() }).unwrap());
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    assert!(matches!(api.world("wrld_a").await, Err(AppError::RateLimited(Some(1)))));
    // The read that follows waits out the second VRChat asked for, and so does signing out,
    // which is sent once it may go rather than turned away (the mock expects it once).
    let started = std::time::Instant::now();
    let (read, _) = tokio::join!(api.world("wrld_b"), api.logout());
    assert_eq!(read.unwrap()["name"], "Calm Dock");
    assert!(started.elapsed() >= Duration::from_millis(900), "{:?}", started.elapsed());
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test]
async fn signing_out_is_done_at_once_and_vrchat_is_told_later_with_the_old_cookie() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/worlds/wrld_a")).respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1")).expect(1).mount(&server).await;
    Mock::given(method("PUT")).and(path("/logout")).and(header("cookie", "auth=authcookie_old")).respond_with(ResponseTemplate::new(200)).expect(1).mount(&server).await;
    let api = Arc::new(ApiClient::with_base(&server.uri(), Timing { attempts: 1, max_wait: Duration::from_secs(5), ..quick() }).unwrap());
    api.set_session(Session { auth: Some("authcookie_old".into()), two_factor: None });
    assert!(matches!(api.world("wrld_a").await, Err(AppError::RateLimited(Some(1)))));
    // Inside the slowdown, signing out still answers at once.
    let started = std::time::Instant::now();
    api.logout().await.unwrap();
    assert!(started.elapsed() < Duration::from_millis(300), "{:?}", started.elapsed());
    assert!(api.auth_token().is_none());
    // A new sign-in made before VRChat is told is neither sent along nor undone.
    api.set_session(Session { auth: Some("authcookie_new".into()), two_factor: None });
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(api.auth_token().as_deref(), Some("authcookie_new"));
    let outs: Vec<_> = server.received_requests().await.unwrap().into_iter().filter(|r| r.url.path() == "/logout").collect();
    assert_eq!(outs.len(), 1);
}

#[tokio::test]
async fn a_read_told_to_wait_longer_than_it_sleeps_goes_once_and_holds_the_rest_for_the_whole_time() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/worlds/wrld_a")).respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "2")).expect(1).mount(&server).await;
    Mock::given(method("GET")).and(path("/worlds/wrld_b")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "wrld_b" }))).expect(0).mount(&server).await;
    // `quick` sleeps at most 200 ms per 429; VRChat asks for two seconds.
    let api = client(&server);
    let started = std::time::Instant::now();
    let got = api.world("wrld_a").await;
    assert!(matches!(got, Err(AppError::RateLimited(Some(2)))), "{got:?}");
    assert!(started.elapsed() < Duration::from_millis(500), "{:?}", started.elapsed());
    // The next read is held for the whole two seconds, not just `max_wait`: it is longer than a
    // read sleeps, so it is answered here with the time left and nothing is sent.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let got = api.world("wrld_b").await;
    assert!(matches!(got, Err(AppError::RateLimited(Some(s))) if (1..=2).contains(&s)), "{got:?}");
}

#[tokio::test]
async fn a_sign_in_vrchat_turns_down_holds_the_next_one_and_the_code() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/auth/user")).respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "30")).expect(1).mount(&server).await;
    Mock::given(method("POST")).and(path("/auth/twofactorauth/totp/verify")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "verified": true }))).expect(0).mount(&server).await;
    let api = Arc::new(client(&server));
    assert!(matches!(api.login("tester", "x").await, Err(AppError::RateLimited(Some(30)))));
    // Pressing Sign in again, or sending a code, is answered here without a request.
    assert!(matches!(api.login("tester", "x").await, Err(AppError::RateLimited(Some(s))) if s > 25 && s <= 30));
    assert!(matches!(api.verify_code("totp", "123456").await, Err(AppError::RateLimited(Some(_)))));
    // Signing out does not lift the wait.
    api.logout().await.unwrap();
    assert!(matches!(api.login("tester", "x").await, Err(AppError::RateLimited(Some(_)))));
}

#[tokio::test]
async fn a_code_vrchat_turns_down_holds_signing_in_too() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/auth/twofactorauth/totp/verify")).respond_with(ResponseTemplate::new(429)).expect(1).mount(&server).await;
    Mock::given(method("GET")).and(path("/auth/user")).respond_with(ResponseTemplate::new(200).set_body_json(user())).expect(0).mount(&server).await;
    let api = client(&server);
    // No Retry-After: the minute of `action_hold`.
    assert!(matches!(api.verify_code("totp", "123456").await, Err(AppError::RateLimited(Some(60)))));
    assert!(matches!(api.login("tester", "x").await, Err(AppError::RateLimited(Some(_)))));
}

#[tokio::test]
async fn signed_pictures_keep_to_the_slowdown_and_start_one_themselves() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/file/busy")).respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "2")).expect(1).mount(&server).await;
    Mock::given(method("GET")).and(path("/file/photo")).respond_with(ResponseTemplate::new(200).insert_header("content-type", "image/png").set_body_bytes(vec![1u8; 4])).expect(0).mount(&server).await;
    Mock::given(method("GET")).and(path("/worlds/wrld_a")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "wrld_a" }))).expect(0).mount(&server).await;
    // Another host is not VRChat's API and is not held back.
    let elsewhere = MockServer::start().await;
    Mock::given(method("GET")).and(path("/pic.png")).respond_with(ResponseTemplate::new(200).insert_header("content-type", "image/png").set_body_bytes(vec![1u8, 2, 3])).expect(1).mount(&elsewhere).await;
    let api = client(&server);
    api.set_session(Session { auth: Some("authcookie_test".into()), two_factor: None });
    let got = api.picture_as(&format!("{}/file/busy", server.uri()), true).await;
    assert!(matches!(got, Err(AppError::RateLimited(Some(2)))), "{got:?}");
    // The next signed picture is refused here, and reads wait out the same two seconds.
    let got = api.picture_as(&format!("{}/file/photo", server.uri()), true).await;
    assert!(matches!(got, Err(AppError::RateLimited(Some(s))) if s <= 2), "{got:?}");
    assert!(matches!(api.world("wrld_a").await, Err(AppError::RateLimited(Some(_)))));
    api.picture_as(&format!("{}/pic.png", elsewhere.uri()), true).await.unwrap();
}

#[tokio::test]
async fn an_avatar_database_that_fails_is_not_put_down_to_vrchat() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/down")).respond_with(ResponseTemplate::new(503)).mount(&server).await;
    Mock::given(method("GET")).and(path("/odd")).respond_with(ResponseTemplate::new(200).set_body_string("<html>busy</html>")).mount(&server).await;
    let api = client(&server);
    // A port nothing listens on stands in for a service that is gone.
    let gone = {
        let spare = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}/db", spare.local_addr().unwrap())
    };
    for provider in [gone, format!("{}/down", server.uri()), format!("{}/odd", server.uri())] {
        let got = api.search_avatars(&provider, "fox").await;
        let Err(AppError::CannotDo(words)) = got else { panic!("{provider}: {got:?}") };
        assert!(words.starts_with("The avatar search service"), "{words}");
        assert!(!words.contains("VRChat"), "{words}");
    }
    let got = api.ask_avatar_database(&format!("{}/odd", server.uri()), "fileId", "file_1").await;
    assert!(matches!(got, Err(AppError::CannotDo(ref w)) if w.starts_with("The avatar search service")), "{got:?}");
}

#[tokio::test]
async fn a_client_for_vrchat_itself_never_fetches_from_this_pc() {
    // The real client only lets https through, and not to this PC or the home network.
    let api = ApiClient::new().unwrap();
    for url in ["http://127.0.0.1:8080/admin/delete?all=1", "https://localhost/pic.png", "https://192.168.1.1/pic.png"] {
        assert!(matches!(api.picture(url).await, Err(AppError::CannotDo(_))), "{url}");
        assert!(matches!(api.search_avatars(url.split('?').next().unwrap(), "fox").await, Err(AppError::CannotDo(_))), "{url}");
    }
}

#[tokio::test]
async fn the_avatar_databases_do_not_wait_out_a_vrchat_slowdown() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/worlds/wrld_a")).respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "2")).mount(&server).await;
    Mock::given(method("GET")).and(path("/db")).respond_with(ResponseTemplate::new(200).set_body_json(json!([]))).expect(1).mount(&server).await;
    let api = Arc::new(ApiClient::with_base(&server.uri(), Timing { attempts: 3, max_wait: Duration::from_secs(5), ..quick() }).unwrap());
    // A read keeps meeting the slowdown and waits it out in the background.
    let reader = tokio::spawn({
        let api = api.clone();
        async move { api.world("wrld_a").await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let started = std::time::Instant::now();
    api.search_avatars(&format!("{}/db", server.uri()), "fox").await.unwrap();
    assert!(started.elapsed() < Duration::from_millis(500), "{:?}", started.elapsed());
    reader.abort();
}
