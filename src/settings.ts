import { useEffect, useRef, useState } from "react";
import { backend } from "./backend";

export const sizes = [1, 1.15, 1.3, 1.5, 1.75, 2];

export const DEFAULT_AVATAR_PROVIDER = "https://api.avtrdb.com/v3/avatar/search/vrcx";

/** Community avatar databases that answered when checked on 2026-11-24. */
export const EXTRA_AVATAR_PROVIDERS = [
  { name: "VRCDB", url: "https://vrcx.vrcdb.com/avatars/Avatar/VRCX" },
  { name: "Puppy's Avatar World", url: "https://paw-api.amelia.fun/vrcx_search" },
];

export interface Settings {
  /** A fixed size, or null to let the size follow the window. */
  size: number | null;
  rows: "roomy" | "tight";
  /**
   * `sunset` violet-to-orange blend, `night` plain dark purple, `mono` black and white,
   * `red` red and black, `custom` two colours of your own, `image` a picture of your own.
   */
  background: "sunset" | "night" | "mono" | "red" | "custom" | "image";
  /** The two colours of the `custom` background. */
  bgA: string;
  bgB: string;
  /** How much the picture is darkened so text stays readable, 0 to 0.85. */
  imageDim: number;
  /** Own colours. `null` means "whatever the background choice comes with". */
  accent: string | null;
  accent2: string | null;
  textColor: string | null;
  softTextColor: string | null;
  alertFavouriteOnline: boolean;
  alertFavouriteLeavesPrivate: boolean;
  alertInvites: boolean;
  alertFriendRequests: boolean;
  alertBoops: boolean;
  /** Group invites, announcements and anything else VRChat sends. */
  alertEverythingElse: boolean;
  /** What the window's close button does. */
  onClose: "ask" | "tray" | "quit";
  /** How "Join in VRChat" starts the game when it is not running yet. */
  launchMode: "auto" | "desktop" | "vr";
  /** Friends left out of the activity feed: user id to the name they had when hidden. */
  feedHidden: Record<string, string>;
  /** Show a run of world changes by one friend as a single line. */
  feedFold: boolean;
  /** The public avatar database that avatar search asks. */
  avatarProvider: string;
  /** Other avatar databases asked as well, by address. */
  avatarExtras: string[];
  /** `rows`: cards read left to right, row by row. `tight`: each card under the shortest column. */
  cardPacking: "rows" | "tight";
  /** Blur every name, picture and line of text about people, for screenshots and streams. */
  hidePeople: boolean;
  /** The Activity panel on the right is shown. */
  activityShown: boolean;
  /** Favourite groups whose friends raise no notification, by group key. */
  alertGroupsOff: string[];
  /** Put my own world, avatar and status changes into the feed. */
  feedMine: boolean;
  /** No Windows notifications between two times of day. */
  quietHours: boolean;
  /** "23:00" style, local time. */
  quietFrom: string;
  quietTo: string;
  /** No Windows notifications while VRChat is running. */
  quietInGame: boolean;
  /** Friends with a notification rule of their own: user id to their name and the rule. */
  alertPeople: Record<string, { name: string; rule: "always" | "never" }>;
  /** Bring the window up when VRChat starts. */
  openWithGame: boolean;
  /** What to do when VRChat closes without a goodbye while I am in an instance. */
  rejoin: "off" | "ask" | "auto";
  /** Make a backup by itself once a day. */
  autoBackup: boolean;
  /** How many automatic backups are kept; older ones are removed. */
  autoBackupKeep: number;
  /** Show where I am on my Discord profile. Off until I turn it on. */
  discordOn: boolean;
  /** The id of my own Discord application, which the activity is shown under. */
  discordAppId: string;
  /** Name the world and show its picture. */
  discordWorld: boolean;
  /** Say what kind of instance it is. */
  discordAccess: boolean;
  /** Say how many people are in it. */
  discordPeople: boolean;
  /** Add a Join button on instances others can enter from a link. */
  discordJoin: boolean;
  /** Old switch, replaced by `discordPrivateMode`. Kept so saved settings still load. */
  discordPrivate: boolean;
  /**
   * In Invite and Invite+ instances: `hush` shows only that I am in a private world, `none`
   * shows nothing at all, `show` treats them like any other instance.
   */
  discordPrivateMode: "hush" | "none" | "show";
  /** `plain` says it straight, `night` gives the lines some neon. */
  discordStyle: "plain" | "night";
  /** Add a button to the world's page on VRChat's website. */
  discordWorldPage: boolean;
}

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function load(): Settings {
  // Choices made before the settings screen existed are carried over once.
  const oldSize = Number(read("size-picked"));
  const oldAlerts = read("alerts") !== "off";
  const defaults: Settings = {
    size: sizes.includes(oldSize) ? oldSize : null,
    rows: read("density") === "compact" ? "tight" : "roomy",
    background: "sunset",
    bgA: "#1b2a6b",
    bgB: "#0f6b5c",
    imageDim: 0.45,
    accent: null,
    accent2: null,
    textColor: null,
    softTextColor: null,
    alertFavouriteOnline: oldAlerts,
    alertFavouriteLeavesPrivate: oldAlerts,
    alertInvites: true,
    alertFriendRequests: true,
    alertBoops: true,
    alertEverythingElse: true,
    onClose: "ask",
    launchMode: "auto",
    feedHidden: {},
    feedFold: true,
    hidePeople: false,
    activityShown: true,
    alertGroupsOff: [],
    feedMine: false,
    quietHours: false,
    quietFrom: "23:00",
    quietTo: "08:00",
    quietInGame: false,
    alertPeople: {},
    openWithGame: false,
    rejoin: "ask",
    autoBackup: true,
    autoBackupKeep: 7,
    discordOn: false,
    discordAppId: "",
    discordWorld: true,
    discordAccess: true,
    discordPeople: false,
    discordJoin: false,
    discordPrivate: false,
    discordPrivateMode: "hush",
    discordStyle: "night",
    discordWorldPage: true,
    avatarProvider: DEFAULT_AVATAR_PROVIDER,
    avatarExtras: EXTRA_AVATAR_PROVIDERS.map((p) => p.url),
    cardPacking: "rows",
  };
  try {
    const stored = JSON.parse(read("settings") ?? "{}") as Partial<Settings>;
    const merged = { ...defaults, ...stored };
    // Settings saved before the three-way choice existed keep what they meant.
    if (stored.discordPrivateMode === undefined && stored.discordPrivate) merged.discordPrivateMode = "show";
    if (merged.size !== null && !sizes.includes(merged.size)) merged.size = null;
    return merged;
  } catch {
    return defaults;
  }
}


