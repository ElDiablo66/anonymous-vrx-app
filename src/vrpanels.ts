import type { DeviceState, FeedItem } from "./backend";
import { clock } from "./bits";

/**
 * The app's panels in the headset are pictures drawn here, with the same fonts and colours as
 * the window, and handed to SteamVR as PNG files.
 *
 * Rules every panel keeps, so it reads well in a headset: 1024 px wide; three text sizes (64, 40
 * and 28, nothing smaller); every distance on an 8 px grid; text on a flat ground, with art only
 * in the side bar and a small striped sun in the corner; one colour for each kind of alert, and
 * every colour checked to read at 4.5:1 or better on the ground it sits on.
 */

export type AlertKind = "friend" | "invite" | "room" | "day" | "battery" | "tracking" | "steamvr";

/** The word over a notice when nothing more fitting was given. */
const LABELS: Record<AlertKind, string> = {
  friend: "Friends",
  invite: "From VRChat",
  room: "Room free",
  day: "Today",
  battery: "Battery low",
  tracking: "Tracking",
  steamvr: "SteamVR",
};

export interface Row {
  name: string;
  what: string;
  at: string;
}

const W = 1024;
const SIZE = { big: 64, mid: 40, small: 28 };
const LEFT = 64;
const PAD = 48;
const GROUND = "#160b30";

/** A colour as red, green and blue, whatever way CSS wrote it. */
function rgbOf(color: string): [number, number, number] | null {
  const c = document.createElement("canvas").getContext("2d");
  if (!c) return null;
  c.fillStyle = "#000000";
  c.fillStyle = color;
  const v = String(c.fillStyle);
  if (/^#[0-9a-f]{6}$/i.test(v)) return [parseInt(v.slice(1, 3), 16), parseInt(v.slice(3, 5), 16), parseInt(v.slice(5, 7), 16)];
  const m = v.match(/rgba?\(([^)]+)\)/);
  if (!m) return null;
  const [r, g, b] = m[1].split(",").map((x) => Number(x.trim()));
  return [r, g, b];
}

function luminance([r, g, b]: [number, number, number]): number {
  const one = (v: number) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * one(r) + 0.7152 * one(g) + 0.0722 * one(b);
}

/** How well one colour reads on another: 1 (not at all) to 21 (black on white). */
export function contrast(a: string, b: string): number {
  const x = rgbOf(a);
  const y = rgbOf(b);
  if (!x || !y) return 1;
  const [hi, lo] = [luminance(x), luminance(y)].sort((p, q) => q - p);
  return (hi + 0.05) / (lo + 0.05);
}

/** A see-through white laid over the ground, as one solid colour. */
function lifted(ground: string, alpha: number): string {
  const g = rgbOf(ground) ?? [22, 11, 48];
  return `rgb(${g.map((v) => Math.round(v + (255 - v) * alpha)).join(" ")})`;
}

/**
 * The colour itself when it reads at 4.5:1 on the ground; otherwise mixed with white, a step at
 * a time, until it does. A dark accent picked in Settings still gives readable words.
 */
export function legible(color: string, ground: string): string {
  let c = rgbOf(color) ?? [255, 255, 255];
  const g = rgbOf(ground) ?? [22, 11, 48];
  const ratio = (x: [number, number, number]) => {
    const [hi, lo] = [luminance(x), luminance(g)].sort((p, q) => q - p);
    return (hi + 0.05) / (lo + 0.05);
  };
  for (let step = 0; step < 12 && ratio(c) < 4.5; step++) c = c.map((v) => Math.round(v + (255 - v) * 0.2)) as [number, number, number];
  return `rgb(${c.join(" ")})`;
}

function tokens() {
  const css = getComputedStyle(document.documentElement);
  const pick = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
  const chip = lifted(GROUND, 0.06);
  const read = (name: string, fallback: string) => legible(pick(name, fallback), chip);
  return {
    ground: GROUND,
    chip,
    text: legible(pick("--text", "#f6edff"), chip),
    soft: legible(pick("--text-soft", "#b9a6dc"), chip),
    line: "rgb(255 255 255 / 0.14)",
    display: pick("--display", '"Barlow Condensed", "Segoe UI", sans-serif'),
    body: pick("--body", '"Instrument Sans Variable", "Segoe UI", sans-serif'),
    accent: {
      friend: read("--teal", "#2de2e6"),
      invite: read("--lamp", "#ff4fa3"),
      room: read("--online", "#5fe3a1"),
      day: read("--web", "#c39bff"),
      battery: read("--ask", "#ffa53c"),
      tracking: read("--busy", "#ff5470"),
      steamvr: read("--join", "#4fb8ff"),
    } as Record<AlertKind, string>,
    sunset: pick("--sunset", "#ff9a3c"),
    lamp: pick("--lamp", "#ff4fa3"),
  };
}

