import type { FeedItem } from "./backend";
import { clock } from "./bits";

/**
 * The app's panels in the headset are pictures drawn here, with the same fonts and colours
 * as the window, and handed to SteamVR as PNG files.
 */

function look() {
  const css = getComputedStyle(document.documentElement);
  const pick = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
  return {
    accent: pick("--lamp", "#ff4fa3"),
    second: pick("--teal", "#2de2e6"),
    text: pick("--text", "#f6edff"),
    soft: pick("--text-soft", "#d3c2ea"),
    display: pick("--display", '"Barlow Condensed", "Segoe UI", sans-serif'),
    body: pick("--body", '"Instrument Sans Variable", "Segoe UI", sans-serif'),
  };
}

/** The fonts have to be loaded before a canvas can write with them. */
async function fontsReady(fonts: string[]) {
  try {
    await Promise.all(fonts.map((f) => document.fonts.load(f)));
  } catch {
    // A font that will not load falls back to the next one in its list.
  }
}

function panel(width: number, height: number, radius: number) {
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const c = canvas.getContext("2d")!;
  const l = look();
  c.beginPath();
  c.roundRect(0, 0, width, height, radius);
  c.clip();
  const ground = c.createLinearGradient(0, 0, width, height);
  ground.addColorStop(0, "#1d0f3c");
  ground.addColorStop(1, "#120826");
  c.fillStyle = ground;
  c.fillRect(0, 0, width, height);
  // A glow in the corner, the colour of the app's accent.
  const glow = c.createRadialGradient(width, 0, 0, width, 0, width * 0.7);
  glow.addColorStop(0, l.accent);
  glow.addColorStop(1, "rgb(0 0 0 / 0)");
  c.globalAlpha = 0.3;
  c.fillStyle = glow;
  c.fillRect(0, 0, width, height);
  c.globalAlpha = 1;
  c.strokeStyle = "rgb(255 255 255 / 0.16)";
  c.lineWidth = 4;
  c.beginPath();
  c.roundRect(2, 2, width - 4, height - 4, radius - 2);
  c.stroke();
  return { canvas, c, l };
}

/** Cuts text to what fits in `width`, with an ellipsis when something had to go. */
function fit(c: CanvasRenderingContext2D, text: string, width: number): string {
  if (c.measureText(text).width <= width) return text;
  let cut = text;
  while (cut.length > 1 && c.measureText(`${cut}…`).width > width) cut = cut.slice(0, -1);
  return `${cut.trimEnd()}…`;
}

/** Breaks text into at most `most` lines that fit in `width`. */
function lines(c: CanvasRenderingContext2D, text: string, width: number, most: number): string[] {
  const out: string[] = [];
  let line = "";
  for (const word of text.split(/\s+/).filter(Boolean)) {
    const tried = line ? `${line} ${word}` : word;
    if (c.measureText(tried).width <= width || !line) {
      line = tried;
    } else {
      out.push(line);
      line = word;
      if (out.length === most) break;
    }
  }
  if (out.length < most && line) out.push(line);
  else if (line && out.length === most) out[most - 1] = fit(c, `${out[most - 1]} ${line}`, width);
  return out.map((l) => fit(c, l, width));
}

/** A notice for in front of the eyes: a title and up to two lines under it. */
export async function drawNotice(title: string, body: string): Promise<string> {
  const { canvas, c, l } = panel(1024, body ? 300 : 210, 44);
  await fontsReady([`italic 700 76px ${l.display}`, `500 44px ${l.body}`]);
  const bar = c.createLinearGradient(0, 0, 0, canvas.height);
  bar.addColorStop(0, l.accent);
  bar.addColorStop(1, l.second);
  c.fillStyle = bar;
  c.fillRect(0, 0, 18, canvas.height);

  c.textBaseline = "alphabetic";
  c.fillStyle = l.soft;
  c.font = `600 30px ${l.body}`;
  c.fillText("Anonymous VRX", 62, 62);
  c.fillStyle = l.text;
  c.font = `italic 700 76px ${l.display}`;
  c.fillText(fit(c, title, 900), 60, 138);
  if (body) {
    c.fillStyle = l.soft;
    c.font = `500 44px ${l.body}`;
    lines(c, body, 900, 2).forEach((text, n) => c.fillText(text, 62, 200 + n * 54));
  }
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

/** The feed for the wrist: the time, how many friends are on, and the newest entries. */
export async function drawWrist(feed: FeedItem[], online: number): Promise<string> {
  const rows = 6;
  const { canvas, c, l } = panel(720, 150 + rows * 92, 40);
  await fontsReady([`italic 700 64px ${l.display}`, `600 34px ${l.body}`, `500 30px ${l.body}`]);
  c.textBaseline = "alphabetic";
  c.fillStyle = l.text;
  c.font = `italic 700 64px ${l.display}`;
  c.fillText(clock(new Date().toISOString()), 40, 86);
  c.textAlign = "right";
  c.fillStyle = l.second;
  c.font = `600 32px ${l.body}`;
  c.fillText(`${online} ${online === 1 ? "friend" : "friends"} in VRChat`, canvas.width - 40, 80);
  c.textAlign = "left";
  c.fillStyle = "rgb(255 255 255 / 0.14)";
  c.fillRect(40, 116, canvas.width - 80, 3);

  const shown = feed.slice(0, rows);
  if (shown.length === 0) {
    c.fillStyle = l.soft;
    c.font = `500 32px ${l.body}`;
    c.fillText("Nothing has happened yet.", 40, 190);
  }
  shown.forEach((item, n) => {
    const y = 176 + n * 92;
    c.fillStyle = l.soft;
    c.font = `500 26px ${l.body}`;
    const time = clock(item.at);
    c.textAlign = "right";
    c.fillText(time, canvas.width - 40, y);
    const room = canvas.width - 100 - c.measureText(time).width;
    c.textAlign = "left";
    c.fillStyle = l.text;
    c.font = `600 34px ${l.body}`;
    c.fillText(fit(c, item.displayName, room), 40, y);
    c.fillStyle = l.soft;
    c.font = `500 30px ${l.body}`;
    c.fillText(fit(c, feedWords(item), canvas.width - 80), 40, y + 40);
  });
  return canvas.toDataURL("image/png");
}