/** Every avatar database a search asks: the main one first, then the extra ones switched on. */
export function savedAvatarProviders(): string[] {
  const s = load();
  const known = new Set(EXTRA_AVATAR_PROVIDERS.map((p) => p.url));
  const extras = (Array.isArray(s.avatarExtras) ? s.avatarExtras : []).filter((u) => known.has(u));
  return [...new Set([s.avatarProvider || DEFAULT_AVATAR_PROVIDER, ...extras])];
}

/** The avatar search address as saved. */
export function savedAvatarProvider(): string {
  return load().avatarProvider || DEFAULT_AVATAR_PROVIDER;
}

/** What each background choice brings when no own colour is picked. */
export const themeColours: Record<Settings["background"], { accent: string; accent2: string; text: string; soft: string; flat: boolean }> = {
  sunset: { accent: "#ff4fa3", accent2: "#2de2e6", text: "#f6edff", soft: "#d3c2ea", flat: false },
  night: { accent: "#ff4fa3", accent2: "#2de2e6", text: "#f6edff", soft: "#b9a6dc", flat: false },
  mono: { accent: "#ffffff", accent2: "#cfcfcf", text: "#f2f2f2", soft: "#a6a6a6", flat: true },
  red: { accent: "#ff2d2d", accent2: "#ff9a9a", text: "#f7ecec", soft: "#c9a3a3", flat: true },
  custom: { accent: "#ff4fa3", accent2: "#2de2e6", text: "#f6edff", soft: "#d3c2ea", flat: false },
  image: { accent: "#ff4fa3", accent2: "#2de2e6", text: "#ffffff", soft: "#dcdcdc", flat: false },
};

/** Black or white, whichever reads better on the given colour. */
export function readableOn(hex: string): string {
  const n = parseInt(hex.replace("#", ""), 16);
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  return r * 0.299 + g * 0.587 + b * 0.114 > 150 ? "#141414" : "#ffffff";
}

const IMAGE_KEY = "background-picture";

function applyTheme(s: Settings, picture: string) {
  const root = document.documentElement;
  const base = themeColours[s.background];
  const accent = s.accent ?? base.accent;
  const accent2 = s.accent2 ?? base.accent2;
  root.dataset.bg = s.background;
  const set = (name: string, value: string | null) => (value === null ? root.style.removeProperty(name) : root.style.setProperty(name, value));
  set("--lamp", accent);
  set("--teal", accent2);
  set("--lamp-pale", `color-mix(in srgb, ${accent2} 72%, #ffffff)`);
  set("--text", s.textColor ?? base.text);
  set("--text-soft", s.softTextColor ?? base.soft);
  // Main buttons keep the pink-to-orange blend unless the accent is someone's own or the theme is a flat one.
  const flat = base.flat || s.accent !== null;
  set("--go", flat ? accent : null);
  set("--go-text", flat ? readableOn(accent) : null);
  set("--bg-a", s.background === "custom" ? s.bgA : null);
  set("--bg-b", s.background === "custom" ? s.bgB : null);
  set("--bg-image", s.background === "image" && picture ? `url("${picture}")` : null);
  set("--bg-dim", s.background === "image" ? String(s.imageDim) : null);
}