type Tokens = ReturnType<typeof tokens>;

/** The fonts have to be loaded before a canvas can write with them. */
async function fonts(t: Tokens) {
  try {
    await Promise.all(
      [`italic 700 ${SIZE.big}px ${t.display}`, `600 ${SIZE.mid}px ${t.body}`, `500 ${SIZE.mid}px ${t.body}`, `600 ${SIZE.small}px ${t.body}`, `500 ${SIZE.small}px ${t.body}`].map((f) =>
        document.fonts.load(f),
      ),
    );
  } catch {
    // A font that will not load falls back to the next one in its list.
  }
}

function frame(t: Tokens, height: number, accent: string) {
  const canvas = document.createElement("canvas");
  canvas.width = W;
  canvas.height = height;
  const c = canvas.getContext("2d")!;
  c.beginPath();
  c.roundRect(0, 0, W, height, 40);
  c.clip();
  // A flat ground, so text never sits on art.
  c.fillStyle = t.ground;
  c.fillRect(0, 0, W, height);
  // The side bar carries the panel's colour.
  c.fillStyle = accent;
  c.fillRect(0, 0, 16, height);
  c.strokeStyle = "rgb(255 255 255 / 0.16)";
  c.lineWidth = 4;
  c.beginPath();
  c.roundRect(2, 2, W - 4, height - 4, 38);
  c.stroke();
  c.textBaseline = "alphabetic";
  return { canvas, c };
}

/** A small striped sun, drawn here: a glow at the top, the panel's colour below, cut by stripes. */
function sun(c: CanvasRenderingContext2D, x: number, y: number, r: number, colour: string, glow: string) {
  c.save();
  c.beginPath();
  c.arc(x, y, r, 0, Math.PI * 2);
  c.clip();
  const fill = c.createLinearGradient(0, y - r, 0, y + r);
  fill.addColorStop(0, glow);
  fill.addColorStop(1, colour);
  c.fillStyle = fill;
  c.fillRect(x - r, y - r, r * 2, r * 2);
  // Stripes through the lower half, wider towards the bottom.
  c.fillStyle = GROUND;
  for (let i = 0; i < 4; i++) c.fillRect(x - r, y + 4 + i * 16, r * 2, 4 + i * 2);
  c.restore();
}

/** Cuts text to what fits in `width`, with an ellipsis when something had to go. */
function fit(c: CanvasRenderingContext2D, text: string, width: number): string {
  if (c.measureText(text).width <= width) return text;
  let cut = text;
  while (cut.length > 1 && c.measureText(`${cut}…`).width > width) cut = cut.slice(0, -1);
  return `${cut.trimEnd()}…`;
}

/** Breaks text into at most `most` lines that fit in `width`; the last one ends early when more is left. */
function wrap(c: CanvasRenderingContext2D, text: string, width: number, most: number): string[] {
  const out: string[] = [];
  let line = "";
  for (const word of text.split(/\s+/).filter(Boolean)) {
    const next = line ? `${line} ${word}` : word;
    if (c.measureText(next).width <= width || !line) line = next;
    else {
      out.push(line);
      line = word;
    }
  }
  if (line) out.push(line);
  if (out.length > most) {
    const kept = out.slice(0, most);
    kept[most - 1] = fit(c, `${kept[most - 1]} ${out.slice(most).join(" ")}`, width);
    return kept.map((l) => fit(c, l, width));
  }
  return out.map((l) => fit(c, l, width));
}

function kindOf(kind: string): AlertKind {
  return kind in LABELS ? (kind as AlertKind) : "friend";
}

