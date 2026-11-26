import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { demoBackend } from "./demo";

export interface Place {
  kind: "world" | "private" | "traveling" | "none";
  worldId: string | null;
  instance: string | null;
  access: string | null;
  region: string | null;
  groupId: string | null;
}

export interface Friend {
  id: string;
  displayName: string;
  state: "online" | "active" | "offline";
  status: string;
  statusDescription: string;
  location: string;
  travelingTo: string;
  place: Place;
  image: string;
  platform: string;
  avatarImage: string;
  /** When they were first seen at this location; empty when unknown. */
  since: string;
  /** True when the real stay is at least this long (they were there before the app looked). */
  sinceAtLeast: boolean;
  /** Trust rank in words; empty until VRChat sends it. */
  rank?: string;
  vrcPlus?: boolean;
  /** Age verified as 18+. */
  adult?: boolean;
}

export interface AvatarWorn {
  avatarId: string;
  name: string;
  authorId: string;
  authorName: string;
  image: string;
  releaseStatus: string;
  lastWorn: string;
  times: number;
  seconds: number;
  /** The part of `seconds` spent inside a world. */
  secondsInGame: number;
  /** How many stays in a world it was worn in. */
  visits: number;
  /** The last moment it was on inside a world, or empty. */
  lastInGame: string;
}

export interface LocalFavorite {
  kind: "avatar" | "world";
  itemId: string;
  name: string;
  authorName: string;
  image: string;
  addedAt?: string;
}

export interface ServiceStatus {
  /** `none`, `minor`, `major` or `critical`. */
  indicator: string;
  description: string;
}

export interface Instance {
  location: string;
  users: number;
  capacity: number;
  full: boolean;
}

export interface World {
  id: string;
  name: string;
  image: string;
}

export interface FeedItem {
  id: number;
  at: string;
  userId: string;
  displayName: string;
  kind:
    | "online"
    | "offline"
    | "location"
    | "status"
    | "statusText"
    | "name"
    | "avatar"
    | "friendAdded"
    | "friendRemoved";
  detail: string;
  previous: string;
  location: string;
  worldId: string | null;
}

export interface Visit {
  id: number;
  location: string;
  worldId: string;
  worldName: string;
  joinedAt: string;
  leftAt: string | null;
  place: Place;
  people: number;
}

/** One VRChat photo on this PC, with where and with whom it was taken when the history knows. */
export interface PhotoInfo {
  path: string;
  /** An address the window can show the picture from. */
  url: string;
  file: string;
  takenAt: string;
  width: number;
  height: number;
  worldId: string;
  worldName: string;
  location: string;
  people: Array<{ userId: string; displayName: string }>;
}

export interface Sighting {
  userId: string;
  displayName: string;
  joinedAt: string;
  leftAt: string | null;
}

/** What Discord shows on my profile. Every text is decided in the window. */
export interface DiscordPresence {
  clientId: string;
  details: string;
  state: string;
  /** Seconds since 1970, or nothing. */
  startedAt: number | null;
  image: string;
  imageText: string;
  buttonLabel: string;
  buttonUrl: string;
  button2Label: string;
  button2Url: string;
  /** The corner picture: an art asset name from the Discord application, or a picture address. */
  smallImage: string;
  smallText: string;
}

export interface HereNow {
  visit: Visit;
  people: Sighting[];
}

/** One event on a group's calendar. */
/** One icon frame, profile effect or nameplate effect VRChat offers. */
export interface Cosmetic {
  /** The moving picture, or the still one when there is no other. */
  picture: string;
  /** The still picture. */
  still: string;
  /** Two colours ("aabbcc") a nameplate effect blends between; empty for the others. */
  from: string;
  to: string;
}

/** How one of my worlds stood on one day. */
export interface WorldDay {
  worldId: string;
  /** "2026-11-25", in the PC's own time. */
  day: string;
  /** Visits and favourites in total up to that day. */
  visits: number;
  favorites: number;
  /** The most people seen in it that day, at the times the app looked. */
  occupants: number;
}

