import { type CSSProperties, useEffect, useState, type ReactNode } from "react";
import { backend, errorText, type Friend, type Profile, type World } from "./backend";
import { Avatar, accessName, clock, dateText, dayLabel, deviceLine, statusTone, statusWords, tidy } from "./bits";
import { WayBack } from "./History";
import { openThing } from "./Detail";
import { live, savedAvatarProvider } from "./settings";
import { openPicture } from "./Viewer";
import { usePic } from "./Pic";
import { AddFriend, BlockMute, FriendFavourite, Unfriend, type BlockState } from "./FriendTools";

/** Any part of the window can ask for someone's profile this way. */
export function openProfile(userId: string) {
  window.dispatchEvent(new CustomEvent("open-profile", { detail: userId }));
}

type Json = Record<string, unknown>;
/**
 * The picture behind an icon frame or a profile effect. VRChat sends an address, or the id of
 * a file it keeps; anything else is left out.
 */
function cosmeticPicture(value: unknown): string {
  const text = typeof value === "string" ? value.trim() : "";
  if (/^(https:\/\/|data:image\/)/.test(text)) return text;
  if (/^file_[0-9a-f-]{36}$/.test(text)) return `https://api.vrchat.cloud/api/1/file/${text}/1/file`;
  return "";
}

/** A banner colour as CSS can use it: "#aabbcc" from "#aabbcc" or "aabbcc"; empty for anything else. */
function colourOf(value: unknown): string {
  const text = typeof value === "string" ? value.trim().replace(/^#/, "") : "";
  return /^[0-9a-f]{6}$/i.test(text) ? `#${text}` : "";
}

type Tab = "info" | "mutuals" | "groups" | "worlds" | "favoriteWorlds" | "avatars" | "activity" | "json";

const tabNames: Array<[Tab, string]> = [
  ["info", "Info"],
  ["mutuals", "Mutual friends"],
  ["groups", "Groups"],
  ["worlds", "Worlds"],
  ["favoriteWorlds", "Favourite worlds"],
  ["avatars", "Avatars"],
  ["activity", "Activity"],
  ["json", "JSON"],
];

const NL_CHAR = String.fromCharCode(10);
/** The tab each open profile was on, so Back returns to it. */
const openTabs = new Map<string, Tab>();
const str = (v: unknown): string => (typeof v === "string" ? v : "");
const num = (v: unknown): number => (typeof v === "number" ? v : 0);
const list = (v: unknown): Json[] => (Array.isArray(v) ? (v.filter((x) => x && typeof x === "object") as Json[]) : []);

function longTime(seconds: number): string {
  const minutes = Math.round(seconds / 60);
  if (minutes < 1) return "under a minute";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours} h ${minutes % 60} min`;
  return `${Math.floor(hours / 24)} days ${hours % 24} h`;
}

function day(iso: string): string {
  return dateText(iso);
}

function ago(iso: string): string {
  if (!iso) return "";
  const minutes = Math.max(0, Math.round((Date.now() - Date.parse(iso)) / 60_000));
  if (minutes < 60) return `${minutes} min ago`;
  if (minutes < 60 * 48) return `${Math.floor(minutes / 60)} h ago`;
  const days = Math.floor(minutes / 1440);
  return days === 1 ? "yesterday" : `${days} days ago`;
}

const feedWords: Record<string, string> = {
  online: "Came online",
  offline: "Left VRChat",
  location: "Changed world",
  status: "Changed status",
  statusText: "Changed status text",
  name: "Changed name",
  avatar: "Changed avatar",
  friendAdded: "Became your friend",
  friendRemoved: "Left your friend list",
};

const platformWords: Record<string, string> = {
  standalonewindows: "PC",
  android: "Quest or Android",
  ios: "iPhone or iPad",
  web: "Website",
};

const languages: Record<string, string> = {
  eng: "English", spa: "Spanish", fra: "French", deu: "German", jpn: "Japanese", kor: "Korean", zho: "Chinese",
  rus: "Russian", por: "Portuguese", ita: "Italian", nld: "Dutch", pol: "Polish", swe: "Swedish", nor: "Norwegian",
  dan: "Danish", fin: "Finnish", tur: "Turkish", ara: "Arabic", tha: "Thai", vie: "Vietnamese", ind: "Indonesian",
  ukr: "Ukrainian", ces: "Czech", hun: "Hungarian", ron: "Romanian", heb: "Hebrew", ase: "American Sign Language",
};

/** My own account settings that VRChat includes when I look at myself. */
const ownSettings: Array<[string, string, boolean]> = [
  ["allowAvatarCopying", "Avatar cloning", false],
  ["isBoopingEnabled", "Booping", false],
  ["hasSharedConnectionsOptOut", "Show mutual friends", true],
  ["hasDiscordFriendsOptOut", "Show Discord connections", true],
  ["hideContentFilterSettings", "Show content filter settings", true],
];

/** Plain names for the sites people link to most. */
const sites: Array<[RegExp, string]> = [
  [/(^|\.)twitch\.tv$/, "Twitch"],
  [/(^|\.)instagram\.com$/, "Instagram"],
  [/(^|\.)(twitter|x)\.com$/, "X"],
  [/(^|\.)(youtube\.com|youtu\.be)$/, "YouTube"],
  [/(^|\.)tiktok\.com$/, "TikTok"],
  [/(^|\.)(discord\.gg|discord\.com)$/, "Discord"],
  [/(^|\.)github\.com$/, "GitHub"],
  [/(^|\.)(ko-fi\.com)$/, "Ko-fi"],
  [/(^|\.)patreon\.com$/, "Patreon"],
  [/(^|\.)booth\.pm$/, "Booth"],
  [/(^|\.)gumroad\.com$/, "Gumroad"],
  [/(^|\.)jinxxy\.com$/, "Jinxxy"],
  [/(^|\.)payhip\.com$/, "Payhip"],
  [/(^|\.)steamcommunity\.com$/, "Steam"],
  [/(^|\.)spotify\.com$/, "Spotify"],
  [/(^|\.)soundcloud\.com$/, "SoundCloud"],
  [/(^|\.)bsky\.app$/, "Bluesky"],
  [/(^|\.)reddit\.com$/, "Reddit"],
  [/(^|\.)vrchat\.com$/, "VRChat"],
];

function siteName(link: string): string {
  try {
    const host = new URL(link).hostname.replace(/^www\./, "");
    return sites.find(([pattern]) => pattern.test(host))?.[1] ?? host;
  } catch {
    return link;
  }
}

function fileId(url: string): string {
  return /file_[0-9a-f-]{36}/.exec(url)?.[0] ?? "";
}

function Thumb({ url }: { url: string }) {
  const shown = usePic(url);
  return <span className="thumb" style={url ? { backgroundImage: `url("${shown}")` } : undefined} />;
}

function PicButton({ url, name }: { url: string; name: string }) {
  return url ? (
    <button className="pic-button" onClick={() => openPicture(url, name, true)} title="Show full size" aria-label={`Show ${name} full size`}>
      <Thumb url={url} />
    </button>
  ) : (
    <Thumb url="" />
  );
}

function Empty({ children }: { children: ReactNode }) {
  return <p className="profile-fine">{children}</p>;
}

export function ProfilePanel({
  userId,
  meId,
  friends,
  worlds,
  inWorld,
  alertRule,
  setAlertRule,
  onBack,
  onClose: closeWindow,
  onDone,
}: {
  /** My notification rule for this person, when they have one of their own. */
  alertRule?: "always" | "never";
  setAlertRule: (rule: "always" | "never" | null, name: string) => void;
  onBack?: () => void;
  userId: string;
  meId: string;
  friends: Map<string, Friend>;
  worlds: Map<string, World>;
  inWorld: boolean;
  onClose: () => void;
  onDone: (message: string) => void;
}) {
  const [profile, setProfile] = useState<Profile | null>(null);
  // A long bio is cut short until asked for, so it does not push everything else away.
  const [bioOpen, setBioOpen] = useState(false);
  // Whether I blocked or muted this person. Asked from the list the app already holds.
  const [blockState, setBlockState] = useState<BlockState | null>(null);
  useEffect(() => {
    let alive = true;
    setBlockState(null);
    if (userId === meId) return;
    void backend
      .moderations(false)
      .then((all) => {
        if (!alive) return;
        const mine = all.filter((m) => m.userId === userId);
        setBlockState({ block: mine.some((m) => m.kind === "block"), mute: mine.some((m) => m.kind === "mute") });
      })
      .catch(() => alive && setBlockState({ block: false, mute: false }));
    // The same person may be unblocked or unmuted from the list behind this profile.
    const changed = (e: Event) => {
      const what = (e as CustomEvent<{ userId: string; kind: "block" | "mute"; on: boolean }>).detail;
      if (what?.userId === userId) setBlockState((old) => (old ? { ...old, [what.kind]: what.on } : old));
    };
    window.addEventListener("moderation:changed", changed);
    return () => {
      alive = false;
      window.removeEventListener("moderation:changed", changed);
    };
  }, [userId, meId]);
  const [problem, setProblem] = useState("");
  const [note, setNote] = useState("");
  const [saved, setSaved] = useState("");
  const [busy, setBusy] = useState(false);
  const [tab, setTab] = useState<Tab>("info");
  // Closing puts every window away, so no tab is remembered past that. Back keeps them.
  const onClose = () => {
    openTabs.clear();
    closeWindow();
  };
  /** Each tab's data, fetched the first time the tab is opened. `null` means it failed. */
  const [parts, setParts] = useState<Record<string, unknown>>({});
  const friend = friends.get(userId);
  const isMe = userId === meId;

  useEffect(() => {
    let alive = true;
    setProfile(null);
    setProblem("");
    setParts({});
    // Coming back from something opened out of a tab lands on that tab again.
    const before = openTabs.get(userId);
    if (before && before !== "info") open(before);
    else setTab("info");
    void backend
      .profile(userId)
      .then((p) => {
        if (!alive) return;
        setProfile(p);
        setNote(p.note);
        setSaved(p.note);
        // Small extras for the Info tab: the group they represent and the avatar they wear.
        const raw = p.raw as Json;
        // VRChat names the worn avatar itself when it may; the avatar database is only the fallback.
        const known = str(raw.currentAvatarName) !== "";
        const lookup = isMe ? str(raw.currentAvatar) : fileId(str(raw.currentAvatarImageUrl) || str(raw.currentAvatarThumbnailImageUrl));
        if (!raw.representedGroup) void load("represented");
        if (!known && lookup) void load("wearing", lookup);
        else if (!known) setParts((old) => ({ ...old, wearing: null }));
      })
      .catch((err) => alive && setProblem(errorText(err)));
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [userId]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  async function load(part: string, extra = "") {
    try {
      const data = await backend.profilePart(userId, part, extra, savedAvatarProvider());
      setParts((old) => ({ ...old, [part]: data }));
    } catch {
      setParts((old) => ({ ...old, [part]: null }));
    }
  }

  function open(next: Tab) {
    setTab(next);
    openTabs.set(userId, next);
    const needs: Record<string, string[]> = {
      mutuals: ["mutualFriends"],
      groups: ["groups", "mutualGroups"],
      worlds: ["worlds"],
      favoriteWorlds: ["favoriteWorlds"],
      avatars: ["avatars"],
    };
    for (const part of needs[next] ?? []) {
      if (!(part in parts) && !(part === "mutualGroups" && isMe)) void load(part);
    }
  }

  async function saveNote() {
    try {
      await backend.setNote(userId, note);
      setSaved(note);
      onDone(note.trim() ? "Note saved on this PC." : "Note removed.");
    } catch (err) {
      onDone(`The note was not saved: ${errorText(err)}`);
    }
  }

  async function act(job: () => Promise<void>, done: string) {
    setBusy(true);
    try {
      await job();
      onDone(done);
    } catch (err) {
      onDone(`That did not work: ${errorText(err)}`);
    } finally {
      setBusy(false);
    }
  }

  const copy = (text: string, done: string) =>
    void navigator.clipboard.writeText(text).then(
      () => onDone(done),
      () => onDone("Could not copy."),
    );
  const web = (url: string) => void backend.openLink(url).catch((err) => onDone(errorText(err)));

  const person = profile?.person;
  const raw = (profile?.raw ?? {}) as Json;
  const name = person?.displayName ?? friend?.displayName ?? "Profile";
  const longBio = (person?.bio ?? "").length > 420 || (person?.bio ?? "").split(NL_CHAR).length > 9;
  const inRoom = friend?.state === "online" && friend.place.kind === "world";
  const standing = isMe
    ? "This is you"
    : friend
      ? friend.state === "active"
        ? deviceLine(friend.platform)
        : statusWords(friend)
      : person
        ? person.isFriend
          ? "Friend"
          : "Not on your friend list"
        : "";
  const statusText = friend?.statusDescription || person?.statusDescription || "";
  const platform = platformWords[person?.lastPlatform ?? ""] ?? "";
  const tags = Array.isArray(raw.tags) ? (raw.tags as unknown[]).map(str) : [];
  const spoken = [
    ...tags.filter((t) => t.startsWith("language_")).map((t) => t.slice(9)),
    ...(Array.isArray(raw.languages) ? (raw.languages as unknown[]).map((l) => str(l).replace(/^language_/, "")) : []),
  ];
  const speaks: Array<[string, string]> = [...new Set(spoken)]
    .filter(Boolean)
    .map((code) => [code, languages[code] ? `${languages[code]} (${code})` : code.toUpperCase()]);
  // Names VRChat remembers, plus changes this app saw itself.
  const pastNames: Array<[string, string]> = [];
  for (const p of list(raw.pastDisplayNames)) pastNames.push([str(p.displayName), str(p.updated_at)]);
  for (const item of profile?.feed ?? []) if (item.kind === "name" && item.previous) pastNames.push([item.previous, item.at]);
  for (const [n, at] of profile?.pastNames ?? []) pastNames.push([n, at]);
  const seenNames = new Set<string>([person?.displayName ?? ""]);
  const formerly = pastNames.filter(([n]) => n && !seenNames.has(n) && !!seenNames.add(n));
  const badges = list(raw.badges).filter((b) => b.hidden !== true);
  const banner = str(raw.bannerUrl) || str(raw.bannerCustomUrl) || str(raw.profilePicOverride) || str(raw.currentAvatarImageUrl);
  const icon = str(raw.iconUrl) || str(raw.userIcon) || person?.image || friend?.image || "";
  const adult = str(raw.ageVerificationStatus) === "18+";
  // What the person picked in VRChat to dress their profile: a frame around the picture, an
  // effect over the banner, and a banner colour.
  const dressed = live().profileCosmetics;
  const frame = usePic(dressed ? cosmeticPicture(raw.iconFrame) : "");
  const effect = usePic(dressed ? cosmeticPicture(raw.profileEffect) : "");
  const tintOn = live().profileBackdrop;
  const tintColour = tintOn ? colourOf(raw.bannerColor) : "";
  const tintPicture = usePic(tintOn && !tintColour ? banner : "");
  const tinted = !!(tintColour || tintPicture);
  const vrcPlus = raw.hasVrcPlus === true || tags.includes("system_supporter");
  const representedRaw = raw.representedGroup ?? parts.represented;
  const representedBase = representedRaw && typeof representedRaw === "object" ? (representedRaw as Json) : null;
  const representedId = representedBase ? str(representedBase.groupId) || str(representedBase.id) : "";
  // The profile only names the group; its picture and member count come from the group itself.
  useEffect(() => {
    if (!representedId) return;
    let alive = true;
    void backend
      .thing("group", representedId)
      .then((g) => alive && g && typeof g === "object" && setParts((old) => ({ ...old, representedFull: g })))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [representedId]);
  const full = parts.representedFull && typeof parts.representedFull === "object" ? (parts.representedFull as Json) : null;
  const represented: Json | null = representedBase
    ? {
        ...representedBase,
        iconUrl: str(representedBase.iconUrl) || str(full?.iconUrl),
        bannerUrl: str(representedBase.bannerUrl) || str(full?.bannerUrl),
        memberCount: num(full?.memberCount) || num(representedBase.memberCount),
        groupId: representedId,
      }
    : null;

  // The avatar database answers with one avatar or a list of them.
  const wearingRaw = parts.wearing;
  const looked = (Array.isArray(wearingRaw) ? list(wearingRaw)[0] : wearingRaw && typeof wearingRaw === "object" ? (wearingRaw as Json) : null) ?? null;
  const wearing: Json | null = str(raw.currentAvatarName)
    ? { id: str(raw.currentAvatar), name: str(raw.currentAvatarName), authorName: str(raw.currentAvatarAuthorName) }
    : looked;
  const wearingId = wearing ? str(wearing.id) || str(wearing.Id) : "";
  const wearingPic = str(raw.currentAvatarThumbnailImageUrl) || str(raw.currentAvatarImageUrl);

  function waiting(part: string, empty: string, render: (rows: Json[]) => ReactNode): ReactNode {
    if (!(part in parts)) return <Empty>Asking VRChat</Empty>;
    if (parts[part] === null) return <Empty>VRChat did not share this. It may be hidden or turned off on their side.</Empty>;
    const rows = list(parts[part]);
    return rows.length === 0 ? <Empty>{empty}</Empty> : render(rows);
  }

  const avatarCards = (rows: Json[]) => (
    <ul className="tiles">
      {rows.map((a) => {
        const id = str(a.id) || str(a.Id);
        const perf = (a.performance ?? {}) as Json;
        const pc = str(perf.pc_rating) || str(perf.standalonewindows);
        const quest = str(perf.android_rating) || str(perf.android);
        return (
          <li key={id} className="tile">
            <button className="tile-open" onClick={() => openThing("avatar", id)} title="Open this avatar">
              <Thumb url={str(a.thumbnailImageUrl) || str(a.imageUrl)} />
              <span className="tile-name">{tidy(str(a.name)) || "Unnamed avatar"}</span>
            </button>
            <span className="tile-note">
              {[str(a.releaseStatus), pc && `PC ${pc}`, quest && `Quest ${quest}`].filter(Boolean).join(", ")}
            </span>
            <div className="wayback">
              <button className="go" disabled={busy} onClick={() => void act(() => backend.wearAvatar(id), `You are now wearing ${str(a.name)}.`)}>
                Wear
              </button>
              <button onClick={() => copy(id, "Avatar ID copied.")}>Copy ID</button>
            </div>
          </li>
        );
      })}
    </ul>
  );

  const worldCards = (rows: Json[]) => (
    <ul className="tiles">
      {rows.map((w) => (
        <li key={str(w.id)} className="tile">
          <button className="tile-open" onClick={() => openThing("world", str(w.id))} title="Open this world">
            <Thumb url={str(w.thumbnailImageUrl) || str(w.imageUrl)} />
            <span className="tile-name">{tidy(str(w.name))}</span>
          </button>
          <span className="tile-note">
            by {str(w.authorName) || "unknown"}, {num(w.visits).toLocaleString()} visits
            {num(w.occupants) > 0 ? `, ${num(w.occupants)} there now` : ""}
          </span>
          <div className="wayback">
            <button onClick={() => copy(`https://vrchat.com/home/world/${str(w.id)}`, "World link copied.")}>Copy link</button>
          </div>
        </li>
      ))}
    </ul>
  );

  const groupRows = (rows: Json[]) => (
    <ul className="rows">
      {rows.map((g) => {
        const id = str(g.groupId) || str(g.id);
        return (
          <li key={id}>
            <button className="row-link" onClick={() => openThing("group", id)} title="Open this group">
              <Thumb url={str(g.iconUrl) || str(g.bannerUrl)} />
              <span>
                <span className="tile-name">{str(g.name)}</span>
                {num(g.memberCount) > 0 && <span className="tile-note">{num(g.memberCount).toLocaleString()} members</span>}
              </span>
            </button>
          </li>
        );
      })}
    </ul>
  );

  return (
    <div className="veil" onClick={onClose}>
      <div
        className={`profile person-window${tinted ? " tinted" : ""}`}
        style={tinted ? ({ "--tint": tintColour || "transparent", "--tint-pic": tintPicture ? `url("${tintPicture}")` : "none", "--tint-strength": String(live().profileBackdropStrength) } as CSSProperties) : undefined}
        role="dialog"
        aria-modal="true"
        aria-label={`${name}, profile`}
        onClick={(e) => e.stopPropagation()}
      >
        <aside className="profile-side">
          <div className="banner-pic" style={banner ? { backgroundImage: `url("${banner}")` } : undefined}>
            {banner && <button className="pic-open" onClick={() => openPicture(banner, `${name} banner`, true)} title="Show the banner full size" aria-label="Show the banner full size" />}
            <button className="icon-open" disabled={!icon} onClick={() => openPicture(icon, `${name} picture`, true)} title="Show the profile picture full size" aria-label="Show the profile picture full size">
              <Avatar name={name} image={icon} tone={friend ? statusTone(friend) : undefined} size={104} />
              {frame && <span className="icon-frame" style={{ backgroundImage: `url("${frame}")` }} aria-hidden="true" />}
            </button>
            {effect && <span className="profile-effect" style={{ backgroundImage: `url("${effect}")` }} aria-hidden="true" />}
          </div>
          <div className="name-wrap" tabIndex={formerly.length ? 0 : undefined}>
            <h2 className="hide-me">
              {name}
              {formerly.length > 0 && <span className="past-mark" aria-hidden="true">▾</span>}
            </h2>
            {formerly.length > 0 && (
              <div className="past-pop" role="note">
                <b>Earlier names</b>
                <ul>
                  {formerly.map(([n, at]) => (
                    <li key={n}>
                      {n}
                      {at && <span> until {day(at)}</span>}
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </div>
          {statusText && <p className="profile-status">{statusText}</p>}
          <ul className="badges">
            {standing && <li className={friend ? `tone-${statusTone(friend)}` : person?.isFriend ? "badge-friend" : ""}>{friend && <i className="tone-dot" />}{standing}</li>}
            {blockState?.block && <li className="chip-blocked">Blocked by you</li>}
            {blockState?.mute && <li className="chip-blocked">Muted by you</li>}
            {person && <li className={`chip-rank rank-${person.rank.replace(/ /g, "")}`}>{person.rank}</li>}
            {adult && <li className="chip-adult">18+ verified</li>}
            {vrcPlus && <li className="chip-plus">VRChat+</li>}
            {person?.pronouns && <li className="chip-pronouns">{person.pronouns}</li>}
            {platform && <li className="chip-platform">Last on {platform}</li>}
            {speaks.map(([code, label]) => (
              <li key={code} className="chip-lang">
                <i className={`flag flag-${code}`} aria-hidden="true" />
                {label}
              </li>
            ))}
          </ul>
          {badges.length > 0 && (
            <ul className="badge-pics" aria-label="VRChat badges">
              {badges.map((b) => (
                <li key={str(b.badgeId)} title={`${str(b.badgeName)}${str(b.badgeDescription) ? `: ${str(b.badgeDescription)}` : ""}`}>
                  {str(b.badgeImageUrl) ? (
                    <img src={str(b.badgeImageUrl)} alt={str(b.badgeName)} loading="lazy" referrerPolicy="no-referrer" />
                  ) : (
                    <span className="badge-word">{str(b.badgeName)}</span>
                  )}
                </li>
              ))}
            </ul>
          )}

          <section>
            <h3>Avatar</h3>
            {wearing && str(wearing.name) ? (
              <div className="side-card">
                <PicButton url={str(wearing.thumbnailImageUrl) || str(wearing.imageUrl) || wearingPic} name={str(wearing.name)} />
                <span>
                  <span className="tile-name">
                    {tidy(str(wearing.name))}
                    {wearing.isPrivate === true && <span className="mark" title="A private avatar: only its owner can use it"> private</span>}
                  </span>
                  {wearing.isPrivate === true ? (
                    <span className="tile-note">{str(wearing.authorId) === userId ? "Their own upload" : "Uploaded by someone else"}</span>
                  ) : (
                    <span className="tile-note">by {str(wearing.authorName) || "unknown"}</span>
                  )}
                  {wearingId ? (
                    <span className="wayback">
                      <button onClick={() => openThing("avatar", wearingId)}>Open</button>
                      {!isMe && (
                        <button disabled={busy} onClick={() => void act(() => backend.wearAvatar(wearingId), `You are now wearing ${str(wearing.name)}.`)}>
                          Wear
                        </button>
                      )}
                    </span>
                  ) : (
                    <span className="tile-note">Private, so it cannot be opened or worn.</span>
                  )}
                </span>
              </div>
            ) : (
              <div className="side-card">
                <PicButton url={wearingPic} name={`${name} avatar`} />
                <span className="tile-note">
                  {"wearing" in parts || !profile
                    ? profile
                      ? wearingPic
                        ? "A private avatar. VRChat shows only its picture."
                        : "A private avatar. VRChat shares its picture only while they are online."
                      : "Loading"
                    : "Looking it up"}
                </span>
              </div>
            )}
          </section>

          {represented && str(represented.name) && (
            <section>
              <h3>Represents</h3>
              <button className="row-link side-card" onClick={() => openThing("group", str(represented.groupId) || str(represented.id))} title="Open this group">
                <Thumb url={str(represented.iconUrl) || str(represented.bannerUrl)} />
                <span>
                  <span className="tile-name">{str(represented.name)}</span>
                  {num(represented.memberCount) > 0 && <span className="tile-note">{num(represented.memberCount).toLocaleString()} members</span>}
                </span>
              </button>
            </section>
          )}
        </aside>

        <div className="profile-main">
          <div className="profile-top">
            <div className="tabs" role="tablist" aria-label="Profile">
              {tabNames
                .filter(([id]) => !(isMe && id === "mutuals"))
                .map(([id, label]) => (
                  <button key={id} role="tab" aria-selected={tab === id} className={tab === id ? "on" : ""} onClick={() => open(id)}>
                    {label}
                  </button>
                ))}
            </div>
            <span className="window-nav">
              {onBack && <button onClick={onBack}>Back</button>}
              <button onClick={onClose}>Close</button>
            </span>
          </div>

          <div className="profile-body">
            {problem && (
              <p className="banner" role="alert">
                VRChat did not send this profile: {problem}
              </p>
            )}
            {!profile && !problem && <Empty>Loading the profile from VRChat</Empty>}

            {tab === "info" && profile && person && (
              <>
                <div className="info-grid">
                  <div className="info-main">
                    <section>
                      <h3>{isMe ? "Where you are" : "Where they are"}</h3>
                      {friend && inRoom ? (
                        <>
                          <p className="profile-where">
                            <b>{tidy(worlds.get(friend.place.worldId ?? "")?.name ?? "A world")}</b>
                            <span className={`access access-${friend.place.access}`}>{accessName(friend.place)}</span>
                            {friend.place.instance && <span>#{friend.place.instance}</span>}
                          </p>
                          <WayBack location={friend.location} onDone={onDone} />
                        </>
                      ) : (
                        <Empty>
                          {friend?.state === "online"
                            ? friend.place.kind === "traveling"
                              ? "On the way somewhere"
                              : "In a private world"
                            : friend?.state === "active"
                              ? deviceLine(friend.platform)
                              : isMe
                                ? "See the Here screen for where you are."
                                : "Offline, or not on your friend list"}
                        </Empty>
                      )}
                      {!isMe && blockState?.block && (
                        <div className="wayback blocked-line">
                          <span className="tile-note">You blocked {name}. You and they do not see or hear each other in VRChat.</span>
                          <BlockMute userId={userId} name={name} state={blockState} setState={setBlockState} only="unblock" onDone={onDone} />
                        </div>
                      )}
                      {!friend && !isMe && !blockState?.block && <AddFriend userId={userId} name={name} onDone={onDone} />}
                      {friend && !isMe && (
                        <div className="wayback">
                          <button
                            disabled={busy || friend.state !== "online"}
                            title={friend.state === "online" ? "Asks them to invite you to where they are" : "They are not in VRChat right now"}
                            onClick={() => void act(() => backend.requestInvite(userId), `Asked ${name} for an invite. Their answer arrives in your Inbox.`)}
                          >
                            Ask for an invite
                          </button>
                          <button
                            disabled={busy || !inWorld}
                            title={inWorld ? "Invites them to the world you are in" : "You are not in a world right now"}
                            onClick={() => void act(() => backend.inviteHere(userId), `Invited ${name} to where you are.`)}
                          >
                            Invite to my world
                          </button>
                          <button disabled={busy} title="Sends them a boop" onClick={() => void act(() => backend.boop(userId), `Booped ${name}.`)}>
                            Boop
                          </button>
                        </div>
                      )}
                    </section>

                    <section>
                      <h3>Bio</h3>
                      {person.bio ? (
                        <>
                          <p className={`profile-bio${longBio && !bioOpen ? " bio-clamp" : ""}`}>{person.bio}</p>
                          {longBio && (
                            <button className="clamp-toggle" aria-expanded={bioOpen} onClick={() => setBioOpen(!bioOpen)}>
                              {bioOpen ? "Show less" : "Show all of it"}
                            </button>
                          )}
                        </>
                      ) : (
                        <Empty>No bio.</Empty>
                      )}
                      {person.bioLinks.length > 0 && (
                        <ul className="link-chips" aria-label="Links">
                          {person.bioLinks.map((link) => (
                            <li key={link}>
                              <button onClick={() => web(link)} title={`${link}\nOpens in your browser`}>
                                {siteName(link)}
                              </button>
                            </li>
                          ))}
                        </ul>
                      )}
                    </section>

                    {!isMe && (
                      <section>
                        <h3>My note</h3>
                        <textarea
                          value={note}
                          onChange={(e) => setNote(e.target.value)}
                          placeholder="How you met, what they like, anything worth remembering. Only you see this; it stays on this PC."
                          rows={3}
                          maxLength={2000}
                        />
                        <div className="wayback">
                          <button className="go" disabled={note === saved} onClick={() => void saveNote()}>
                            Save note
                          </button>
                        </div>
                        {person.vrchatNote && <p className="profile-fine">Note kept on VRChat's side: {person.vrchatNote}</p>}
                      </section>
                    )}
                  </div>

                  <aside className="info-rail">
                    {!isMe && (
                      <section>
                        <h3>You and <span className="hide-me">{name}</span></h3>
                        <dl className="facts">
                          <div>
                            <dt>Times together</dt>
                            <dd>{profile.together.times}</dd>
                          </div>
                          <div>
                            <dt>Time together</dt>
                            <dd>{profile.together.times ? longTime(profile.together.seconds) : "None recorded"}</dd>
                          </div>
                          <div>
                            <dt>First seen together</dt>
                            <dd>{profile.together.first ? day(profile.together.first) : "Not yet"}</dd>
                          </div>
                          <div>
                            <dt>Last seen together</dt>
                            <dd>{profile.together.last ? day(profile.together.last) : "Not yet"}</dd>
                          </div>
                        </dl>
                        <p className="profile-fine">From VRChat's log files on this PC, since this app could read them.</p>
                      </section>
                    )}

                    {friend && !isMe && <FriendFavourite userId={userId} name={name} onDone={onDone} />}

                    {friend && !isMe && (
                      <section>
                        <h3>Notifications</h3>
                        <div className="segments" role="radiogroup" aria-label={`Notifications about ${name}`}>
                          {(
                            [
                              [null, "Like their group"],
                              ["always", "Always"],
                              ["never", "Never"],
                            ] as const
                          ).map(([rule, label]) => (
                            <button key={label} role="radio" aria-checked={(alertRule ?? null) === rule} className={(alertRule ?? null) === rule ? "on" : ""} onClick={() => setAlertRule(rule, name)}>
                              {label}
                            </button>
                          ))}
                        </div>
                        <p className="profile-fine">When they come online or leave a private world.</p>
                      </section>
                    )}

                    <section>
                      <h3>Account</h3>
                      <dl className="fact-rows">
                        {str(raw.last_activity) && (
                          <div>
                            <dt>Last activity</dt>
                            <dd>{ago(str(raw.last_activity))}</dd>
                          </div>
                        )}
                        {person.joined && (
                          <div>
                            <dt>On VRChat since</dt>
                            <dd>{day(person.joined)}</dd>
                          </div>
                        )}
                        {person.lastLogin && (
                          <div>
                            <dt>Last sign-in</dt>
                            <dd>{day(person.lastLogin)}</dd>
                          </div>
                        )}
                      </dl>
                      <div className="quiet-actions">
                        <button onClick={() => copy(person.id, "User ID copied.")}>Copy user ID</button>
                        <button onClick={() => copy(`https://vrchat.com/home/user/${person.id}`, "Profile link copied.")}>Copy link</button>
                      </div>
                      {!isMe && (
                        <div className="quiet-actions quiet-actions-apart">
                          {friend && <Unfriend userId={userId} name={name} onDone={onDone} />}
                          <BlockMute userId={userId} name={name} state={blockState} setState={setBlockState} onDone={onDone} />
                        </div>
                      )}
                    </section>

                    {isMe && (
                      <section>
                        <h3>My settings on VRChat</h3>
                        <dl className="fact-rows">
                          {ownSettings
                            .filter(([key]) => typeof raw[key] === "boolean")
                            .map(([key, label, inverted]) => (
                              <div key={key}>
                                <dt>{label}</dt>
                                <dd>{(raw[key] as boolean) !== inverted ? "On" : "Off"}</dd>
                              </div>
                            ))}
                        </dl>
                        <p className="profile-fine">Read only. Change these in VRChat.</p>
                      </section>
                    )}
                  </aside>
                </div>
              </>
            )}

            {tab === "mutuals" && (
              <section>
                <h3>Friends you both have</h3>
                {waiting("mutualFriends", "No mutual friends that VRChat shows.", (rows) => (
                  <ul className="rows rows-people">
                    {rows.map((u) => (
                      <li key={str(u.id)}>
                        <button className="row-link" onClick={() => openProfile(str(u.id))}>
                          <Avatar name={str(u.displayName) || "Hidden"} image={str(u.iconUrl) || str(u.userIcon) || str(u.currentAvatarThumbnailImageUrl) || str(u.currentAvatarImageUrl)} size={36} />
                          <span className="tile-name">{str(u.displayName) || "Hidden mutual"}</span>
                        </button>
                      </li>
                    ))}
                  </ul>
                ))}
              </section>
            )}

            {tab === "groups" && (
              <>
                {!isMe && (
                  <section>
                    <h3>Groups you are both in</h3>
                    {waiting("mutualGroups", "None.", groupRows)}
                  </section>
                )}
                <section>
                  <h3>All groups</h3>
                  {waiting("groups", "No groups to show.", groupRows)}
                </section>
              </>
            )}

            {tab === "worlds" && (
              <section>
                <h3>Worlds by <span className="hide-me">{name}</span></h3>
                {waiting("worlds", "No public worlds.", worldCards)}
              </section>
            )}

            {tab === "favoriteWorlds" && (
              <section>
                <h3>Favourite worlds</h3>
                {waiting("favoriteWorlds", "No favourite worlds they share.", worldCards)}
              </section>
            )}

            {tab === "avatars" && (
              <section>
                <h3>{isMe ? "My avatars" : <>Public avatars by <span className="hide-me">{name}</span></>}</h3>
                {!isMe && <p className="profile-fine">From the community avatar database, which only knows public avatars.</p>}
                {waiting("avatars", isMe ? "You have not uploaded any avatars." : "No public avatars found.", avatarCards)}
              </section>
            )}

            {tab === "activity" &&
              profile &&
              (profile.feed.length === 0 ? (
                <Empty>Nothing recorded for <span className="hide-me">{name}</span> yet. This fills in from now on.</Empty>
              ) : (
                <section>
                  <h3>Recent activity</h3>
                  <ol className="profile-feed">
                    {profile.feed.map((item) => (
                      <li key={item.id}>
                        <time dateTime={item.at}>
                          {dayLabel(item.at)}, {clock(item.at)}
                        </time>
                        <span>
                          {feedWords[item.kind] ?? item.kind}
                          {item.kind === "location" && item.worldId && `: ${tidy(worlds.get(item.worldId)?.name ?? "a world")}`}
                          {item.kind === "name" && `: ${item.previous} to ${item.detail}`}
                          {(item.kind === "status" || item.kind === "statusText") && item.detail && `: ${item.detail}`}
                        </span>
                      </li>
                    ))}
                  </ol>
                </section>
              ))}

            {tab === "json" && profile && (
              <section>
                <h3>Raw data from VRChat</h3>
                <div className="wayback">
                  <button onClick={() => copy(JSON.stringify(raw, null, 2), "Copied.")}>Copy all</button>
                </div>
                <pre className="raw">{JSON.stringify(raw, null, 2)}</pre>
              </section>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