/** A notice for in front of the eyes: a coloured label, a title and up to two lines. */
export async function drawNotice(kind: string, label: string, title: string, body: string): Promise<string> {
  const t = tokens();
  await fonts(t);
  const k = kindOf(kind);
  const probe = document.createElement("canvas").getContext("2d")!;
  const textWidth = W - LEFT - 160;
  probe.font = `500 ${SIZE.mid}px ${t.body}`;
  const lines = body.trim() ? wrap(probe, body.trim(), textWidth, 2) : [];
  // Baselines on the 8 px grid: label 80, title 160, body 224 and 272.
  const height = lines.length === 0 ? 208 : lines.length === 1 ? 272 : 320;
  const { canvas, c } = frame(t, height, t.accent[k]);
  sun(c, W - 96, 96, 48, t.accent[k], t.sunset);
  c.fillStyle = t.accent[k];
  c.font = `600 ${SIZE.small}px ${t.body}`;
  c.fillText(fit(c, label.trim() || LABELS[k], textWidth), LEFT, 80);
  c.fillStyle = t.text;
  c.font = `italic 700 ${SIZE.big}px ${t.display}`;
  c.fillText(fit(c, title, textWidth), LEFT, 160);
  c.fillStyle = t.soft;
  c.font = `500 ${SIZE.mid}px ${t.body}`;
  lines.forEach((l, i) => c.fillText(l, LEFT, 224 + i * 48));
  return canvas.toDataURL("image/png");
}

/** What a feed entry says, in a few words. */
export function feedWords(item: FeedItem): string {
  switch (item.kind) {
    case "online":
      return item.detail ? `came online in ${item.detail}` : "came online";
    case "offline":
      return "went offline";
    case "location":
      return item.detail ? `went to ${item.detail}` : "went to a private world";
    case "status":
      return `is now ${item.detail}`;
    case "statusText":
      return item.detail ? `wrote "${item.detail}"` : "cleared their status";
    case "name":
      return `is now called ${item.detail}`;
    case "avatar":
      return item.detail ? `changed to ${item.detail}` : "changed avatar";
    case "friendAdded":
      return "is now a friend";
    case "friendRemoved":
      return "is no longer a friend";
  }
}

/** The feed as rows of name, what happened and when. */
export function feedRows(feed: FeedItem[], most: number): Row[] {
  return feed.slice(0, most).map((f) => ({ name: f.displayName, what: feedWords(f), at: clock(f.at) }));
}

/** What one device chip says after its name. */
function deviceValue(d: DeviceState): string {
  if (d.lost) return "lost";
  if (d.battery === null) return "";
  return `${d.battery}%${d.charging ? " charging" : ""}`;
}

/** A device that lost tracking first, then low batteries, then the rest, each group in the order given. */
export function devicesInOrder(devices: DeviceState[]): DeviceState[] {
  const rank = (d: DeviceState) => (d.lost ? 0 : d.battery !== null && d.battery <= 20 && !d.charging ? 1 : 2);
  return devices.map((d, i) => ({ d, i })).sort((a, b) => rank(a.d) - rank(b.d) || a.i - b.i).map((x) => x.d);
}

/**
 * One chip per device, on up to `rows` lines 72 px apart. The name and the number are set apart,
 * so "Tracker 1" and "80%" never read as one figure. What does not fit is counted at the end.
 */
function chips(c: CanvasRenderingContext2D, t: Tokens, devices: DeviceState[], y: number, rows = 1) {
  c.fillStyle = t.line;
  c.fillRect(PAD, y - 56, W - PAD * 2, 2);
  const sorted = devicesInOrder(devices);
  let x = PAD;
  let row = 1;
  for (let i = 0; i < sorted.length; i++) {
    const d = sorted[i];
    const low = d.battery !== null && d.battery <= 20 && !d.charging;
    const value = deviceValue(d);
    c.font = `500 ${SIZE.small}px ${t.body}`;
    const nameWidth = c.measureText(d.name).width;
    c.font = `600 ${SIZE.small}px ${t.body}`;
    const valueWidth = value ? c.measureText(value).width + 16 : 0;
    const width = Math.ceil((nameWidth + valueWidth + 32) / 8) * 8;
    const rest = sorted.length - i;
    const last = row === rows;
    const moreWidth = last && rest > 1 ? c.measureText(`+${rest - 1} more`).width + 16 : 0;
    if (x + width + moreWidth > W - PAD) {
      if (!last && x > PAD) {
        row += 1;
        y += 72;
        x = PAD;
        i -= 1;
        continue;
      }
      c.fillStyle = t.soft;
      c.fillText(`+${rest} more`, x, y);
      break;
    }
    const tone = d.lost ? t.accent.tracking : low ? t.accent.battery : "";
    c.fillStyle = t.chip;
    c.beginPath();
    c.roundRect(x, y - 40, width, 56, 24);
    c.fill();
    if (tone) {
      // A problem shows as a coloured edge and coloured words, on the same plain chip.
      c.strokeStyle = tone;
      c.lineWidth = 3;
      c.stroke();
    }
    c.font = `500 ${SIZE.small}px ${t.body}`;
    c.fillStyle = tone || t.soft;
    c.fillText(d.name, x + 16, y);
    if (value) {
      c.font = `600 ${SIZE.small}px ${t.body}`;
      c.fillStyle = tone || t.text;
      c.fillText(value, x + 16 + nameWidth + 16, y);
    }
    x += width + 16;
  }
}