/**
 * Shrinks a chosen picture so it fits in the window's storage, and returns it as text.
 * Rejects when the file is not a picture the window can read.
 */
export function shrinkPicture(file: File, longest = 1920): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("That file could not be read."));
    reader.onload = () => {
      const img = new Image();
      img.onerror = () => reject(new Error("That file is not a picture this app can show."));
      img.onload = () => {
        const scale = Math.min(1, longest / Math.max(img.width, img.height));
        const canvas = document.createElement("canvas");
        canvas.width = Math.round(img.width * scale);
        canvas.height = Math.round(img.height * scale);
        canvas.getContext("2d")!.drawImage(img, 0, 0, canvas.width, canvas.height);
        resolve(canvas.toDataURL("image/jpeg", 0.82));
      };
      img.src = String(reader.result);
    };
    reader.readAsDataURL(file);
  });
}

/** The launch choice as saved, for parts of the window that do not hold the settings. */
export function savedLaunchMode(): Settings["launchMode"] {
  return load().launchMode;
}

/** A big, wide window gets larger text so nothing looks lost in it. */
export function sizeForWindow(): number {
  const width = window.innerWidth;
  if (width >= 4400) return 2;
  if (width >= 3400) return 1.5;
  if (width >= 2500) return 1.3;
  if (width >= 2000) return 1.15;
  return 1;
}

export function useSettings() {
  const [settings, setSettings] = useState<Settings>(load);
  const [fitted, setFitted] = useState(sizeForWindow);
  const [picture, setPictureState] = useState(() => read(IMAGE_KEY) ?? "");

  // The file the app keeps is the copy to trust: the window's own store reaches the disk with
  // a delay, so a change made just before closing or updating could be missing from it.
  const fileRead = useRef(false);
  useEffect(() => {
    let alive = true;
    void backend
      .loadSettings()
      .then((text) => {
        if (!alive) return;
        if (text) {
          const kept = JSON.parse(text) as Partial<Settings>;
          setSettings((old) => ({ ...old, ...kept }));
        }
      })
      .catch(() => {})
      .finally(() => {
        fileRead.current = true;
        // A fresh object, so the settings are written to the file once from here on.
        if (alive) setSettings((old) => ({ ...old }));
      });
    return () => {
      alive = false;
    };
  }, []);

  // Saved a moment after the last change, so dragging a colour does not write a file
  // dozens of times a second. Closing the window saves at once.
  useEffect(() => {
    const save = () => {
      const text = JSON.stringify(settings);
      try {
        localStorage.setItem("settings", text);
      } catch {
        // Without storage the choices last until the window closes.
      }
      // Not before the file was read, or the defaults would overwrite it.
      if (fileRead.current) void backend.saveSettings(text).catch(() => {});
    };
    const later = window.setTimeout(save, 350);
    window.addEventListener("pagehide", save);
    return () => {
      window.clearTimeout(later);
      window.removeEventListener("pagehide", save);
    };
  }, [settings]);

  useEffect(() => {
    const refit = () => setFitted(sizeForWindow());
    window.addEventListener("resize", refit);
    return () => window.removeEventListener("resize", refit);
  }, []);

  const size = settings.size ?? fitted;
  useEffect(() => {
    document.documentElement.style.zoom = String(size);
  }, [size]);
  useEffect(() => {
    document.documentElement.dataset.density = settings.rows === "tight" ? "compact" : "comfortable";
  }, [settings.rows]);
  useEffect(() => applyTheme(settings, picture), [settings, picture]);
  useEffect(() => {
    document.documentElement.dataset.cards = settings.cardPacking;
  }, [settings.cardPacking]);
  useEffect(() => {
    document.documentElement.dataset.hide = settings.hidePeople ? "on" : "off";
  }, [settings.hidePeople]);

  /** Keeps a background picture, or removes it with an empty string. False when it did not fit. */
  const setPicture = (data: string): boolean => {
    try {
      if (data) localStorage.setItem(IMAGE_KEY, data);
      else localStorage.removeItem(IMAGE_KEY);
    } catch {
      return false;
    }
    setPictureState(data);
    return true;
  };

  const change = (patch: Partial<Settings>) => setSettings((old) => ({ ...old, ...patch }));
  return { settings, change, size, fitted, picture, setPicture };
}
