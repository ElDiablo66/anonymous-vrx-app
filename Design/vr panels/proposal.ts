/**
 * Mockups of the proposed 0.1.10 headset panels. Not part of the app: once a design is picked,
 * it moves into src/vrpanels.ts. Same fonts and colour tokens as the window.
 *
 * Rules every panel keeps: 1024 px wide; three text sizes (64, 40, 28); spacing on an 8 px grid;
 * text on a flat ground, art only in the side bar and the corner sun; one accent per alert type.
 */

export type AlertType = "friend" | "invite" | "room" | "day" | "battery" | "tracking" | "steamvr";

export interface Device {
  name: string;
  battery: number | null;
  charging?: boolean;
  lost?: boolean;
}

export interface Row {
  name: string;
  what: string;
  at: string;
}

const W = 1024;
const SIZE = { big: 64, mid: 40, small: 28 };
const LEFT = 64;
const PAD = 48;

function tokens() {
  const css = getComputedStyle(document.documentElement);
  const pick = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
  return {
    ground: "#160b30",
    groundEdge: "#1d0f3c",
    text: pick("--text", "#f6edff"),
    soft: pick("--text-soft", "#d3c2ea"),
    line: "rgb(255 255 255 / 0.14)",
    display: pick("--display", '"Barlow Condensed", "Segoe UI", sans-serif'),
    body: pick("--body", '"Instrument Sans Variable", "Segoe UI", sans-serif'),
    accent: {
      friend: pick("--teal", "#2de2e6"),
      invite: pick("--lamp", "#ff4fa3"),
      room: pick("--online", "#5fe3a1"),
      day: pick("--web", "#c39bff"),
      battery: pick("--ask", "#ffa53c"),
      tracking: pick("--busy", "#ff5470"),
      steamvr: pick("--join", "#4fb8ff"),
    } as Record<AlertType, string>,
    sunset: pick("--sunset", "#ff9a3c"),
    lamp: pick("--lamp", "#ff4fa3"),
  };
}

async function fonts() {
  const t = tokens();
  try {
    await Promise.all([`italic 700 ${SIZE.big}px ${t.display}`, `600 ${SIZE.mid}px ${t.body}`, `500 ${SIZE.mid}px ${t.body}`, `600 ${SIZE.small}px ${t.body}`, `500 ${SIZE.small}px ${t.body}`].map((f) => document.fonts.load(f)));
  } catch {
    // A font that will not load falls back to the next one in its list.
  }
}

function frame(height: number, accent: string) {
  const canvas = document.createElement("canvas");
  canvas.width = W;
  canvas.height = height;
  const c = canvas.getContext("2d")!;
  const t = tokens();
  c.beginPath();
  c.roundRect(0, 0, W, height, 40);
  c.clip();
  // A flat ground, so text never sits on art.
  c.fillStyle = t.ground;
  c.fillRect(0, 0, W, height);
  // The side bar carries the alert's colour.
  c.fillStyle = accent;
  c.fillRect(0, 0, 16, height);
  c.strokeStyle = "rgb(255 255 255 / 0.16)";
  c.lineWidth = 4;
  c.beginPath();
  c.roundRect(2, 2, W - 4, height - 4, 38);
  c.stroke();
  return { canvas, c, t };
}

/** A small striped sun in the top-right corner, drawn here (no borrowed art). */
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
  // Stripes through the lower half, the way a synthwave sun is drawn.
  c.fillStyle = "#160b30";
  for (let i = 0; i < 4; i++) c.fillRect(x - r, y + 4 + i * 16, r * 2, 4 + i * 2);
  c.restore();
}

function fit(c: CanvasRenderingContext2D, text: string, width: number): string {
  if (c.measureText(text).width <= width) return text;
  let cut = text;
  while (cut.length > 1 && c.measureText(`${cut}…`).width > width) cut = cut.slice(0, -1);
  return `${cut.trimEnd()}…`;
}

function wrap(c: CanvasRenderingContext2D, text: string, width: number, most: number): string[] {
  const words = text.split(/\s+/).filter(Boolean);
  const out: string[] = [];
  let line = "";
  for (const w of words) {
    const next = line ? `${line} ${w}` : w;
    if (c.measureText(next).width <= width || !line) line = next;
    else {
      out.push(line);
      line = w;
    }
  }
  if (line) out.push(line);
  if (out.length > most) {
    const kept = out.slice(0, most);
    kept[most - 1] = fit(c, `${kept[most - 1]} ${out.slice(most).join(" ")}`, width);
    return kept;
  }
  return out;
}