/** The feed for the arm: the clock, friends in VRChat, the six newest changes and the batteries. */
export async function drawWrist(feed: FeedItem[], online: number, devices: DeviceState[]): Promise<string> {
  const t = tokens();
  await fonts(t);
  const shown = feedRows(feed, 6);
  const listHeight = Math.max(1, shown.length) * 96;
  const footer = devices.length ? 112 : 0;
  const height = 160 + listHeight + footer + 32;
  const { canvas, c } = frame(t, height, t.accent.friend);
  c.fillStyle = t.text;
  c.font = `italic 700 ${SIZE.big}px ${t.display}`;
  c.fillText(clock(new Date().toISOString()), LEFT, 96);
  c.fillStyle = t.accent.friend;
  c.font = `600 ${SIZE.small}px ${t.body}`;
  c.textAlign = "right";
  c.fillText(`${online} ${online === 1 ? "friend" : "friends"} in VRChat`, W - PAD, 88);
  c.textAlign = "left";
  c.fillStyle = t.line;
  c.fillRect(LEFT, 128, W - LEFT - PAD, 2);
  if (shown.length === 0) {
    c.fillStyle = t.soft;
    c.font = `500 ${SIZE.mid}px ${t.body}`;
    c.fillText("Nothing has happened yet.", LEFT, 208);
  }
  shown.forEach((r, i) => {
    const top = 160 + i * 96;
    c.font = `500 ${SIZE.small}px ${t.body}`;
    const timeWidth = c.measureText(r.at).width;
    c.fillStyle = t.soft;
    c.textAlign = "right";
    c.fillText(r.at, W - PAD, top + 40);
    c.textAlign = "left";
    c.fillStyle = t.text;
    c.font = `600 ${SIZE.mid}px ${t.body}`;
    c.fillText(fit(c, r.name, W - LEFT - PAD - timeWidth - 32), LEFT, top + 40);
    c.fillStyle = t.soft;
    c.font = `500 ${SIZE.small}px ${t.body}`;
    c.fillText(fit(c, r.what, W - LEFT - PAD), LEFT, top + 80);
  });
  if (devices.length) chips(c, t, devices, 160 + listHeight + 80);
  return canvas.toDataURL("image/png");
}

/** The pages of the dashboard tab: the feed, and the settings for notices and for the arm. */
export type DashPage = "feed" | "notices" | "arm";

/** A place on the dashboard picture that does something when clicked, in its pixels. */
export interface Hit {
  x: number;
  y: number;
  w: number;
  h: number;
  /** `page:<page>`, `set:<setting>:<JSON value>`, `step:<setting>:<amount>` or `test`. */
  act: string;
}

/** The settings the dashboard tab shows and changes, and which page is open. */
export interface DashView {
  page: DashPage;
  vrOwn: boolean;
  vrRoute: "xs" | "own";
  vrNoticeSpot: "above" | "low";
  vrSeconds: number;
  vrDevices: boolean;
  vrWrist: "off" | "left" | "right";
  vrWristSpot: "forearm" | "wrist" | "inside";
  vrWristShift: number;
  vrWristSize: number;
  vrOpacity: number;
  /** The words for "the other programs first", or "" when no other program is switched on. */
  others: string;
}

/** The dashboard picture is always this tall, so the tab keeps its size from page to page. */
export const DASH_HEIGHT = 816;
const CHIP = 56;

/** A button: filled with the colour when it is the choice in force, plain otherwise. */
function button(c: CanvasRenderingContext2D, t: Tokens, x: number, y: number, text: string, on: boolean, act: string, hits: Hit[]): number {
  c.font = `600 ${SIZE.small}px ${t.body}`;
  const w = Math.ceil((c.measureText(text).width + 48) / 8) * 8;
  c.fillStyle = on ? t.accent.friend : t.chip;
  c.beginPath();
  c.roundRect(x, y, w, CHIP, 24);
  c.fill();
  if (!on) {
    c.strokeStyle = t.line;
    c.lineWidth = 2;
    c.stroke();
  }
  c.fillStyle = on ? t.ground : t.text;
  c.fillText(text, x + 24, y + 38);
  hits.push({ x, y, w, h: CHIP, act });
  return w;
}