export interface GameCache {
  path: string;
  found: boolean;
  bytes: number;
  /** Worlds and avatars kept. */
  items: number;
  /** Versions a newer download has replaced, and how much room they take. */
  oldVersions: number;
  oldBytes: number;
  limitGb: number;
}

export interface VrRules {
  /** The app's own notice panel in SteamVR. */
  own: boolean;
  xs: boolean;
  ovrHud: boolean;
  ovrWrist: boolean;
  seconds: number;
  opacity: number;
}

export interface CalendarEvent {
  id: string;
  groupId: string;
  groupName: string;
  title: string;
  description: string;
  category: string;
  access: string;
  startsAt: string;
  endsAt: string;
  image: string;
  interested: number;
  friends: number;
  following: boolean;
  featured: boolean;
}

/** Someone I blocked or muted. */
export interface Moderation {
  kind: "block" | "mute";
  userId: string;
  name: string;
  at: string;
}

export interface Favorites {
  groups: Array<{ key: string; name: string }>;
  /** Friend user id to group key. */
  friends: Record<string, string>;
}

export interface Notice {
  id: string;
  version: 1 | 2;
  kind: string;
  senderId: string;
  senderName: string;
  title: string;
  message: string;
  at: string;
  seen: boolean;
  location: string;
  worldName: string;
  image: string;
  replies: Array<{ kind: string; data: string; text: string }>;
  /** Where a newer notice points, such as `group:grp_...`. Missing on ones kept by an older version. */
  link?: string;
}

/** A notification you already dealt with. */
export interface Handled {
  id: number;
  at: string;
  action: "accept" | "dismiss" | "reply" | "inviteBack" | "boopBack" | "elsewhere";
  replyText: string;
  notice: Notice;
}

export interface Person {
  id: string;
  displayName: string;
  image: string;
  bio: string;
  bioLinks: string[];
  pronouns: string;
  status: string;
  statusDescription: string;
  rank: string;
  joined: string;
  lastLogin: string;
  lastPlatform: string;
  isFriend: boolean;
  location: string;
  vrchatNote: string;
}

/** The colour of the dot for a status word VRChat uses. */
export function statusDot(status: string): string {
  return status === "join me" ? "join" : status === "ask me" ? "ask" : status === "busy" ? "busy" : status === "active" ? "online" : "offline";
}

export interface WorldCard {
  id: string;
  name: string;
  author: string;
  description: string;
  image: string;
  capacity: number;
  visits: number;
  favorites: number;
  occupants: number;
  tags: string[];
}

export interface AvatarCard {
  id: string;
  name: string;
  author: string;
  authorId: string;
  description: string;
  image: string;
  pc: string;
  mobile: string;
}

export interface Profile {
  person: Person;
  /** My own note, kept on this PC. */
  note: string;
  together: { times: number; seconds: number; first: string | null; last: string | null };
  feed: FeedItem[];
  /** Names this PC saw them use: [name, when last seen]. */
  pastNames?: Array<[string, string]>;
  /** The profile exactly as VRChat sent it. */
  raw: unknown;
}


export interface BackupInfo {
  name: string;
  path: string;
  bytes: number;
}

export interface Me {
  id: string;
  displayName: string;
  status: string;
  statusDescription: string;
  image: string;
}

export interface SignIn {
  status: "signedIn" | "signedOut" | "needsCode";
  me: Me | null;
  methods: string[];
}

export interface FavouriteCheck {
  dead: Array<{ entryId: string; favoriteId: string; why: "deleted" | "private"; name: string }>;
  checked: number;
  left: number;
  stopped: boolean;
}

export interface BackendError {
  kind: string;
  message: string;
}