/** An alert in front of you: a coloured label, a title and up to two lines. */
export async function drawAlert(type: AlertType, label: string, title: string, body: string): Promise<string> {
  await fonts();
  const probe = document.createElement("canvas").getContext("2d")!;
  const t = tokens();
  const textWidth = W - LEFT - 160;
  probe.font = `500 ${SIZE.mid}px ${t.body}`;
  const lines = body ? wrap(probe, body, textWidth, 2) : [];
  // Baselines on the 8 px grid: label 80, title 160, body 224 and 272.
  const height = lines.length === 0 ? 208 : lines.length === 1 ? 272 : 320;
  const { canvas, c } = frame(height, t.accent[type]);
  sun(c, W - 96, 96, 48, t.accent[type], t.sunset);
  c.fillStyle = t.accent[type];
  c.font = `600 ${SIZE.small}px ${t.body}`;
  c.fillText(fit(c, label, textWidth), LEFT, 80);
  c.fillStyle = t.text;
  c.font = `italic 700 ${SIZE.big}px ${t.display}`;
  c.fillText(fit(c, title, textWidth), LEFT, 160);
  c.fillStyle = t.soft;
  c.font = `500 ${SIZE.mid}px ${t.body}`;
  lines.forEach((l, i) => c.fillText(l, LEFT, 224 + i * 48));
  return canvas.toDataURL("image/png");
}

/**
 * One chip per device: a device that lost tracking first, then low batteries, then the rest.
 * The name and the number are set apart so "Tracker 1" and "80%" never read as one figure.
 * What does not fit is counted at the end.
 */
function batteries(c: CanvasRenderingContext2D, t: ReturnType<typeof tokens>, devices: Device[], y: number) {
  c.fillStyle = t.line;
  c.fillRect(PAD, y - 56, W - PAD * 2, 2);
  const rank = (d: Device) => (d.lost ? 0 : d.battery !== null && d.battery <= 0.2 ? 1 : 2);
  const sorted = [...devices].sort((a, b) => rank(a) - rank(b));
  let x = PAD;
  for (let i = 0; i < sorted.length; i++) {
    const d = sorted[i];
    const low = d.battery !== null && d.battery <= 0.2;
    const value = d.lost ? "lost" : d.battery === null ? "" : `${Math.round(d.battery * 100)}%${d.charging ? " charging" : ""}`;
    c.font = `500 ${SIZE.small}px ${t.body}`;
    const nameWidth = c.measureText(d.name).width;
    c.font = `600 ${SIZE.small}px ${t.body}`;
    const valueWidth = value ? c.measureText(value).width + 16 : 0;
    const width = nameWidth + valueWidth + 32;
    const rest = sorted.length - i;
    c.font = `600 ${SIZE.small}px ${t.body}`;
    const moreWidth = rest > 1 ? c.measureText(`+${rest - 1} more`).width + 16 : 0;
    if (x + width + moreWidth > W - PAD) {
      c.fillStyle = t.soft;
      c.fillText(`+${rest} more`, x, y);
      break;
    }
    const tone = d.lost ? t.accent.tracking : low ? t.accent.battery : "";
    c.fillStyle = tone || "rgb(255 255 255 / 0.08)";
    c.globalAlpha = tone ? 0.22 : 1;
    c.beginPath();
    c.roundRect(x, y - 36, width, 48, 24);
    c.fill();
    c.globalAlpha = 1;
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

/** The wrist: the clock, friends in VRChat, the newest changes and, at the foot, the batteries. */
export async function drawWrist(clock: string, online: number, rows: Row[], devices: Device[]): Promise<string> {
  await fonts();
  const t = tokens();
  const shown = rows.slice(0, 6);
  const listHeight = Math.max(1, shown.length) * 96;
  const footer = devices.length ? 112 : 0;
  const height = 160 + listHeight + footer + 32;
  const { canvas, c } = frame(height, t.accent.friend);
  c.fillStyle = t.text;
  c.font = `italic 700 ${SIZE.big}px ${t.display}`;
  c.fillText(clock, LEFT, 96);
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
  if (devices.length) batteries(c, t, devices, 160 + listHeight + 80);
  return canvas.toDataURL("image/png");
}

/** A tab in SteamVR's own dashboard: more of the feed, and every device's battery. */
export async function drawDashboard(clock: string, online: number, rows: Row[], devices: Device[]): Promise<string> {
  await fonts();
  const t = tokens();
  const shown = rows.slice(0, 7);
  const height = 768;
  const { canvas, c } = frame(height, t.accent.friend);
  sun(c, W - 96, 96, 48, t.lamp, t.sunset);
  c.fillStyle = t.text;
  c.font = `italic 700 ${SIZE.big}px ${t.display}`;
  c.fillText("Anonymous VRX", LEFT, 96);
  c.fillStyle = t.accent.friend;
  c.font = `600 ${SIZE.small}px ${t.body}`;
  c.fillText(`${clock}, ${online} ${online === 1 ? "friend" : "friends"} in VRChat`, LEFT, 144);
  c.fillStyle = t.line;
  c.fillRect(LEFT, 176, W - LEFT - PAD, 2);
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
  batteries(c, t, devices, height - 48);
  return canvas.toDataURL("image/png");
}