/** One setting: its name, and under it the choices side by side. */
function choices(c: CanvasRenderingContext2D, t: Tokens, top: number, label: string, list: Array<[string, string, boolean]>, hits: Hit[]) {
  c.font = `500 ${SIZE.small}px ${t.body}`;
  c.fillStyle = t.soft;
  c.fillText(label, LEFT, top + 28);
  let x = LEFT;
  for (const [text, act, on] of list) x += button(c, t, x, top + 40, text, on, act, hits) + 16;
}

/** One setting with a number: its name, then minus, the value and plus. */
function stepper(c: CanvasRenderingContext2D, t: Tokens, top: number, label: string, value: string, key: string, step: number, hits: Hit[]) {
  c.font = `500 ${SIZE.small}px ${t.body}`;
  c.fillStyle = t.soft;
  c.fillText(label, LEFT, top + 28);
  const y = top + 40;
  const less = button(c, t, LEFT, y, "\u2212", false, `step:${key}:${-step}`, hits);
  c.font = `600 ${SIZE.small}px ${t.body}`;
  c.fillStyle = t.text;
  c.textAlign = "center";
  c.fillText(value, LEFT + less + 72, y + 38);
  c.textAlign = "left";
  button(c, t, LEFT + less + 144, y, "+", false, `step:${key}:${step}`, hits);
}

/**
 * The tab in SteamVR's dashboard. The feed page has more of the feed and every device's battery;
 * the other two pages hold the same settings as the window, so they can be changed without
 * taking the headset off. Gives the picture and where each button sits on it.
 */