export type LiveEvents = {
  "friends:reset": null;
  "favorites:reset": null;
  "friend:changed": Friend;
  "friend:removed": string;
  "feed:new": FeedItem;
  "notices:changed": null;
  /** VRChat closed with no goodbye while I was in an instance. `rejoined` says the app already started it again. */
  "game:crashed": { location: string; worldName: string; rejoined: boolean };
  "avatars:changed": null;
  "notice:new": Notice;
  "here:changed": null;
  /** VRChat closed and its cache was emptied, as asked for in Settings. */
  "cache:cleared": { bytes: number };
  /** How far the download of an update is, in bytes. `total` is 0 when the size is not known. */
  "update:progress": { done: number; total: number };
  /** A notice for the app's own panel in the headset: the window draws it and hands it back. */
  "vr:draw": { title: string; body: string };
  "world:resolved": World;
  "instance:info": Instance;
  "pipeline:status": string;
  "session:ended": null;
  "window:close-asked": null;
  "sync:error": BackendError;
};


export interface Backend {
  demo: boolean;
  restore(): Promise<SignIn>;
  login(username: string, password: string): Promise<SignIn>;
  verifyCode(method: string, code: string): Promise<SignIn>;
  logout(): Promise<void>;
  friends(): Promise<Friend[]>;
  favorites(): Promise<Favorites>;
  notices(): Promise<Notice[]>;
  handled(limit: number): Promise<Handled[]>;
  profile(userId: string): Promise<Profile>;
  /** One tab of a profile: `mutualFriends`, `mutualGroups`, `groups`, `represented`, `worlds`, `favoriteWorlds`, `avatars`, `wearing`. */
  profilePart(userId: string, part: string, extra: string, provider: string): Promise<unknown>;
  /** A world, avatar or group as VRChat describes it; also `groupMembers` and `groupPosts`. */
  thing(kind: string, id: string): Promise<unknown>;
  /** VRChat's photos on this PC, newest first, `limit` at a time after skipping `skip`. */
  /** `query` keeps photos whose world or one of whose people matches; `until` is a day, YYYY-MM-DD. `everything` counts all photos, `total` the ones that match. */
  photos(limit: number, skip: number, query?: string, until?: string): Promise<{ total: number; everything: number; photos: PhotoInfo[] }>;
  /** Opens the photo's folder with the file selected. */
  showPhoto(path: string): Promise<void>;
  /** Opens the folder with the credits and licence texts. */
  openLicencesFolder(): Promise<void>;
  /** Writes the window's settings to a file at once. */
  saveSettings(text: string): Promise<void>;
  /** The settings file, if there is one. */
  loadSettings(): Promise<string | null>;
  /** Sets what Discord shows on my profile; nothing clears it. Only reaches the Discord app on this PC. */
  setDiscord(presence: DiscordPresence | null): Promise<void>;
  /** Changes one of my own uploads: make it `public` or `private`, or `delete` it for good. */
  uploadAction(kind: "avatar" | "world", id: string, action: "public" | "private" | "delete"): Promise<void>;
  /** `add` sends a friend request, `cancel` takes it back, `remove` unfriends. */
  friendAction(userId: string, action: "add" | "cancel" | "remove"): Promise<void>;
  friendStatus(userId: string): Promise<{ isFriend?: boolean; outgoingRequest?: boolean; incomingRequest?: boolean }>;
  /** The people I blocked or muted, newest first. `fresh` asks VRChat again. */
  moderations(fresh: boolean): Promise<Moderation[]>;
  /** One month of events (`month` is YYYY-MM): my groups', the ones I follow, or VRChat's featured ones.
   *  `discover` has no month: it is what is on now and coming up across all of VRChat. */
  calendar(which: "mine" | "following" | "featured" | "discover", month: string, fresh?: boolean): Promise<{ events: CalendarEvent[]; more: boolean }>;
  /** Follows a group event, or stops following it. */
  followEvent(groupId: string, eventId: string, on: boolean): Promise<void>;
  /** Picture, rank, marks and status of someone, for a list row. Asked once per person. */
  personBrief(userId: string): Promise<Friend>;
  /** Blocks or mutes someone, or takes it back with `on` false. */
  setModeration(userId: string, kind: "block" | "mute", on: boolean): Promise<void>;
  renameFavoriteGroup(kind: string, group: string, name: string): Promise<void>;
  /** A picture as a data address, fetched with the sign-in when VRChat wants it. */
  loadPicture(url: string): Promise<string>;
  /** The favourites answer from last time, or null. Instant. */
  favoritesCached(kind: string): Promise<unknown>;
  /** `join` or `leave` a group. */
  groupAction(groupId: string, action: "join" | "leave"): Promise<void>;
  /** My favourites of one kind: `{ groups, entries, items, capacity }`. */
  favoritesOf(kind: string): Promise<unknown>;
  /** Every favourite group with its type, name and how many favourites it holds (`counts` is keyed `kind:group`). */
  /** With `counts` false only the names are asked for, which is quick. */
  favoriteGroups(counts: boolean): Promise<{ groups: Array<Record<string, unknown>>; counts: Record<string, number> }>;
  addFavorite(kind: string, id: string, group: string): Promise<void>;
  /** Removes favourites by entry id; resolves to how many were removed. */
  removeFavorites(entryIds: string[]): Promise<number>;
  /** Asks VRChat about favourites it did not list; only confirmed deleted or private ones come back. */
  checkFavorites(kind: string): Promise<FavouriteCheck>;
  /** Avatars I wore, one row each, most recent first. */
  avatarHistory(limit: number): Promise<AvatarWorn[]>;
  /** The saved feed with a search, kinds and an upper time bound; `before` pages back. */
  searchFeed(query: string, kinds: string[], before: number | undefined, until: string | undefined, limit: number): Promise<FeedItem[]>;
  /** Shared visits per user id, from the game log. */
  togetherCounts(): Promise<Record<string, number>>;
  /** User id to when this PC last saw them do anything. From my own records. */
  lastSeen(): Promise<Record<string, string>>;
  /** My own stars, by world or avatar id. They never leave this PC. */
  ratings(): Promise<Record<string, number>>;
  /** Sets my stars, 1 to 5; 0 takes them away. */
  setRating(itemId: string, stars: number): Promise<void>;
  /** Item id to the tags I gave it. They never leave this PC. */
  tags(): Promise<Record<string, string[]>>;
  /** Replaces my tags on an avatar or a world; gives back the tags as saved. */
  setTags(itemId: string, tags: string[]): Promise<string[]>;
  /** World id to [how often I was there, when last]. */
  worldVisits(): Promise<Record<string, [number, string]>>;
  /** Switches recording my own changes in the feed. */
  setOwnFeed(on: boolean): Promise<void>;
  /** Quiet hours and quiet while playing. Minutes after midnight. */
  setQuiet(rules: { hours: boolean; from: number; to: number; inGame: boolean }): Promise<void>;
  localFavorites(kind: string): Promise<LocalFavorite[]>;
  addLocalFavorite(favorite: LocalFavorite): Promise<void>;
  removeLocalFavorite(kind: string, itemId: string): Promise<void>;
  /** My note on any id: a person, an avatar or a world. */
  note(id: string): Promise<string>;
  setStatus(status: string, text: string): Promise<void>;
  serviceStatus(): Promise<ServiceStatus>;
  /** Which notifications get a Windows pop-up. */
  setAlerts(alerts: { invites: boolean; friendRequests: boolean; boops: boolean; everythingElse: boolean }): Promise<void>;
  /** Says what to do when VRChat starts, and when it closes without warning. */
  setGameWatch(watch: { openWithGame: boolean; rejoin: "off" | "ask" | "auto"; launchMode: "auto" | "desktop" | "vr"; clearCache: boolean }): Promise<void>;
  /** Whether the app starts with Windows. */
  getAutostart(): Promise<boolean>;
  setAutostart(on: boolean): Promise<void>;
  /** Boops a friend. */
  boop(userId: string): Promise<void>;
  /** Saves a picture to Pictures\Anonymous VRX; resolves to the full path. */
  savePicture(url: string, name: string): Promise<string>;
  openPicturesFolder(): Promise<void>;
  setNote(userId: string, note: string): Promise<void>;
  /** User ids I wrote a note about. */
  noted(): Promise<string[]>;
  friendLog(limit: number): Promise<FeedItem[]>;
  searchUsers(query: string): Promise<Person[]>;
  searchWorlds(query: string): Promise<WorldCard[]>;
  /** Asks a public avatar database. Only the search text leaves the app. */
  searchAvatars(provider: string, query: string): Promise<AvatarCard[]>;
  /** Switches you to an avatar. */
  wearAvatar(avatarId: string): Promise<void>;
  /** Invites a friend to the world I am in. */
  inviteHere(userId: string): Promise<void>;
  /** Opens a web link in the browser. */
  openLink(url: string): Promise<void>;
  /** `accept`, `dismiss`, `reply` (with the reply's kind and data) or `inviteBack`. */
  noticeAction(id: string, action: string, replyKind?: string, replyData?: string): Promise<void>;
  /** Shows a Windows notification. Resolves to false when Windows refused permission. Does nothing in the browser demo. */
  /** Shows a Windows notification. `always` skips the quiet rules, for the test button. */
  notify(title: string, body: string, always?: boolean): Promise<boolean>;
  worlds(): Promise<World[]>;
  /** Asks for a world's name and picture when they are not known yet; they arrive as `world:resolved`. */
  wantWorld(worldId: string): Promise<void>;
  instances(): Promise<Instance[]>;
  /** Sends you an invite to an instance; it shows up inside VRChat. */
  inviteMyself(location: string): Promise<void>;
  /** Asks a friend to invite you to where they are. */
  requestInvite(userId: string): Promise<void>;
  /** Makes a new instance of a world and gives back its location. `access`: public, friendsPlus, friends, invitePlus or invite. */
  createInstance(worldId: string, access: string, region: string): Promise<string>;
  /** Starts VRChat in that instance, or moves the running game there. Resolves to what was done. */
  launchInstance(location: string, mode: "auto" | "desktop" | "vr"): Promise<"running" | "desktop" | "vr">;
  /** Opens VRChat's join page for an instance in the browser. */
  openJoinPage(location: string): Promise<void>;
  feed(limit: number, before?: number): Promise<FeedItem[]>;
  pipelineStatus(): Promise<string>;
  /** Hides the window; the app keeps running by the clock. */
  hideToTray(): Promise<void>;
  quit(): Promise<void>;
  /** Copies history and settings to a new folder in Documents. */
  backupNow(settings: string): Promise<BackupInfo>;
  /** Whether VRCX's history file is on this PC. */
  vrcxFound(): Promise<boolean>;
  /** Brings VRCX's history for my account into this app, after making a backup. Safe to run again. */
  /** How my own worlds stood day by day over the last `days` days. Noted each time my worlds are listed. */
  worldStats(days: number): Promise<WorldDay[]>;
  /** The running version, and whether this edition can update itself. */
  updateInfo(): Promise<{ version: string; supported: boolean }>;
  /** Asks whether a newer version is out. */
  updateCheck(): Promise<{ version: string; notes: string } | null>;
  /** Downloads and installs the update that was found; the app closes and opens again as the new version. */
  updateInstall(): Promise<void>;
  /** Every icon frame, profile effect and nameplate effect, by the id a profile names. Asked for once a day. */
  cosmetics(): Promise<Record<string, Cosmetic>>;
  /** Changes whenever VRCX has written to its history. Empty when VRCX is not on this PC. */
  vrcxStamp(): Promise<string>;
  /** Draw without the graphics card, from the next start on. */
  getNoGpu(): Promise<{ wanted: boolean; now: boolean }>;
  setNoGpu(on: boolean): Promise<void>;
  /** Closes the app and opens it again. */
  restartApp(): Promise<void>;
  /** VRChat's own download cache on this PC. */
  gameCache(): Promise<GameCache>;
  /** Clears it: everything, or only versions a newer download replaced. Refused while VRChat runs. */
  clearGameCache(oldOnly: boolean): Promise<{ bytes: number; folders: number; kept: number }>;
  /** How big VRChat lets the cache grow, in GB. Gives back what was written (never under 30). */
  setGameCacheLimit(gb: number): Promise<number>;
  /** Which overlay programs get my notifications in the headset. */
  setVrNotices(rules: VrRules): Promise<void>;
  /** Whether the app should be in SteamVR at all: a notice panel or a wrist feed is switched on. */
  /** Shows a picture (a PNG data address) in front of the eyes for some seconds. */
  /** Puts a picture on a wrist, or takes the feed off with `null`. `width` is in metres. */
  /** Where the app's own panels stand, in words. */
  /** Sends a sample notice now and says how each program took it. */
  testVrNotice(rules: VrRules): Promise<Array<{ target: string; ok: boolean; problem: string }>>;
  /** `byItself` is the quiet run the app makes when VRCX's file has changed. */
  importVrcx(settings: string, byItself?: boolean, since?: string): Promise<{ broughtOver: { feed: number; friendLog: number; visits: number; people: number; notes: number; favourites: number }; backup: string }>;
  /** Makes the daily automatic backup if one is due and keeps the newest `keep`. Null when none was due. */
  autoBackup(settings: string, keep: number): Promise<BackupInfo | null>;
  backups(): Promise<BackupInfo[]>;
  /** Lines a backup up for the next start; resolves to that backup's settings text. */
  restoreBackup(name: string, settings: string): Promise<string>;
  openBackupsFolder(): Promise<void>;
  here(): Promise<HereNow | null>;
  visits(limit: number, before?: number): Promise<Visit[]>;
  visitPeople(visitId: number): Promise<Sighting[]>;
  on<K extends keyof LiveEvents>(
    name: K,
    handler: (payload: LiveEvents[K]) => void,
  ): Promise<() => void>;
}

