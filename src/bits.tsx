import { useState } from "react";
import { SafeImg } from "./Pic";
import type { Friend, Place } from "./backend";
import { live } from "./settings";

const accessNames: Record<string, string> = {
  public: "Public",
  friendsPlus: "Friends+",
  friends: "Friends",
  invitePlus: "Invite+",
  invite: "Invite",
  group: "Group",
  groupPlus: "Group+",
  groupPublic: "Group public",
};

export function accessName(place: Place): string {
  return place.access ? (accessNames[place.access] ?? place.access) : "";
}

const regionNames: Record<string, string> = {
  us: "US West",
  use: "US East",
  eu: "Europe",
  jp: "Japan",
};

export function regionName(place: Place): string {
  return place.region ? (regionNames[place.region] ?? place.region.toUpperCase()) : "";
}

/** The dot colour follows the status the friend picked in VRChat. */
export function statusTone(f: Friend): string {
  if (f.state === "offline") return "offline";
  if (f.state === "active") return "web";
  switch (f.status) {
    case "join me":
      return "join";
    case "ask me":
      return "ask";
    case "busy":
      return "busy";
    default:
      return "online";
  }
}

export function statusWords(f: Friend): string {
  if (f.state === "offline") return "Offline";
  if (f.state === "active") return "On the website or phone";
  switch (f.status) {
    case "join me":
      return "Join me";
    case "ask me":
      return "Ask me";
    case "busy":
      return "Do not disturb";
    default:
      return "Online";
  }
}

function initials(name: string): string {
  const parts = name.trim().split(/[\s._-]+/).filter(Boolean);
  const letters = parts.length > 1 ? parts[0][0] + parts[1][0] : name.trim().slice(0, 2);
  return letters.toUpperCase();
}

function hue(text: string): number {
  let h = 0;
  for (const c of text) h = (h * 31 + c.charCodeAt(0)) % 360;
  return h;
}

export function Avatar({ name, image, tone, size }: { name: string; image: string; tone?: string; size?: number }) {
  const [broken, setBroken] = useState(false);
  const showImage = image && !broken;
  const box = size ? { width: size, height: size } : { width: "var(--row)", height: "var(--row)" };
  return (
    <span className="avatar" style={box}>
      {showImage ? (
        <SafeImg src={image} loading="lazy" onGiveUp={() => setBroken(true)} />
      ) : (
        <span className="avatar-letters" style={{ background: `oklch(0.52 0.14 ${hue(name)})`, fontSize: size ? size * 0.36 : "calc(var(--row) * 0.36)" }}>
          {initials(name)}
        </span>
      )}
      {tone && <span className={`dot dot-${tone}`} />}
    </span>
  );
}

/** How long ago, in short words: "4 min", "2 h 10 min". Empty when the time is unknown. */
export function sinceWords(iso: string): string {
  if (!iso) return "";
  const minutes = Math.max(0, Math.round((Date.now() - Date.parse(iso)) / 60_000));
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest ? `${hours} h ${rest} min` : `${hours} h`;
}

/** How long ago in whole days, for things that happened a while back: "today", "3 days ago", "5 months ago". */
export function daysAgo(iso: string): string {
  if (!iso) return "";
  const days = Math.floor((Date.now() - Date.parse(iso)) / 86_400_000);
  if (days < 1) return "today";
  if (days === 1) return "yesterday";
  if (days < 60) return `${days} days ago`;
  if (days < 730) return `${Math.round(days / 30)} months ago`;
  return `${Math.round(days / 365)} years ago`;
}

/**
 * World names often use full-width brackets and spaces, which the heading font
 * draws with big gaps. This swaps them for the ordinary characters.
 */
export function tidy(name: string): string {
  return name.normalize("NFKC").replace(/\s+/g, " ").trim();
}

/** Where a friend who is not in the game is signed in, in plain words. Empty when unknown. */
export function deviceWords(platform: string): string {
  switch (platform) {
    case "web":
      return "Website";
    case "android":
      return "Android";
    case "ios":
      return "iPhone or iPad";
    default:
      return "";
  }
}

/** The same as a short sentence, for places with room for one. */
export function deviceLine(platform: string): string {
  switch (platform) {
    case "web":
      return "On the website";
    case "android":
      return "On an Android phone";
    case "ios":
      return "On an iPhone or iPad";
    default:
      return "On the website or phone";
  }
}

/**
 * The time of day: "4:03pm" on a 12-hour clock, "16:03" on a 24-hour one. Which of the two
 * follows Windows unless Settings says otherwise.
 */
export function clock(iso: string): string {
  const pick = live().timeFormat;
  const text = new Date(iso).toLocaleTimeString([], { hour: pick === "24" ? "2-digit" : "numeric", minute: "2-digit", ...(pick === "pc" ? {} : { hour12: pick === "12" }) });
  return text.replace(/\s*([AaPp])\.?\s?[Mm]\.?$/, (_, half: string) => `${half.toLowerCase()}m`);
}

/** "2026-11-25", in the PC's own time. */
function isoDay(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/** A date: "25 November 2026" or "25 Nov 2026", or "2026-11-25" when Settings asks for that. Empty when unknown. */
export function dateText(iso: string | number, month: "long" | "short" = "long"): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return live().dateFormat === "iso" ? isoDay(d) : d.toLocaleDateString([], { day: "numeric", month, year: "numeric" });
}

/**
 * What colours a name in a list: the trust rank's colour, a colour of the person's own
 * (always the same one for the same person), or nothing.
 */
export function nameTone(userId: string, rank?: string): { className: string; style?: { color: string }; title?: string } {
  const how = live().nameColours;
  if (how === "person" && userId) return { className: "rank-name", style: { color: `oklch(0.8 0.13 ${hue(userId)})` }, title: rank || undefined };
  if (how === "rank" && rank) return { className: `rank-name rank-${rank.replace(/ /g, "")}`, title: rank };
  return { className: "", title: rank || undefined };
}

/** The part of a location that names the instance itself: "12345" or a word the maker picked. */
export function instanceName(location: string): string {
  const cut = location.indexOf(":");
  return cut < 0 ? "" : location.slice(cut + 1).split("~")[0];
}

/** Only people VRChat has verified as 18 or older get into this instance. */
export function ageGated(location: string): boolean {
  return /~ageGate(~|$)/.test(location);
}

export function dayLabel(iso: string): string {
  const d = new Date(iso);
  const today = new Date();
  const yesterday = new Date(today.getTime() - 86_400_000);
  if (d.toDateString() === today.toDateString()) return "Today";
  if (d.toDateString() === yesterday.toDateString()) return "Yesterday";
  if (live().dateFormat === "iso") return `${isoDay(d)}, ${d.toLocaleDateString([], { weekday: "long" })}`;
  return d.toLocaleDateString([], { weekday: "long", day: "numeric", month: "long" });
}