export async function drawDashboard(feed: FeedItem[], online: number, devices: DeviceState[], view: DashView): Promise<{ png: string; hits: Hit[] }> {
  const t = tokens();
  await fonts(t);
  const hits: Hit[] = [];
  const height = DASH_HEIGHT;
  const { canvas, c } = frame(t, height, t.accent.friend);
  c.fillStyle = t.text;
  c.font = `italic 700 ${SIZE.big}px ${t.display}`;
  c.fillText("Anonymous VRX", LEFT, 96);
  // The pages, as buttons along the top right.
  const pages: Array<[DashPage, string]> = [
    ["feed", "Feed"],
    ["notices", "Notices"],
    ["arm", "Arm"],
  ];
  c.font = `600 ${SIZE.small}px ${t.body}`;
  const widths = pages.map(([, words]) => Math.ceil((c.measureText(words).width + 48) / 8) * 8);
  let x = W - PAD - widths.reduce((sum, w) => sum + w, 0) - 16 * (pages.length - 1);
  pages.forEach(([page, words], i) => {
    button(c, t, x, 48, words, view.page === page, `page:${page}`, hits);
    x += widths[i] + 16;
  });
  c.fillStyle = t.line;
  c.fillRect(LEFT, 128, W - LEFT - PAD, 2);

  if (view.page === "notices") {
    const v = view;
    choices(c, t, 152, "Notices in front of you", [
      ["Off", "set:vrOwn:false", !v.vrOwn],
      ["On", "set:vrOwn:true", v.vrOwn],
    ], hits);
    const any = v.vrOwn || v.vrDevices;
    if (v.others && any) {
      choices(c, t, 256, "Which one shows a notice", [
        [v.others, 'set:vrRoute:"xs"', v.vrRoute === "xs"],
        ["This app's panel first", 'set:vrRoute:"own"', v.vrRoute === "own"],
      ], hits);
    } else {
      c.font = `500 ${SIZE.small}px ${t.body}`;
      c.fillStyle = t.soft;
      c.fillText("Which one shows a notice", LEFT, 256 + 28);
      c.fillStyle = t.text;
      c.fillText(any ? "Only this app's panel is switched on." : "Nothing here is switched on.", LEFT, 256 + 78);
    }
    choices(c, t, 360, "Where the notice floats", [
      ["Over the middle", 'set:vrNoticeSpot:"above"', v.vrNoticeSpot === "above"],
      ["Under the middle", 'set:vrNoticeSpot:"low"', v.vrNoticeSpot === "low"],
    ], hits);
    choices(
      c,
      t,
      464,
      "Stays up for",
      [2, 4, 6, 10].map((n) => [`${n} s`, `set:vrSeconds:${n}`, v.vrSeconds === n] as [string, string, boolean]),
      hits,
    );
    choices(c, t, 568, "Battery, tracking and SteamVR alerts", [
      ["Off", "set:vrDevices:false", !v.vrDevices],
      ["On", "set:vrDevices:true", v.vrDevices],
    ], hits);
    if (any) button(c, t, LEFT, 696, "Send a test notice", false, "test", hits);
    else {
      c.font = `500 ${SIZE.small}px ${t.body}`;
      c.fillStyle = t.soft;
      c.fillText("Switch on notices or alerts to try one.", LEFT, 696 + 38);
    }
  } else if (view.page === "arm") {
    const v = view;
    choices(c, t, 152, "Feed on the arm", [
      ["Off", 'set:vrWrist:"off"', v.vrWrist === "off"],
      ["Left hand", 'set:vrWrist:"left"', v.vrWrist === "left"],
      ["Right hand", 'set:vrWrist:"right"', v.vrWrist === "right"],
    ], hits);
    choices(c, t, 256, "Where on the arm", [
      ["Up the arm", 'set:vrWristSpot:"forearm"', v.vrWristSpot === "forearm"],
      ["Back of the wrist", 'set:vrWristSpot:"wrist"', v.vrWristSpot === "wrist"],
      ["Inside of the wrist", 'set:vrWristSpot:"inside"', v.vrWristSpot === "inside"],
    ], hits);
    stepper(c, t, 360, "Move it along the arm", `${v.vrWristShift > 0 ? "+" : ""}${v.vrWristShift} cm`, "vrWristShift", 1, hits);
    choices(c, t, 464, "Size of the feed", [
      ["Small", "set:vrWristSize:0.14", v.vrWristSize === 0.14],
      ["Medium", "set:vrWristSize:0.18", v.vrWristSize === 0.18],
      ["Large", "set:vrWristSize:0.24", v.vrWristSize === 0.24],
    ], hits);
    stepper(c, t, 568, "How solid the panels are", `${Math.round(v.vrOpacity * 100)}%`, "vrOpacity", 0.05, hits);
  } else {
    // With devices, two lines of chips take the room of two feed rows.
    const shown = feedRows(feed, devices.length ? 6 : 8);
    c.fillStyle = t.accent.friend;
    c.font = `600 ${SIZE.small}px ${t.body}`;
    c.fillText(`${clock(new Date().toISOString())}, ${online} ${online === 1 ? "friend" : "friends"} in VRChat`, LEFT, 176);
    if (shown.length === 0) {
      c.fillStyle = t.soft;
      c.font = `500 ${SIZE.mid}px ${t.body}`;
      c.fillText("Nothing has happened yet.", LEFT, 256);
    }
    shown.forEach((r, i) => {
      const top = 200 + i * 64;
      c.font = `500 ${SIZE.small}px ${t.body}`;
      const timeWidth = c.measureText(r.at).width;
      c.fillStyle = t.soft;
      c.textAlign = "right";
      c.fillText(r.at, W - PAD, top + 40);
      c.textAlign = "left";
      c.font = `600 ${SIZE.small}px ${t.body}`;
      c.fillStyle = t.text;
      const name = fit(c, r.name, 280);
      c.fillText(name, LEFT, top + 40);
      const nameWidth = c.measureText(name).width;
      c.font = `500 ${SIZE.small}px ${t.body}`;
      c.fillStyle = t.soft;
      c.fillText(fit(c, r.what, W - LEFT - PAD - nameWidth - timeWidth - 48), LEFT + nameWidth + 16, top + 40);
    });
    if (devices.length) chips(c, t, devices, height - 48 - 72, 2);
  }
  return { png: canvas.toDataURL("image/png"), hits };
}

/** The small square on the dashboard tab's button: the sun on the ground. */
export async function drawThumb(): Promise<string> {
  const t = tokens();
  const canvas = document.createElement("canvas");
  canvas.width = 256;
  canvas.height = 256;
  const c = canvas.getContext("2d")!;
  c.beginPath();
  c.roundRect(0, 0, 256, 256, 48);
  c.clip();
  c.fillStyle = t.ground;
  c.fillRect(0, 0, 256, 256);
  c.save();
  c.translate(128, 128);
  c.scale(2, 2);
  c.translate(-128, -128);
  sun(c, 128, 128, 48, t.lamp, t.sunset);
  c.restore();
  return canvas.toDataURL("image/png");
}