/** Answers kept for a short while, so going back to a window shows at once. */
const kept = new Map<string, { at: number; value: Promise<unknown> }>();

function remembered<T>(key: string, ms: number, load: () => Promise<T>): Promise<T> {
  const hit = kept.get(key);
  if (hit && Date.now() - hit.at < ms) return hit.value as Promise<T>;
  const value = load();
  kept.set(key, { at: Date.now(), value });
  value.catch(() => kept.delete(key));
  if (kept.size > 400) kept.delete(kept.keys().next().value as string);
  return value;
}

function forget(prefix: string) {
  for (const key of [...kept.keys()]) if (key.startsWith(prefix)) kept.delete(key);
}

const desktop: Backend = {
  demo: false,
  restore: () => invoke("session_restore"),
  login: (username, password) => invoke("login", { username, password }),
  verifyCode: (method, code) => invoke("verify_code", { method, code }),
  logout: () => invoke("logout"),
  friends: () => invoke("get_friends"),
  favorites: () => invoke("get_favorites"),
  notices: () => invoke("get_notices"),
  handled: (limit) => invoke("get_handled", { limit }),
  profile: (userId) => remembered(`profile:${userId}`, 60_000, () => invoke("get_profile", { userId })),
  profilePart: (userId, part, extra, provider) =>
    remembered(`part:${userId}:${part}:${extra}`, 120_000, () => invoke("get_profile_part", { userId, part, extra, provider })),
  thing: (kind, id) =>
    remembered(`thing:${kind}:${id}`, kind === "instance" || kind === "groupInstances" ? 20_000 : 120_000, () => invoke("get_thing", { kind, id })),
  favoritesCached: (kind) => invoke("get_favorites_cached", { kind }),
  friendAction: async (userId, action) => {
    await invoke("friend_action", { userId, action });
    forget(`profile:${userId}`);
  },
  friendStatus: (userId) => invoke("get_friend_status", { userId }),
  moderations: (fresh) => invoke("get_moderations", { fresh }),
  personBrief: (userId) => invoke("get_person_brief", { userId }),
  calendar: (which, month, fresh) => invoke("get_calendar", { which, month, fresh: fresh ?? false }),
  followEvent: async (groupId, eventId, on) => {
    await invoke("follow_event", { groupId, eventId, on });
  },
  setModeration: async (userId, kind, on) => {
    await invoke("set_moderation", { userId, kind, on });
  },
  uploadAction: async (kind, id, action) => {
    await invoke("upload_action", { kind, id, action });
    forget(`thing:${kind}:${id}`);
    forget("part:");
  },
  photos: async (limit, skip, query, until) => {
    const page = await invoke<{ total: number; everything: number; photos: Array<Omit<PhotoInfo, "url">> }>("photos", { limit, skip, query: query ?? "", until: until ?? "" });
    return { total: page.total, everything: page.everything, photos: page.photos.map((p) => ({ ...p, url: convertFileSrc(p.path) })) };
  },
  showPhoto: (path) => invoke("show_photo", { path }),
  openLicencesFolder: () => invoke("open_licences_folder"),
  setDiscord: (presence) => invoke("set_discord", { presence }),
  saveSettings: (text) => invoke("save_settings", { text }),
  loadSettings: () => invoke("load_settings"),
  renameFavoriteGroup: (kind, group, name) => invoke("rename_favorite_group", { kind, group, name }),
  loadPicture: (url) => invoke("load_picture", { url }),
  groupAction: async (groupId, action) => {
    await invoke("group_action", { groupId, action });
    forget(`thing:group:${groupId}`);
    forget("part:");
  },
  favoritesOf: (kind) => invoke("get_favorites_of", { kind }),
  favoriteGroups: (counts) => invoke("get_favorite_groups", { counts }),
  addFavorite: (kind, id, group) => invoke("add_favorite", { kind, id, group }),
  removeFavorites: (entryIds) => invoke("remove_favorites", { entryIds }),
  checkFavorites: (kind) => invoke("check_favorites", { kind }),
  boop: (userId) => invoke("boop", { userId }),
  setAlerts: (alerts) => invoke("set_alerts", { alerts }),
  setGameWatch: (watch) => invoke("set_game_watch", { watch }),
  getAutostart: () => invoke("get_autostart"),
  setAutostart: (on) => invoke("set_autostart", { on }),
  avatarHistory: (limit) => invoke("get_avatar_history", { limit }),
  searchFeed: (query, kinds, before, until, limit) =>
    invoke("search_feed", { query, kinds, before: before ?? null, until: until ?? null, limit }),
  togetherCounts: () => invoke("get_together_counts"),
  lastSeen: () => invoke("get_last_seen"),
  ratings: () => invoke("get_ratings"),
  setRating: (itemId, stars) => invoke("set_rating", { itemId, stars }),
  worldVisits: () => invoke("get_world_visits"),
  tags: () => invoke("get_tags"),
  setTags: (itemId, tags) => invoke("set_tags", { itemId, tags }),
  setOwnFeed: (on) => invoke("set_own_feed", { on }),
  setQuiet: (rules) => invoke("set_quiet", { rules }),
  localFavorites: (kind) => invoke("get_local_favorites", { kind }),
  addLocalFavorite: (favorite) => invoke("add_local_favorite", { favorite }),
  removeLocalFavorite: (kind, itemId) => invoke("remove_local_favorite", { kind, itemId }),
  note: (id) => invoke("get_note", { id }),
  setStatus: (status, text) => invoke("set_status", { status, text }),
  serviceStatus: () => invoke("get_service_status"),
  savePicture: (url, name) => invoke("save_picture", { url, name }),
  openPicturesFolder: () => invoke("open_pictures_folder"),
  setNote: async (userId, note) => {
    await invoke("set_note", { userId, note });
    forget(`profile:${userId}`);
  },
  noted: () => invoke("get_noted"),
  friendLog: (limit) => invoke("get_friend_log", { limit }),
  searchUsers: (query) => invoke("search_users", { query }),
  searchWorlds: (query) => invoke("search_worlds", { query }),
  searchAvatars: (provider, query) => invoke("search_avatars", { provider, query }),
  wearAvatar: (avatarId) => invoke("wear_avatar", { avatarId }),
  inviteHere: (userId) => invoke("invite_here", { userId }),
  openLink: (url) => invoke("open_link", { url }),
  noticeAction: (id, action, replyKind, replyData) =>
    invoke("notice_action", { id, action, replyKind: replyKind ?? null, replyData: replyData ?? null }),
  notify: async (title, body, always) => {
    // The headset gets it as well, by its own rules.
    if (!always) void invoke("notify_vr", { title, body }).catch(() => {});
    if (!always && !(await invoke<boolean>("may_notify"))) return false;
    let allowed = await isPermissionGranted();
    if (!allowed) allowed = (await requestPermission()) === "granted";
    if (allowed) sendNotification({ title, body });
    return allowed;
  },
  worlds: () => invoke("get_worlds"),
  wantWorld: (worldId) => invoke("want_world", { worldId }),
  instances: () => invoke("get_instances"),
  inviteMyself: (location) => invoke("invite_myself", { location }),
  requestInvite: (userId) => invoke("request_invite", { userId }),
  createInstance: (worldId, access, region) => invoke("create_instance", { worldId, access, region }),
  openJoinPage: (location) => invoke("open_join_page", { location }),
  launchInstance: (location, mode) => invoke("launch_instance", { location, mode }),
  feed: (limit, before) => invoke("get_feed", { limit, before: before ?? null }),
  pipelineStatus: () => invoke("get_pipeline_status"),
  hideToTray: () => invoke("hide_to_tray"),
  quit: () => invoke("quit_app"),
  backupNow: (settings) => invoke("backup_now", { settings }),
  backups: () => invoke("list_backups"),
  restoreBackup: (name, settings) => invoke("restore_backup", { name, settings }),
  autoBackup: (settings, keep) => invoke("auto_backup", { settings, keep }),
  vrcxFound: () => invoke("vrcx_found"),
  importVrcx: (settings, byItself, since) => invoke("import_vrcx", { settings, byItself: byItself ?? false, since: since ?? null }),
  vrcxStamp: () => invoke("vrcx_stamp"),
  cosmetics: () => invoke("get_cosmetics"),
  worldStats: (days) => invoke("get_world_stats", { days }),
  updateInfo: () => invoke("update_info"),
  updateCheck: () => invoke("update_check"),
  updateInstall: () => invoke("update_install"),
  getNoGpu: () => invoke("get_no_gpu"),
  setNoGpu: (on) => invoke("set_no_gpu", { on }),
  restartApp: () => invoke("restart_app"),
  gameCache: () => invoke("game_cache"),
  clearGameCache: (oldOnly) => invoke("clear_game_cache", { oldOnly }),
  setGameCacheLimit: (gb) => invoke("set_game_cache_limit", { gb }),
  setVrNotices: (rules) => invoke("set_vr_notices", { rules }),
  testVrNotice: (rules) => invoke("test_vr_notice", { rules }),
  openBackupsFolder: () => invoke("open_backups_folder"),
  here: () => invoke("get_here"),
  visits: (limit, before) => invoke("get_visits", { limit, before: before ?? null }),
  visitPeople: (visitId) => invoke("get_visit_people", { visitId }),
  on: (name, handler) => listen(name, (e) => handler(e.payload as never)),
};

const insideDesktopWindow = "__TAURI_INTERNALS__" in window;

/** Outside the desktop window (a plain browser tab) the app runs on made-up data. */
export const backend: Backend = insideDesktopWindow ? desktop : demoBackend;

export function errorText(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) {
    return String((e as BackendError).message);
  }
  return String(e);
}
