// Made-up friends and worlds for looking at the layout in a browser tab.
// Nothing here talks to VRChat.

import type { AvatarWorn, Backend, Moderation, FeedItem, Friend, Handled, LiveEvents, LocalFavorite, Notice, Person, Place, Sighting, Visit, World } from "./backend";

/** A flat picture in two colours, so the demo has something where a real picture goes. */
const swatch = (a: string, b: string) =>
  `data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 6"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${a}"/><stop offset="1" stop-color="${b}"/></linearGradient></defs><rect width="8" height="6" fill="url(#g)"/><circle cx="5.6" cy="2" r="1.1" fill="#fff" opacity=".35"/></svg>`)}`;

const worlds: World[] = [
  { id: "wrld_demo_pier", name: "\uFF3BAutumn\uFF3D\u3000Quiet Pier at Dusk", image: swatch("#ff7a5c", "#3a1a78") },
  { id: "wrld_demo_arcade", name: "Basement Arcade", image: swatch("#2de2e6", "#1a2a6b") },
  { id: "wrld_demo_attic", name: "Rainy Attic Hangout", image: swatch("#5a6a8a", "#1c1f33") },
  { id: "wrld_demo_mirror", name: "Mirror Room 4", image: swatch("#c39bff", "#40206a") },
  { id: "wrld_demo_long", name: "Fate/Grand Order: Gilgamesh's Ziggurat, the long way round [NO AI] (READ DESCRIPTION)", image: swatch("#ffd166", "#7a3b12") },
];

function place(location: string): Place {
  if (location === "private") return { kind: "private", worldId: null, instance: null, access: null, region: null, groupId: null };
  if (location === "offline" || location === "") return { kind: "none", worldId: null, instance: null, access: null, region: null, groupId: null };
  const [worldId, rest = ""] = location.split(":");
  const access = rest.includes("~hidden") ? "friendsPlus" : rest.includes("~friends") ? "friends" : rest.includes("~private") ? "invite" : rest.includes("~group") ? "groupPlus" : "public";
  const region = /region\((\w+)\)/.exec(rest)?.[1] ?? null;
  return { kind: "world", worldId, instance: rest.split("~")[0], access, region, groupId: null };
}

const spots = [
  "wrld_demo_pier:40211~hidden(usr_x)~region(eu)",
  "wrld_demo_arcade:9917~region(us)",
  "wrld_demo_attic:5120~friends(usr_y)~region(eu)",
  "wrld_demo_mirror:77~group(grp_z)~groupAccessType(plus)~region(jp)",
];

function friend(n: number, name: string, state: Friend["state"], status: string, text: string, location: string): Friend {
  return {
    id: `usr_demo_${n}`,
    displayName: name,
    state,
    status,
    statusDescription: text,
    location,
    travelingTo: "",
    place: place(location),
    image: "",
    platform: state === "online" ? (n % 4 === 0 ? "android" : "standalonewindows") : n % 2 === 0 ? "android" : "web",
    avatarImage: "",
    since: state === "online" ? new Date(Date.now() - n * 11 * 60_000).toISOString() : "",
    sinceAtLeast: n % 3 === 0,
    rank: ["Trusted User", "Known User", "User", "New User"][n % 4],
    vrcPlus: n % 3 === 0,
    adult: n % 2 === 1,
  };
}

const people: Friend[] = [
  friend(1, "Mossy", "online", "join me", "pier sunsets, come sit", spots[0]),
  friend(2, "Pip Halloway", "online", "active", "", spots[0]),
  friend(3, "juno.wav", "online", "active", "mixing a set", spots[0]),
  friend(4, "Rook", "online", "ask me", "testing avatar physics", spots[1]),
  friend(5, "Tamsin", "online", "active", "high score or bust", spots[1]),
  friend(6, "Okapi", "online", "busy", "world building, do not poke", spots[2]),
  friend(7, "Bexley", "online", "active", "", spots[3]),
  friend(8, "Nori", "online", "join me", "mirror dwellers unite", spots[3]),
  friend(9, "Fennel", "online", "ask me", "", "private"),
  friend(10, "Wren", "online", "busy", "date night", "private"),
  friend(11, "Calloway", "active", "active", "on my phone", "offline"),
  friend(12, "Sable", "active", "ask me", "", "offline"),
  ...["Vesper", "Onyx", "Lumen", "Pixel", "Sorrel", "Quill", "Tansy", "Umber", "Wisp", "Yarrow", "Zephyr", "Cricket"].map((n, i) =>
    friend(60 + i, n, "online", i % 3 === 0 ? "join me" : "active", i % 2 ? "dancing" : "", spots[1]),
  ),
  ...["Ash", "Birdie", "Coda", "Dune", "Elio", "Farrow", "Gale", "Hollis", "Indra", "Jett", "Kestrel", "Lark"].map((n, i) =>
    friend(20 + i, n, "offline", "offline", "", "offline"),
  ),
];

let feedId = 0;
const feed: FeedItem[] = [];
function log(f: Friend, kind: FeedItem["kind"], minutesAgo: number, detail = "", previous = "", location = f.location) {
  feed.unshift({
    id: ++feedId,
    at: new Date(Date.now() - minutesAgo * 60_000).toISOString(),
    userId: f.id,
    displayName: f.displayName,
    kind,
    detail,
    previous,
    location,
    worldId: place(location).worldId,
  });
}
log(people[10], "offline", 190, "", "", spots[1]);
log(people[5], "online", 142);
log(people[5], "statusText", 131, "world building, do not poke", "");
log(people[0], "online", 96);
log(people[3], "status", 71, "ask me", "active");
log(people[1], "location", 48);
log(people[9], "online", 33);
log(people[2], "location", 21);
log(people[7], "statusText", 12, "mirror dwellers unite", "afk");
log(people[6], "location", 4);
log(people[2], "location", 3, "", "", spots[1]);
log(people[2], "location", 2, "", "", spots[3]);
log(people[2], "location", 1, "", "", spots[0]);

const ago = (minutes: number) => new Date(Date.now() - minutes * 60_000).toISOString();

function visit(id: number, spot: string, name: string, from: number, to: number | null, people: number): Visit {
  return {
    id,
    location: spot,
    worldId: place(spot).worldId ?? "",
    worldName: name,
    joinedAt: ago(from),
    leftAt: to === null ? null : ago(to),
    place: place(spot),
    people,
  };
}

const demoVisits: Visit[] = [
  visit(4, spots[0], "Quiet Pier at Dusk", 38, null, 6),
  visit(3, spots[1], "Basement Arcade", 95, 40, 9),
  visit(2, spots[3], "Mirror Room 4", 60 * 26, 60 * 24, 14),
  visit(1, spots[2], "Rainy Attic Hangout", 60 * 28, 60 * 26 + 5, 4),
];

const seen = (f: Friend | string, from: number, to: number | null): Sighting => ({
  userId: typeof f === "string" ? `usr_demo_stranger_${f}` : f.id,
  displayName: typeof f === "string" ? f : f.displayName,
  joinedAt: ago(from),
  leftAt: to === null ? null : ago(to),
});

const demoPeople: Sighting[] = [
  seen(people[0], 38, null),
  seen(people[1], 36, null),
  seen(people[2], 21, null),
  seen("Marlowe", 30, null),
  seen("quietstatic", 33, 9),
];

const blank = { title: "", seen: false, location: "", worldName: "", image: "", replies: [] };
let demoNotices: Notice[] = [
  { ...blank, id: "not_demo_1", version: 1, kind: "invite", senderId: "usr_demo_1", senderName: "Mossy", message: "pier sunsets, come sit", at: ago(3), location: spots[0], worldName: "Quiet Pier at Dusk" },
  { ...blank, id: "not_demo_2", version: 1, kind: "requestInvite", senderId: "usr_demo_4", senderName: "Rook", message: "room for one more?", at: ago(12) },
  { ...blank, id: "not_demo_3", version: 1, kind: "friendRequest", senderId: "usr_demo_stranger_1", senderName: "Marlowe", message: "", at: ago(50), seen: true },
  {
    ...blank,
    id: "not_demo_4",
    version: 2,
    kind: "group.invite",
    senderId: "grp_demo",
    senderName: "",
    title: "Night Owls invited you to join",
    message: "A late-hours hangout group.",
    at: ago(60 * 25),
    seen: true,
    replies: [
      { kind: "accept", data: "grp_demo", text: "Join" },
      { kind: "decline", data: "grp_demo", text: "Decline" },
    ],
  },
];

const demoBlocked: Moderation[] = [
  { kind: "block", userId: "usr_demo_blocked", name: "Static Crow", at: new Date(Date.now() - 6 * 86400000).toISOString() },
  { kind: "mute", userId: "usr_demo_muted", name: "Loud Harbor", at: new Date(Date.now() - 30 * 86400000).toISOString() },
];

const demoHandled: Handled[] = [];
let demoAutostart = false;
const demoRatings: Record<string, number> = {};
const demoFollowed = new Set<string>(["cal_demo_1"]);
const demoTags: Record<string, string[]> = {};
const demoRemoved = new Set<string>();
let demoMember = false;
const demoNames = new Map<string, string>();
const demoFavFriends = [{ id: "fvrt_f1", favoriteId: "usr_demo_2", tags: ["group_0"] }];
const demoLocal: LocalFavorite[] = [];
const demoAvatars: AvatarWorn[] = [
  { avatarId: "avtr_demo_Robot", name: "Robot", authorId: "usr_demo_me", authorName: "Rook", image: "", releaseStatus: "public", lastWorn: new Date(Date.now() - 20 * 60_000).toISOString(), times: 6, seconds: 5 * 3600, secondsInGame: Math.round((5 * 3600) * 0.7), visits: 4, lastInGame: ago(35) },
  { avatarId: "avtr_demo_Fox", name: "Sunset Fox", authorId: "usr_demo_4", authorName: "Rook", image: "", releaseStatus: "public", lastWorn: new Date(Date.now() - 26 * 3600_000).toISOString(), times: 2, seconds: 3 * 3600 + 1200, secondsInGame: Math.round((3 * 3600 + 1200) * 0.7), visits: 4, lastInGame: ago(35) },
  { avatarId: "avtr_demo_Knight", name: "Knight", authorId: "usr_demo_5", authorName: "Tamsin", image: "", releaseStatus: "public", lastWorn: new Date(Date.now() - 5 * 24 * 3600_000).toISOString(), times: 1, seconds: 900, secondsInGame: 630, visits: 4, lastInGame: ago(35) },
];
const demoNotes = new Map<string, string>([["usr_demo_1", "Met at the pier. Plays drums."]]);

function demoPerson(f: Friend): Person {
  return {
    id: f.id,
    displayName: f.displayName,
    image: "",
    bio: `${f.displayName} likes quiet worlds, long sunsets and bad puns.`,
    bioLinks: ["https://example.test/" + f.displayName.toLowerCase().replace(/\W+/g, "")],
    pronouns: "they/them",
    status: f.status,
    statusDescription: f.statusDescription,
    rank: "Known User",
    joined: "2021-04-02",
    lastLogin: new Date().toISOString(),
    lastPlatform: f.platform,
    isFriend: true,
    location: f.location,
    vrchatNote: "",
  };
}
const demoBackups: Array<{ name: string; path: string; bytes: number }> = [];

type Handler = (payload: never) => void;
const handlers = new Map<string, Set<Handler>>();
function emit<K extends keyof LiveEvents>(name: K, payload: LiveEvents[K]) {
  handlers.get(name)?.forEach((h) => h(payload as never));
}

let timer: number | undefined;
function startMoving() {
  if (timer !== undefined) return;
  timer = window.setInterval(() => {
    const movers = people.filter((p) => p.state === "online" && p.place.kind === "world");
    const f = movers[Math.floor(Math.random() * movers.length)];
    const next = spots[Math.floor(Math.random() * spots.length)];
    if (!f || next === f.location) return;
    f.location = next;
    f.place = place(next);
    f.since = new Date().toISOString();
    f.sinceAtLeast = false;
    emit("friend:changed", { ...f });
    log(f, "location", 0);
    emit("feed:new", feed[0]);
  }, 9000);
}

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
const demoMe = { id: "usr_demo_me", displayName: "You", status: "active", statusDescription: "demo data", image: "" };
let signedIn = false;

/** Lets the browser demo show the close question: run `demoAskClose()` in the console. */
(window as unknown as { demoAskClose: () => void }).demoAskClose = () => emit("window:close-asked", null);

export const demoBackend: Backend = {
  demo: true,
  async restore() {
    // "?show=<screen>" opens the demo already signed in, for taking pictures of the interface.
    if (new URLSearchParams(location.search).has("show")) {
      signedIn = true;
      startMoving();
      document.documentElement.dataset.shot = "on";
      return { status: "signedIn", me: demoMe, methods: [] };
    }
    return { status: "signedOut", me: null, methods: [] };
  },
  async login() {
    await wait(500);
    return { status: "needsCode", me: null, methods: ["totp", "otp"] };
  },
  async verifyCode(_method, code) {
    await wait(400);
    if (!/^\d{6}$/.test(code.trim())) {
      throw { kind: "badCode", message: "That code was not accepted" };
    }
    signedIn = true;
    startMoving();
    return { status: "signedIn", me: demoMe, methods: [] };
  },
  async logout() {
    signedIn = false;
    window.clearInterval(timer);
    timer = undefined;
  },
  async friends() {
    return signedIn ? people.map((p) => ({ ...p })) : [];
  },
  async favorites() {
    return {
      groups: [
        { key: "group_0", name: "Close friends" },
        { key: "group_1", name: "Dance crew" },
      ],
      friends: {
        usr_demo_1: "group_0",
        usr_demo_9: "group_0",
        usr_demo_11: "group_0",
        usr_demo_4: "group_1",
        usr_demo_8: "group_1",
        usr_demo_22: "group_1",
      },
    };
  },
  async notify() {
    return true;
  },
  async handled(limit) {
    return demoHandled.slice(0, limit);
  },
  async hideToTray() {},
  async quit() {},
  async backupNow() {
    const made = { name: `backup ${new Date().toISOString().slice(0, 19).replace("T", " ").replaceAll(":", ".")}`, path: "Documents\\Anonymous VRX backups", bytes: 245760 };
    demoBackups.unshift(made);
    return made;
  },
  async backups() {
    return demoBackups;
  },
  async vrcxFound() {
    return true;
  },
  async worldStats(days) {
    const out = [];
    for (const [n, id] of ["wrld_demo_pier", "wrld_demo_arcade"].entries()) {
      let visits = n ? 2100 : 12400;
      let favorites = n ? 60 : 300;
      for (let d = Math.min(days, 30); d >= 0; d--) {
        const day = new Date(Date.now() - d * 86_400_000);
        visits += Math.round((n ? 6 : 22) + 14 * Math.abs(Math.sin(d * (n + 1.3))) + (day.getDay() % 6 === 0 ? 25 : 0));
        favorites += d % (n ? 5 : 2) === 0 ? 1 : 0;
        out.push({ worldId: id, day: `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, "0")}-${String(day.getDate()).padStart(2, "0")}`, visits, favorites, occupants: Math.round(3 + 9 * Math.abs(Math.cos(d + n))) });
      }
    }
    return out;
  },
  async updateInfo() {
    return { version: "0.1.1", supported: true };
  },
  async updateCheck() {
    await wait(600);
    return new URLSearchParams(location.search).has("update") ? { version: "0.2.0", notes: "Friend map, faster search." } : null;
  },
  async updateInstall() {
    await wait(1500);
  },
  async cosmetics() {
    const frame = `data:image/svg+xml,${encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#ffd166"/><stop offset="1" stop-color="#ef476f"/></linearGradient></defs><circle cx="50" cy="50" r="39" fill="none" stroke="url(#g)" stroke-width="5"/><circle cx="50" cy="50" r="45" fill="none" stroke="#ffd166" stroke-opacity=".45" stroke-width="1.5" stroke-dasharray="3 5"/></svg>')}`;
    const embers = `data:image/svg+xml,${encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 150" preserveAspectRatio="none"><defs><linearGradient id="f" x1="0" y1="1" x2="0" y2="0"><stop offset="0" stop-color="#ff5a1f" stop-opacity=".85"/><stop offset=".55" stop-color="#ffb347" stop-opacity=".25"/><stop offset="1" stop-color="#ffb347" stop-opacity="0"/></linearGradient></defs><path d="M0 150V95q20-30 40-5t40-20 40 25 40-35 40 30 40-20 40 25 40-15v70z" fill="url(#f)"/></svg>')}`;
    return {
      demo_frame: { picture: frame, still: frame, from: "", to: "" },
      demo_effect: { picture: embers, still: embers, from: "", to: "" },
      demo_plate: { picture: "", still: "", from: "7a1010", to: "f97316" },
    };
  },
  async vrcxStamp() {
    return "";
  },
  async getNoGpu() {
    return { wanted: false, now: false };
  },
  async setNoGpu() {},
  async restartApp() {},
  async gameCache() {
    return { path: "C:\\Users\\You\\AppData\\LocalLow\\VRChat\\VRChat\\Cache-WindowsPlayer", found: true, bytes: 23_640_000_000, items: 412, oldVersions: 37, oldBytes: 3_180_000_000, limitGb: 30 };
  },
  async clearGameCache(oldOnly) {
    return { bytes: oldOnly ? 3_180_000_000 : 23_640_000_000, folders: oldOnly ? 37 : 412, kept: 0 };
  },
  async setGameCacheLimit(gb) {
    return Math.max(30, gb);
  },
  async setVrNotices() {},
  async testVrNotice(rules) {
    return [
      ...(rules.xs ? [{ target: "XSOverlay", ok: true, problem: "" }] : []),
      ...(rules.ovrHud ? [{ target: "OVR Toolkit, in view", ok: false, problem: "It is not running, or its outside messages are switched off." }] : []),
      ...(rules.ovrWrist ? [{ target: "OVR Toolkit, on the wrist", ok: false, problem: "It is not running, or its outside messages are switched off." }] : []),
    ];
  },
  async importVrcx() {
    await wait(900);
    return { broughtOver: { feed: 1840, friendLog: 61, visits: 412, people: 3907, notes: 4, favourites: 12 }, backup: "before bringing over VRCX 2026-11-25 21.10.00" };
  },
  async autoBackup() {
    return null;
  },
  async restoreBackup() {
    await wait(300);
    return "";
  },
  async openBackupsFolder() {},
  async profile(userId) {
    await wait(300);
    const f = people.find((p) => p.id === userId) ?? { ...people[0], id: userId, displayName: "Stranger", state: "offline" as const };
    return {
      person: { ...demoPerson(f), isFriend: people.some((p) => p.id === userId) },
      note: demoNotes.get(userId) ?? "",
      pastNames: [["Moss", ago(60 * 24 * 90)]] as Array<[string, string]>,
      together: { times: 7, seconds: 5 * 3600 + 1260, first: ago(60 * 24 * 40), last: ago(55) },
      feed: feed.filter((i) => i.userId === userId).slice(0, 20),
      raw: {
        id: userId,
        displayName: f.displayName,
        tags: ["system_trust_trusted", "language_eng", "language_spa", "system_supporter"],
        ageVerificationStatus: "18+",
        bannerColor: "#c2410c",
        bannerType: "avatarBanner",
        iconFrame: "demo_frame",
        profileEffect: "demo_effect",
        nameplateEffect: "demo_plate",
        last_activity: ago(33),
        currentAvatar: "avtr_demo_worn",
        currentAvatarName: "Sunset Fox",
        currentAvatarAuthorName: "Rook",
        bio: `${f.displayName} likes quiet worlds, long sunsets and bad puns.`,
        badges: [{ badgeId: "bdg_1", badgeName: "Early supporter", badgeDescription: "Was here early", badgeImageUrl: "" }],
        languages: ["eng", "spa"],
        hasVrcPlus: true,
        currentAvatarImageUrl: "https://example.test/file_11111111-2222-3333-4444-555555555555/1/file",
        allowAvatarCopying: false,
        isBoopingEnabled: true,
        hasSharedConnectionsOptOut: false,
      },
    };
  },
  async thing(kind, id) {
    await wait(250);
    if (kind === "groupMembers") return people.slice(0, 9).map((p) => ({ userId: p.id, user: { displayName: p.displayName } }));
    if (kind === "groupPosts") return { posts: [{ id: "post_1", title: "Friday meet-up", text: "Same pier, same time.", createdAt: ago(60 * 30) }] };
    if (kind === "instance") return { n_users: 6, capacity: 32 };
    if (kind === "groupInstances") return [{ instanceId: "40211", location: spots[0], memberCount: 3, world: { id: "wrld_demo_pier", name: "Quiet Pier at Dusk" } }];
    if (kind === "groupPhotos") return [];
    if (kind === "group") return { galleries: [{ id: "ggal_demo_1", name: "Meet-ups" }], id, name: "Night Owls", description: "A late-hours hangout group.", rules: "Be kind.", shortCode: "OWLS", discriminator: "0042", memberCount: 412, onlineMemberCount: 23, joinState: "open", membershipStatus: demoMember ? "member" : "inactive", ownerId: people[0].id, createdAt: ago(60 * 24 * 400) };
    if (kind === "world") return { id, name: worlds.find((w) => w.id === id)?.name ?? "Quiet Pier at Dusk", authorName: "Rook", authorId: people[3].id, description: "Sit on the pier and watch the sun go down.", capacity: 32, recommendedCapacity: 16, visits: 12840, favorites: 310, occupants: 6, publicOccupants: 4, privateOccupants: 2, version: 12, instances: [["40211~region(eu)", 4], ["9917~region(us)", 2]], tags: ["author_tag_chill", "author_tag_sunset"], created_at: ago(60 * 24 * 900), updated_at: ago(60 * 24 * 20) };
    return { id, name: id === "avtr_demo_Robot" ? "My Robot" : "Sunset Fox", authorName: id === "avtr_demo_Robot" ? "You" : "Rook", authorId: id === "avtr_demo_Robot" ? "usr_demo_me" : people[3].id, description: "A fox for golden hour.", releaseStatus: "public", performance: { standalonewindows: "Good", android: "Medium" }, version: 4, created_at: ago(60 * 24 * 200), updated_at: ago(60 * 24 * 9) };
  },
  async favoritesOf(kind) {
    await wait(250);
    if (kind === "friend") {
      return {
        capacity: 150,
        groups: [{ name: "group_0", displayName: demoNames.get("group_0") ?? "Close friends" }, { name: "group_1", displayName: demoNames.get("group_1") ?? "Dance crew" }],
        entries: demoFavFriends.filter((e) => !demoRemoved.has(e.id)),
        items: [],
      };
    }
    if (kind === "avatar") {
      return {
        capacity: 3,
        groups: [{ name: "avatars1", displayName: demoNames.get("avatars1") ?? "Everyday" }, { name: "avatars2", displayName: demoNames.get("avatars2") ?? "avatars2" }],
        entries: [
          { id: "fvrt_1", favoriteId: "avtr_demo_Robot", tags: ["avatars1"] },
          { id: "fvrt_2", favoriteId: "avtr_demo_gone", tags: ["avatars1"] },
          { id: "fvrt_3", favoriteId: "avtr_demo_private", tags: ["avatars1"] },
        ].filter((e) => !demoRemoved.has(e.id)),
        items: [
          { id: "avtr_demo_Robot", name: "Robot", authorName: "Rook", authorId: "usr_demo_4", releaseStatus: "public" },
          { id: "avtr_demo_private", name: "Old Favourite", authorName: "Tamsin", authorId: "usr_demo_5", releaseStatus: "private" },
        ],
      };
    }
    return {
      groups: [{ name: "worlds1", displayName: "Chill" }],
      entries: worlds.map((w, i) => ({ id: `fvrt_w${i}`, favoriteId: w.id, tags: ["worlds1"] })).filter((e) => !demoRemoved.has(e.id)),
      items: worlds.map((w) => ({ id: w.id, name: w.name, authorName: "Rook", releaseStatus: "public", thumbnailImageUrl: w.image })),
    };
  },
  async addFavorite(kind, id, group) {
    await wait(250);
    if (kind === "friend") demoFavFriends.push({ id: `fvrt_f${demoFavFriends.length + 1}`, favoriteId: id, tags: [group] });
  },
  async removeFavorites(entryIds) {
    await wait(300);
    entryIds.forEach((id) => demoRemoved.add(id));
    return entryIds.length;
  },
  async checkFavorites(kind) {
    await wait(600);
    const dead =
      kind === "avatar"
        ? [
            { entryId: "fvrt_2", favoriteId: "avtr_demo_gone", why: "deleted" as const, name: "" },
            { entryId: "fvrt_3", favoriteId: "avtr_demo_private", why: "private" as const, name: "Old Favourite" },
          ].filter((d) => !demoRemoved.has(d.entryId))
        : [];
    return { dead, checked: dead.length, left: 0, stopped: false };
  },
  async setAlerts() {},
  async setGameWatch() {},
  async getAutostart() {
    return demoAutostart;
  },
  async setAutostart(on) {
    await wait(150);
    demoAutostart = on;
  },
  async favoriteGroups(counts) {
    await wait(counts ? 1200 : 150);
    return {
      groups: [
        { type: "friend", name: "group_0", displayName: "Close friends" },
        { type: "friend", name: "group_1", displayName: "Dance crew" },
        { type: "avatar", name: "avatars1", displayName: "Everyday" },
        { type: "world", name: "worlds1", displayName: "Chill" },
      ],
      counts: (counts ? { "friend:group_0": 3, "friend:group_1": 2, "avatar:avatars1": 3, "world:worlds1": 2 } : {}) as Record<string, number>,
    };
  },
  async setDiscord() {},
  async saveSettings() {},
  async loadSettings() {
    return null;
  },
  async photos(limit, skip, query, until) {
    await wait(300);
    const all = Array.from({ length: 7 }, (_, i) => {
      const hue = (i * 47) % 360;
      const svg = `<svg xmlns='http://www.w3.org/2000/svg' width='640' height='360'><rect width='640' height='360' fill='hsl(${hue} 55% 32%)'/><circle cx='${180 + i * 40}' cy='170' r='70' fill='hsl(${(hue + 40) % 360} 80% 62%)'/></svg>`;
      return {
        path: `C:/demo/VRChat_demo_${i}.png`,
        url: `data:image/svg+xml,${encodeURIComponent(svg)}`,
        file: `VRChat_demo_${i}.png`,
        takenAt: ago(40 + i * 190),
        width: 1920,
        height: 1080,
        worldId: i === 5 ? "" : "wrld_demo_pier",
        worldName: i === 5 ? "" : i % 2 ? "Quiet Pier at Dusk" : "Basement Arcade",
        location: "",
        people: i === 5 ? [] : [{ userId: "usr_demo_1", displayName: "Fennel" }, { userId: "usr_demo_2", displayName: "Mossy" }].slice(0, 1 + (i % 2)),
      };
    });
    const q = (query ?? "").trim().toLowerCase();
    const end = until ? Date.parse(`${until}T23:59:59`) : Infinity;
    const hits = all.filter(
      (p) => Date.parse(p.takenAt) <= end && (!q || p.worldName.toLowerCase().includes(q) || p.people.some((who) => who.displayName.toLowerCase().includes(q))),
    );
    return { total: hits.length, everything: all.length, photos: hits.slice(skip, skip + limit) };
  },
  async showPhoto() {
    throw { kind: "cannotDo", message: "Folders open in the desktop app; this is demo data." };
  },
  async openLicencesFolder() {
    throw { kind: "cannotDo", message: "The licence texts come with the desktop app; this is demo data." };
  },
  async uploadAction() {
    await wait(300);
  },
  async friendAction() {
    await wait(300);
  },
  async moderations() {
    await wait(250);
    return demoBlocked.slice();
  },
  async calendar(which, month) {
    await wait(350);
    const now = new Date();
    if (which !== "discover" && month !== `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}`) return { events: [], more: false };
    const at = (hours: number) => new Date(Date.now() + hours * 3_600_000).toISOString();
    const all = [
      { id: "cal_demo_1", groupId: "grp_demo", groupName: "Night Owls", title: "Pier sunset hangout", description: "Bring a blanket. We sit on the pier until the sun is gone, then move to the arcade.", category: "hangout", access: "public", startsAt: at(0.4), endsAt: at(2.4), image: "", interested: 41, friends: 3, featured: false },
      { id: "cal_demo_2", groupId: "grp_demo", groupName: "Night Owls", title: "Friday dance night", description: "Three DJs, two rooms.", category: "music", access: "group", startsAt: at(26), endsAt: at(30), image: "", interested: 128, friends: 5, featured: true },
      { id: "cal_demo_3", groupId: "grp_demo_2", groupName: "Arcade High Scores", title: "High score tournament, round 2", description: "", category: "gaming", access: "public", startsAt: at(74), endsAt: at(77), image: "", interested: 9, friends: 0, featured: false },
      { id: "cal_demo_4", groupId: "grp_demo_2", groupName: "Arcade High Scores", title: "Last week's finals", description: "", category: "gaming", access: "public", startsAt: at(-50), endsAt: at(-47), image: "", interested: 30, friends: 1, featured: false },
    ].map((e) => ({ ...e, following: demoFollowed.has(e.id) }));
    const wide = [
      { id: "cal_demo_9", groupId: "grp_demo_3", groupName: "VRChat", title: "Community meetup, live from the Hub", description: "Open to everyone, on now.", category: "hangout", access: "public", startsAt: at(-0.5), endsAt: at(1.5), image: "", interested: 2140, friends: 2, featured: true, following: demoFollowed.has("cal_demo_9") },
      ...all.filter((e) => Date.parse(e.endsAt) > Date.now() && e.access === "public"),
    ];
    const events = which === "discover" ? wide : which === "following" ? all.filter((e) => e.following) : which === "featured" ? all.filter((e) => e.featured) : all;
    return { events, more: false };
  },
  async followEvent(_groupId, eventId, on) {
    await wait(250);
    if (on) demoFollowed.add(eventId);
    else demoFollowed.delete(eventId);
  },
  async personBrief(userId) {
    await wait(300);
    const like = people[0];
    const m = demoBlocked.find((b) => b.userId === userId);
    return { ...like, id: userId, displayName: m?.name ?? "Someone", image: "", state: "offline", status: "busy", statusDescription: "", rank: "Known User", vrcPlus: true, adult: false };
  },
  async setModeration(userId, kind, on) {
    await wait(250);
    const at = demoBlocked.findIndex((m) => m.userId === userId && m.kind === kind);
    if (at >= 0) demoBlocked.splice(at, 1);
    if (on) demoBlocked.unshift({ kind, userId, name: "Someone", at: new Date().toISOString() });
  },
  async friendStatus() {
    return { isFriend: false, outgoingRequest: false, incomingRequest: false };
  },
  async renameFavoriteGroup(_kind, group, name) {
    await wait(200);
    demoNames.set(group, name);
  },
  async loadPicture(url) {
    return url;
  },
  async favoritesCached() {
    return null;
  },
  async groupAction(_id, action) {
    await wait(300);
    demoMember = action === "join";
  },
  async avatarHistory() {
    await wait(200);
    return demoAvatars;
  },
  async searchFeed(query, kinds, before, until, limit) {
    await wait(150);
    const q = query.trim().toLowerCase();
    return feed
      .filter(
        (f) =>
          (before === undefined || f.id < before) &&
          (!until || f.at < until) &&
          (kinds.length === 0 || kinds.includes(f.kind)) &&
          (!q ||
            f.displayName.toLowerCase().includes(q) ||
            f.detail.toLowerCase().includes(q) ||
            (worlds.find((w) => w.id === f.worldId)?.name.toLowerCase().includes(q) ?? false)),
      )
      .slice(0, limit);
  },
  async lastSeen() {
    const out: Record<string, string> = {};
    people.forEach((p, i) => {
      out[p.id] = new Date(Date.now() - i * i * 5 * 86400000).toISOString();
    });
    return out;
  },
  async ratings() {
    return { ...demoRatings };
  },
  async setRating(itemId, stars) {
    if (stars > 0) demoRatings[itemId] = stars;
    else delete demoRatings[itemId];
  },
  async worldVisits() {
    const out: Record<string, [number, string]> = {};
    for (const v of demoVisits) out[v.worldId] = [(out[v.worldId]?.[0] ?? 0) + 1, v.joinedAt];
    return out;
  },
  async tags() {
    return JSON.parse(JSON.stringify(demoTags));
  },
  async setTags(itemId, tags) {
    const clean = [...new Set(tags.map((t) => t.trim()).filter(Boolean))];
    if (clean.length) demoTags[itemId] = clean;
    else delete demoTags[itemId];
    return clean;
  },
  async wantWorld() {},
  async setOwnFeed() {},
  async setQuiet() {},
  async togetherCounts() {
    return Object.fromEntries(people.slice(0, 8).map((p, i) => [p.id, 8 - i]));
  },
  async localFavorites(kind) {
    return demoLocal.filter((f) => f.kind === kind);
  },
  async addLocalFavorite(favorite) {
    const at = demoLocal.findIndex((f) => f.kind === favorite.kind && f.itemId === favorite.itemId);
    if (at >= 0) demoLocal.splice(at, 1);
    demoLocal.unshift({ ...favorite, addedAt: new Date().toISOString() });
  },
  async removeLocalFavorite(kind, itemId) {
    const at = demoLocal.findIndex((f) => f.kind === kind && f.itemId === itemId);
    if (at >= 0) demoLocal.splice(at, 1);
  },
  async note(id) {
    return demoNotes.get(id) ?? "";
  },
  async setStatus(status, text) {
    await wait(200);
    demoMe.status = status;
    demoMe.statusDescription = text;
  },
  async serviceStatus() {
    return { indicator: "none", description: "All Systems Operational" };
  },
  async boop() {
    await wait(250);
  },
  async savePicture(_url, name) {
    await wait(300);
    return `C:\\Users\\you\\Pictures\\Anonymous VRX\\${name}.png`;
  },
  async openPicturesFolder() {},
  async profilePart(userId, part) {
    await wait(250);
    const mine = userId === "usr_demo_me";
    if (part === "mutualFriends") return people.slice(3, 6).map((p) => ({ id: p.id, displayName: p.displayName }));
    if (part === "mutualGroups") return [{ groupId: "grp_demo_1", name: "Night Owls", memberCount: 412 }];
    if (part === "groups") return ["Night Owls", "Pier Regulars", "Arcade High Scores"].map((n, i) => ({ groupId: `grp_demo_${i + 1}`, name: n, memberCount: 412 * (i + 1) }));
    if (part === "represented") return { groupId: "grp_demo_1", name: "Night Owls", memberCount: 412 };
    if (part === "worlds" || part === "favoriteWorlds") return worlds.slice(0, part === "worlds" ? (mine ? 2 : 1) : 3).map((w, i) => ({ id: w.id, name: w.name, authorName: "Rook", visits: 12840, occupants: 4, favorites: 310, releaseStatus: i ? "private" : "public", updated_at: ago(60 * 24 * (i + 2)) }));
    if (part === "avatars") return ["Robot", "Fox"].map((n, i) => ({ id: `avtr_demo_${n}`, name: mine ? `My ${n}` : n, releaseStatus: mine && i ? "private" : "public", performance: { pc_rating: "Good", android_rating: "Medium" } }));
    if (part === "wearing") return { id: "avtr_demo_worn", name: mine ? "My Robot" : "Sunset Fox", authorName: mine ? "You" : "Rook" };
    return [];
  },
  async setNote(userId, note) {
    if (note.trim()) demoNotes.set(userId, note);
    else demoNotes.delete(userId);
  },
  async noted() {
    return [...demoNotes.keys()];
  },
  async friendLog() {
    return [
      { id: 903, at: ago(30), userId: "usr_demo_60", displayName: "Vesper", kind: "friendAdded", detail: "", previous: "", location: "", worldId: null },
      { id: 902, at: ago(60 * 20), userId: "usr_demo_2", displayName: "Pip Halloway", kind: "name", detail: "Pip Halloway", previous: "Pip", location: "", worldId: null },
      { id: 901, at: ago(60 * 50), userId: "usr_demo_gone", displayName: "Marlowe", kind: "friendRemoved", detail: "", previous: "", location: "", worldId: null },
    ];
  },
  async searchUsers(query) {
    await wait(300);
    const q = query.toLowerCase();
    return people.filter((p) => p.displayName.toLowerCase().includes(q)).slice(0, 24).map(demoPerson);
  },
  async searchWorlds(query) {
    await wait(300);
    const q = query.toLowerCase();
    return worlds
      .filter((w) => w.name.toLowerCase().includes(q))
      .map((w) => ({ id: w.id, name: w.name, author: "Rook", description: "", image: "", capacity: 32, visits: 12840, favorites: 310, occupants: 6, tags: ["chill", "sunset"] }));
  },
  async inviteHere() {
    await wait(300);
  },
  async searchAvatars(_provider, query) {
    await wait(300);
    return ["Robot", "Fox", "Knight"].map((n, i) => ({
      id: `avtr_demo_${i}`,
      name: `${query} ${n}`,
      author: people[i].displayName,
      authorId: people[i].id,
      description: "",
      image: "",
      pc: ["Good", "Medium", "Very Poor"][i],
      mobile: ["Medium", "Poor", ""][i],
    }));
  },
  async wearAvatar() {
    await wait(300);
  },
  async openLink() {},
  async notices() {
    return demoNotices;
  },
  async noticeAction(id, action, replyKind, replyData) {
    await wait(300);
    const notice = demoNotices.find((n) => n.id === id);
    if (notice) {
      const replyText = notice.replies.find((r) => r.kind === replyKind && r.data === replyData)?.text ?? "";
      demoHandled.unshift({ id: demoHandled.length + 1, at: new Date().toISOString(), action: action as Handled["action"], replyText, notice });
    }
    demoNotices = demoNotices.filter((n) => n.id !== id);
    emit("notices:changed", null);
  },
  async worlds() {
    return worlds;
  },
  async instances() {
    return [
      { location: spots[0], users: 14, capacity: 32, full: false },
      { location: spots[1], users: 40, capacity: 40, full: true },
      { location: spots[3], users: 9, capacity: 24, full: false },
    ];
  },
  async createInstance(worldId, access, region) {
    await wait(500);
    const kind = { public: "", friendsPlus: "~hidden(usr_demo_me)", friends: "~friends(usr_demo_me)", invitePlus: "~private(usr_demo_me)~canRequestInvite", invite: "~private(usr_demo_me)" }[access] ?? "";
    return `${worldId}:${10000 + Math.floor(Math.random() * 89999)}${kind}~region(${region})`;
  },
  async inviteMyself() {
    await wait(300);
  },
  async requestInvite() {
    await wait(300);
  },
  async openJoinPage() {},
  async launchInstance(_location, mode) {
    await wait(200);
    return mode === "vr" ? "vr" : "desktop";
  },
  async feed(limit, before) {
    return feed.filter((f) => before === undefined || f.id < before).slice(0, limit);
  },
  async pipelineStatus() {
    return signedIn ? "live" : "off";
  },
  async here() {
    return { visit: demoVisits[0], people: demoPeople };
  },
  async visits(limit, before) {
    return demoVisits.filter((v) => before === undefined || v.id < before).slice(0, limit);
  },
  async visitPeople(visitId) {
    return visitId === 4 ? demoPeople : demoPeople.slice(0, 3).map((p) => ({ ...p, leftAt: ago(41) }));
  },
  async on(name, handler) {
    if (!handlers.has(name)) handlers.set(name, new Set());
    handlers.get(name)!.add(handler as Handler);
    return () => handlers.get(name)?.delete(handler as Handler);
  },
};
