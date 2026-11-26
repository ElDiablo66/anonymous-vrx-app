import { discordPresence } from "./presence";
import type { HereNow } from "./backend";
import { PersonMenu } from "./PersonMenu";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  type CalendarEvent,
  backend,
  errorText,
  type Favorites,
  type FeedItem,
  type Friend,
  type Instance,
  type Me,
  type Notice,
  type ServiceStatus,
  type World,
} from "./backend";
import { Avatar, sinceWords, statusTone, tidy } from "./bits";
import { Login } from "./Login";
import { Around } from "./Around";
import { Everyone } from "./Everyone";
import { Activity } from "./Activity";
import { Here } from "./Here";
import { History } from "./History";
import { Inbox, noticeLine } from "./Inbox";
import { SettingsPage } from "./SettingsPage";
import { ProfilePanel, openProfile } from "./Profile";
import { Search } from "./Search";
import { DetailPanel, type ThingKind } from "./Detail";
import { Favourites } from "./Favourites";
import { PictureViewer } from "./Viewer";
import { MyStatus } from "./MyStatus";
import { InstancePanel } from "./Instance";
import { Uploads } from "./Uploads";
import { Events, monthOf } from "./Events";
import { useSettings } from "./settings";
import logo from "./assets/logo.svg";

type View = "around" | "here" | "history" | "everyone" | "favourites" | "events" | "uploads" | "search" | "inbox" | "settings";

/** What the page is called while a tab other than the first is open. */
const tabTitles: Partial<Record<View, Record<string, string>>> = {
  history: { Worlds: "World history", Photos: "Photos", Avatars: "Avatar history", "Friend activity": "Friend activity" },
  everyone: { "Friend list": "Everyone", "Friend log": "Friend log", "Blocked and muted": "Blocked and muted" },
  events: { "My groups": "Events of your groups", "All of VRChat": "Events across VRChat", Following: "Events you follow", Featured: "Featured events" },
  favourites: { Avatars: "Favourite avatars", Worlds: "Favourite worlds", Friends: "Favourite friends", "Group names": "Favourite group names" },
  uploads: { Avatars: "My avatars", Worlds: "My worlds" },
  inbox: { Waiting: "Inbox", Done: "Inbox, done" },
  search: { People: "Search people", Worlds: "Search worlds", Avatars: "Search avatars" },
};

const titles: Record<View, string> = {
  around: "Around right now",
  here: "Where you are",
  history: "World history",
  everyone: "Everyone",
  favourites: "Favourites",
  events: "Events",
  uploads: "My uploads",
  search: "Search",
  inbox: "Inbox",
  settings: "Settings",
};

const linkWords: Record<string, string> = {
  live: "Live",
  connecting: "Connecting",
  reconnecting: "Reconnecting",
  off: "Not connected",
};

const icons: Record<View, string> = {
  around: "M4 20V10l8-6 8 6v10h-5v-6H9v6z",
  here: "M12 2a7 7 0 0 0-7 7c0 5 7 13 7 13s7-8 7-13a7 7 0 0 0-7-7zm0 9.5A2.5 2.5 0 1 1 12 6.5a2.5 2.5 0 0 1 0 5z",
  history: "M12 3a9 9 0 1 0 9 9h-2a7 7 0 1 1-7-7zm-1 4v6l4.5 2.7 1-1.7-3.5-2.1V7zM13 3v2a7 7 0 0 1 6 6h2a9 9 0 0 0-8-8z",
  everyone:
    "M8 12.2a3.2 3.2 0 1 0 0-6.4 3.2 3.2 0 0 0 0 6.4zm9 .4a2.6 2.6 0 1 0 0-5.2 2.6 2.6 0 0 0 0 5.2zM2.5 19c.6-3 2.8-4.6 5.5-4.6s4.9 1.6 5.5 4.6zm12.1 0c-.2-1.5-.8-2.8-1.7-3.8 1-.6 2.3-.9 4.1-.9 2.4 0 4.1 1.5 4.5 4.700z",
  favourites: "M12 2.6l2.9 6 6.6.9-4.8 4.6 1.2 6.5L12 17.5l-5.9 3.1 1.2-6.5L2.5 9.5l6.6-.9z",
  events: "M7 2h2v2h6V2h2v2h3v17H4V4h3zm11 7H6v10h12zM8 11h3v3H8z",
  uploads: "M12 3l6 6h-4v6h-4V9H6zM4 17h16v4H4z",
  search: "M10.5 3a7.5 7.5 0 0 1 5.9 12.1l4.5 4.5-1.4 1.4-4.5-4.5A7.5 7.5 0 1 1 10.5 3zm0 2a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11z",
  inbox: "M4 4h16l2 9v7H2v-7zm1.6 2-1.4 6.500H9a3 3 0 0 0 6 0h4.800L18.4 6z",
  settings:
    "M10.3 2h3.400l.5 2.6 1.7.7 2.2-1.5 2.4 2.4-1.5 2.2.7 1.7 2.6.5v3.400l-2.6.5-.7 1.7 1.5 2.2-2.4 2.4-2.2-1.5-1.7.7-.5 2.600h-3.400l-.5-2.6-1.7-.7-2.2 1.5-2.4-2.4 1.5-2.2-.7-1.700L2 13.700v-3.400l2.6-.5.7-1.7-1.5-2.2 2.4-2.4 2.2 1.5 1.7-.7zM12 8.500a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7z",
};

const railHints: Record<View, string> = {
  around: "Friends by the world they are in",
  here: "The instance you are in and who is in it",
  history: "Worlds you visited and who was there",
  everyone: "Your whole friend list, and the log of who was added, removed or renamed",
  favourites: "Your favourite avatars, worlds and friends",
  events: "Events of your groups, the ones you follow, and reminders",
  uploads: "The avatars and worlds you uploaded",
  search: "Find people, worlds and avatars",
  inbox: "Friend requests, invites and other notifications",
  settings: "Size, rows, notifications, account",
};

const railNames: Record<View, string> = {
  around: "Around",
  here: "Here",
  history: "History",
  everyone: "Everyone",
  favourites: "Favourites",
  events: "Events",
  uploads: "Uploads",
  search: "Search",
  inbox: "Inbox",
  settings: "Settings",
};

export function App() {
  const [me, setMe] = useState<Me | null>(null);
  const [starting, setStarting] = useState(true);
  const [startProblem, setStartProblem] = useState("");
  const [friends, setFriends] = useState<Map<string, Friend>>(new Map());
  const [worlds, setWorlds] = useState<Map<string, World>>(new Map());
  const [feed, setFeed] = useState<FeedItem[]>([]);
  const [favorites, setFavorites] = useState<Favorites>({ groups: [], friends: {} });
  const [notices, setNotices] = useState<Notice[]>([]);
  const [instances, setInstances] = useState<Map<string, Instance>>(new Map());
  const [, setMinute] = useState(0);
  const [inWorld, setInWorld] = useState(false);
  const [myWorld, setMyWorld] = useState("");
  const [hereNow, setHereNow] = useState<HereNow | null>(null);
  const [link, setLink] = useState("off");
  const [statusOpen, setStatusOpen] = useState(false);
  const [service, setService] = useState<ServiceStatus | null>(null);

  useEffect(() => {
    // VRChat's own status page, read now and every ten minutes.
    let alive = true;
    const read = () =>
      void backend
        .serviceStatus()
        .then((s) => alive && setService(s))
        .catch(() => {});
    read();
    const timer = setInterval(read, 10 * 60_000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, []);
  const [notice, setNotice] = useState("");
  const [view, setView] = useState<View>(() => {
    // The browser demo can start on any screen with "?show=<screen>".
    const asked = backend.demo ? new URLSearchParams(location.search).get("show") : null;
    return asked && asked in titles ? (asked as View) : "around";
  });
  const [query, setQuery] = useState("");
  const [logTick, setLogTick] = useState(0);
  const [toast, setToast] = useState("");
  /** Open windows, newest last. Only the newest shows; Back goes to the one before it. */
  const [windows, setWindows] = useState<Array<{ kind: ThingKind | "profile"; id: string }>>(() => {
    // The browser demo can start with a window open: "?open=<user, world, avatar or group id>".
    const asked = backend.demo ? new URLSearchParams(location.search).get("open") : null;
    if (!asked) return [];
    const kind = asked.startsWith("wrld_") && asked.includes(":") ? "instance" : asked.startsWith("wrld_") ? "world" : asked.startsWith("avtr_") ? "avatar" : asked.startsWith("grp_") ? "group" : "profile";
    return [{ kind, id: asked }];
  });
  const top = windows[windows.length - 1];

  useEffect(() => {
    const push = (next: { kind: ThingKind | "profile"; id: string }) =>
      setWindows((old) => [...old.filter((w) => !(w.kind === next.kind && w.id === next.id)), next].slice(-20));
    const openThing = (e: Event) => push((e as CustomEvent).detail);
    const openPerson = (e: Event) => push({ kind: "profile", id: String((e as CustomEvent).detail) });
    window.addEventListener("open-thing", openThing);
    window.addEventListener("open-profile", openPerson);
    return () => {
      window.removeEventListener("open-thing", openThing);
      window.removeEventListener("open-profile", openPerson);
    };
  }, []);
  const back = windows.length > 1 ? () => setWindows((old) => old.slice(0, -1)) : undefined;
  const [noted, setNoted] = useState<Set<string>>(new Set());
  const [feedTick, setFeedTick] = useState(0);


  const loadNoted = () =>
    backend
      .noted()
      .then((ids) => setNoted(new Set(ids)))
      .catch(() => {});
  useEffect(() => {
    if (me) void loadNoted();
  }, [me]);
  const [closing, setClosing] = useState(false);
  const [always, setAlways] = useState(false);
  const { settings, change, fitted, picture, setPicture } = useSettings();

  // Discord shows where I am, as far as my settings allow. Sent only when something changed.
  const sentPresence = useRef("");
  useEffect(() => {
    if (!me) return;
    const presence = discordPresence(settings, hereNow, worlds);
    const text = JSON.stringify(presence);
    if (text === sentPresence.current) return;
    sentPresence.current = text;
    void backend.setDiscord(presence).catch(() => {});
  }, [me, hereNow, worlds, settings]);

  // The daily backup: looked at a minute after signing in and every hour after. The app
  // itself decides whether one is due, so asking often costs nothing.
  const settingsNow = useRef(settings);
  settingsNow.current = settings;
  useEffect(() => {
    if (!me || !settings.autoBackup) return;
    const run = () => {
      const s = settingsNow.current;
      void backend.autoBackup(JSON.stringify(s), s.autoBackupKeep).catch(() => {});
    };
    const first = window.setTimeout(run, 60_000);
    const hourly = window.setInterval(run, 3_600_000);
    return () => {
      window.clearTimeout(first);
      window.clearInterval(hourly);
    };
  }, [me, settings.autoBackup]);

  // History that came over from VRCX sits between the entries already shown and gives them
  // new numbers, so the feed is read again.
  useEffect(() => {
    if (!me) return;
    const again = () => void backend.feed(200).then(setFeed).catch(() => {});
    window.addEventListener("history:imported", again);
    return () => window.removeEventListener("history:imported", again);
  }, [me]);

  // My own worlds are noted a little after the start and twice a day, so their numbers have
  // a point for every day the app ran. One question each time.
  useEffect(() => {
    if (!me) return;
    const note = () => void backend.profilePart(me.id, "worlds", "", "").catch(() => {});
    const first = window.setTimeout(note, 90_000);
    const later = window.setInterval(note, 12 * 3_600_000);
    return () => {
      window.clearTimeout(first);
      window.clearInterval(later);
    };
  }, [me]);

  // A newer version: looked for a little after the start and every six hours after that.
  // `auto` installs it straight away, `tell` puts up a note with a button.
  const [update, setUpdate] = useState<{ version: string; notes: string } | null>(null);
  const [updating, setUpdating] = useState("");
  const installUpdate = (version: string) => {
    setUpdating(`Getting version ${version}`);
    void backend.updateInstall().catch((err) => {
      setUpdating("");
      setToast(`The update did not install: ${errorText(err)}`);
    });
  };
  useEffect(() => {
    if (settings.updates === "off") return;
    let alive = true;
    const look = (atStart: boolean) =>
      void backend
        .updateCheck()
        .then((found) => {
          if (!alive || !found) return;
          setUpdate(found);
          // Only right after the start: later in the day a restart would cut into whatever I am doing.
          if (atStart && settingsNow.current.updates === "auto") installUpdate(found.version);
        })
        // No connection, or GitHub is slow: the next look tries again.
        .catch(() => {});
    const first = window.setTimeout(() => look(true), 12_000);
    const later = window.setInterval(() => look(false), 6 * 3_600_000);
    return () => {
      alive = false;
      window.clearTimeout(first);
      window.clearInterval(later);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings.updates === "off"]);
  useEffect(() => {
    if (!updating) return;
    let stop: (() => void) | undefined;
    let gone = false;
    void backend
      .on("update:progress", (p) => {
        if (p.total > 0) setUpdating(`Getting the new version, ${Math.round((p.done / p.total) * 100)}%`);
      })
      .then((off) => (gone ? off() : (stop = off)));
    return () => {
      gone = true;
      stop?.();
    };
  }, [updating !== ""]);

  // What VRCX has written since the last look is taken over without asking, when I want that.
  useEffect(() => {
    if (!me || !settings.vrcxAuto) return;
    const key = `vrcx-seen:${me.id}`;
    const look = async () => {
      try {
        const stamp = await backend.vrcxStamp();
        if (!stamp || localStorage.getItem(key) === stamp) return;
        // The first look reads everything; later ones only what VRCX wrote since two days
        // before the last look, which is short however long the history is.
        const mark = `vrcx-mark:${me.id}`;
        const started = Date.now();
        const { broughtOver: got } = await backend.importVrcx(JSON.stringify(settingsNow.current), true, localStorage.getItem(mark) ?? undefined);
        localStorage.setItem(key, stamp);
        localStorage.setItem(mark, new Date(started - 2 * 86_400_000).toISOString());
        const total = got.feed + got.friendLog + got.visits + got.notes + got.favourites + got.avatars + got.tags;
        if (total > 0) {
          setToast(`Brought over ${total.toLocaleString()} new ${total === 1 ? "entry" : "entries"} from VRCX.`);
          window.dispatchEvent(new Event("history:imported"));
        }
      } catch {
        // VRCX may be in the middle of writing; the next look tries again.
      }
    };
    const first = window.setTimeout(() => void look(), 8_000);
    const hourly = window.setInterval(() => void look(), 3_600_000);
    return () => {
      window.clearTimeout(first);
      window.clearInterval(hourly);
    };
  }, [me, settings.vrcxAuto]);

  // Which overlay programs get my notifications while I am in the headset.
  useEffect(() => {
    void backend
      .setVrNotices({ own: settings.vrOwn, xs: settings.vrXs, ovrHud: settings.vrOvrHud, ovrWrist: settings.vrOvrWrist, seconds: settings.vrSeconds, opacity: settings.vrOpacity })
      .catch(() => {});
  }, [settings.vrOwn, settings.vrXs, settings.vrOvrHud, settings.vrOvrWrist, settings.vrSeconds, settings.vrOpacity]);


  // The title names the tab that is open on screens that have tabs.
  const [tabTitle, setTabTitle] = useState("");
  useEffect(() => {
    const read = () => {
      const on = document.querySelector<HTMLElement>(".stage > .table-wrap > .tabs [aria-selected='true'], .stage .tabs[role='tablist'] [aria-selected='true']");
      const label = (on?.textContent ?? "").split(",")[0].trim();
      setTabTitle(tabTitles[view]?.[label] ?? "");
    };
    read();
    const later = () => window.setTimeout(read, 0);
    document.addEventListener("click", later);
    const soon = window.setTimeout(read, 120);
    return () => {
      document.removeEventListener("click", later);
      window.clearTimeout(soon);
    };
  }, [view]);

  // Ctrl+Shift+H hides or shows names and pictures from anywhere in the app.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === "h") {
        e.preventDefault();
        change({ hidePeople: !settings.hidePeople });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [settings.hidePeople, change]);

  useEffect(() => {
    void backend
      .setAlerts({
        invites: settings.alertInvites,
        friendRequests: settings.alertFriendRequests,
        boops: settings.alertBoops,
        everythingElse: settings.alertEverythingElse,
      })
      .catch(() => {});
  }, [settings.alertInvites, settings.alertFriendRequests, settings.alertBoops, settings.alertEverythingElse]);
  useEffect(() => {
    void backend
      .setGameWatch({ openWithGame: settings.openWithGame, rejoin: settings.rejoin, launchMode: settings.launchMode, clearCache: settings.clearCacheOnClose })
      .catch(() => {});
  }, [settings.openWithGame, settings.rejoin, settings.launchMode, settings.clearCacheOnClose]);
  useEffect(() => {
    let stop: (() => void) | undefined;
    let gone = false;
    void backend
      .on("cache:cleared", (c) => {
        setToast(`VRChat closed. Its cache was emptied, ${(c.bytes / 1024 ** 3).toFixed(1)} GB freed.`);
        window.dispatchEvent(new Event("cache:cleared"));
      })
      .then((off) => (gone ? off() : (stop = off)));
    return () => {
      gone = true;
      stop?.();
    };
  }, []);
  useEffect(() => {
    void backend.setOwnFeed(settings.feedMine).catch(() => {});
  }, [settings.feedMine, me]);
  // The browser demo can open a tab on the screen ("?tab=Worlds"), a tab in the open window
  // ("?wtab=Avatars") and run a search ("?q=robot"), so every view can be looked at by address.
  useEffect(() => {
    if (!backend.demo || !me) return;
    const asked = new URLSearchParams(location.search);
    const press = (within: string, label: string | null) => {
      if (!label) return;
      const all = [...document.querySelectorAll<HTMLButtonElement>(`${within} button`)];
      all.find((b) => (b.textContent ?? "").trim().toLowerCase().startsWith(label.toLowerCase()))?.click();
    };
    const timers = [
      window.setTimeout(() => press(".stage .tabs", asked.get("tab")), 500),
      window.setTimeout(() => press(".profile-top .tabs", asked.get("wtab")), 1400),
      window.setTimeout(() => {
        const q = asked.get("q");
        const box = document.querySelector<HTMLInputElement>(".search-bar input");
        if (!q || !box) return;
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(box, q);
        box.dispatchEvent(new Event("input", { bubbles: true }));
        box.form?.requestSubmit();
      }, 1200),
    ];
    return () => timers.forEach((t) => window.clearTimeout(t));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [me?.id]);

  // Lists kept outside this screen's state are told when the account changes.
  const meIdNow = me?.id ?? "";
  useEffect(() => {
    window.dispatchEvent(new Event("account:changed"));
  }, [meIdNow]);
  useEffect(() => {
    const minutes = (text: string) => {
      const [h, m] = text.split(":").map(Number);
      return (Number.isFinite(h) ? h : 0) * 60 + (Number.isFinite(m) ? m : 0);
    };
    void backend
      .setQuiet({ hours: settings.quietHours, from: minutes(settings.quietFrom), to: minutes(settings.quietTo), inGame: settings.quietInGame })
      .catch(() => {});
  }, [settings.quietHours, settings.quietFrom, settings.quietTo, settings.quietInGame]);
  // Reminders for the events I follow. The list is read when I sign in, every half hour and
  // when I follow or unfollow something; each event is announced once, the set number of
  // minutes before it starts. It works with the window in the tray.
  useEffect(() => {
    const lead = settings.eventReminder;
    if (!me || !lead) return;
    let alive = true;
    let following: CalendarEvent[] = [];
    const KEY = "event-reminders";
    const told = new Set<string>();
    try {
      for (const k of JSON.parse(localStorage.getItem(KEY) ?? "[]") as string[]) told.add(k);
    } catch {
      /* a broken list only means a reminder may come twice */
    }
    const read = () => {
      const now = new Date();
      // Near the end of a month an event can start in the next one.
      const months = [monthOf(now), monthOf(new Date(now.getTime() + lead * 60_000 + 86_400_000))];
      void Promise.all([...new Set(months)].map((m) => backend.calendar("following", m).catch(() => ({ events: [] as CalendarEvent[], more: false })))).then((pages) => {
        if (alive) following = pages.flatMap((p) => p.events).filter((e) => e.following);
      });
    };
    const check = () => {
      const now = Date.now();
      for (const e of following) {
        const starts = Date.parse(e.startsAt);
        const key = `${e.id}@${e.startsAt}`;
        if (!(starts > now && starts - now <= lead * 60_000) || told.has(key)) continue;
        told.add(key);
        try {
          localStorage.setItem(KEY, JSON.stringify([...told].slice(-200)));
        } catch {
          /* not being able to save it only means a reminder may come twice */
        }
        const minutes = Math.max(1, Math.round((starts - now) / 60_000));
        void backend.notify(`${e.title} starts in ${minutes} min`, e.groupName ? `An event of ${e.groupName}` : "An event you follow");
      }
    };
    read();
    const first = window.setTimeout(check, 20_000);
    const everyMinute = window.setInterval(check, 60_000);
    const everyHalfHour = window.setInterval(read, 30 * 60_000);
    window.addEventListener("events:changed", read);
    return () => {
      alive = false;
      window.clearTimeout(first);
      window.clearInterval(everyMinute);
      window.clearInterval(everyHalfHour);
      window.removeEventListener("events:changed", read);
    };
  }, [me?.id, settings.eventReminder]);
  // VRChat closed without warning: where I was, until I go back or put the note away.
  const [crash, setCrash] = useState<{ location: string; worldName: string } | null>(null);
  const asked = useRef(false);

  // Live event handlers read the newest values through these.
  const now = useRef({ favorites, settings, worlds });
  now.current = { favorites, settings, worlds };

  async function restore() {
    setStarting(true);
    setStartProblem("");
    try {
      const reply = await backend.restore();
      setMe(reply.me);
    } catch (err) {
      setStartProblem(errorText(err));
    } finally {
      setStarting(false);
    }
  }

  useEffect(() => {
    if (asked.current) return;
    asked.current = true;
    void restore();
  }, []);

  // The close button asks first, unless a choice was saved or nobody is signed in.
  const signedIn = useRef(false);
  signedIn.current = me !== null;
  const meNow = useRef("");
  meNow.current = me?.id ?? "";
  useEffect(() => {
    const stop = backend.on("window:close-asked", () => {
      const choice = now.current.settings.onClose;
      if (!signedIn.current || choice === "quit") void backend.quit();
      else if (choice === "tray") void backend.hideToTray();
      else setClosing(true);
    });
    return () => void stop.then((off) => off());
  }, []);

  // Redraw once a minute so "12 min" keeps counting.
  useEffect(() => {
    const timer = window.setInterval(() => setMinute((n) => n + 1), 60_000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(""), 5000);
    return () => window.clearTimeout(timer);
  }, [toast]);

  function finishClose(choice: "tray" | "quit") {
    if (always) change({ onClose: choice });
    setClosing(false);
    setAlways(false);
    // Give the saved choice a moment to reach storage before the window goes.
    window.setTimeout(() => void (choice === "tray" ? backend.hideToTray() : backend.quit()), 60);
  }

  useEffect(() => {
    if (!me) return;
    let alive = true;
    void backend
      .here()
      .then((h) => {
        if (!alive) return;
        setInWorld(h !== null);
        setMyWorld(h ? h.visit.worldName || "a world" : "");
        setHereNow(h);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [me, logTick]);

  useEffect(() => {
    if (!me) return;
    let alive = true;
    const stops: Array<() => void> = [];
    const hold = (p: Promise<() => void>) => p.then((stop) => (alive ? stops.push(stop) : stop()));

    const loadFriends = () =>
      backend.friends().then((list) => alive && setFriends(new Map(list.map((f) => [f.id, f]))));
    const loadFavorites = () => backend.favorites().then((f) => alive && setFavorites(f));
    const loadNotices = () => backend.notices().then((list) => alive && setNotices(list));

    void loadFriends();
    void loadFavorites();
    void loadNotices();
    void backend.worlds().then((list) => alive && setWorlds(new Map(list.map((w) => [w.id, w]))));
    void backend.instances().then((list) => alive && setInstances(new Map(list.map((i) => [i.location, i]))));
    void backend.feed(200).then((list) => alive && setFeed(list)).catch(() => {});
    void backend.pipelineStatus().then((s) => alive && setLink(s));

    void hold(backend.on("friends:reset", () => void loadFriends()));
    void hold(backend.on("favorites:reset", () => void loadFavorites()));
    void hold(backend.on("notices:changed", () => void loadNotices()));
    void hold(backend.on("friend:changed", (f) => setFriends((old) => new Map(old).set(f.id, f))));
    void hold(
      backend.on("friend:removed", (id) =>
        setFriends((old) => {
          const next = new Map(old);
          next.delete(id);
          return next;
        }),
      ),
    );
    void hold(
      backend.on("feed:new", (item) => {
        setFeed((old) => [item, ...old].slice(0, 500));
        if (item.kind === "friendAdded" || item.kind === "friendRemoved" || item.kind === "name") setFeedTick((n) => n + 1);
        const { favorites, settings, worlds } = now.current;
        // A rule set on one person comes before their favourite group's.
        const own = settings.alertPeople?.[item.userId]?.rule;
        if (own === "never" || item.userId === me?.id) return;
        if (own !== "always") {
          if (!(item.userId in favorites.friends)) return;
          // A favourite group can be told to stay quiet.
          if ((settings.alertGroupsOff ?? []).includes(favorites.friends[item.userId])) return;
        }
        const world = item.worldId ? (worlds.get(item.worldId)?.name ?? "a world") : "";
        if (item.kind === "online" && settings.alertFavouriteOnline) {
          void backend.notify(`${item.displayName} is online`, world ? `In ${world}` : "In a private world");
        } else if (item.kind === "location" && item.previous === "private" && world && settings.alertFavouriteLeavesPrivate) {
          void backend.notify(`${item.displayName} left the private world`, `Now in ${world}`);
        }
      }),
    );
    void hold(
      backend.on("notice:new", (n) => {
        // The Windows pop-up comes from the app itself; the window shows a short note too.
        if (n.senderId === meNow.current) return;
        const { title } = noticeLine(n);
        setToast(`${title}. See the Inbox.`);
      }),
    );
    void hold(backend.on("here:changed", () => setLogTick((n) => n + 1)));
    void hold(backend.on("world:resolved", (w) => setWorlds((old) => new Map(old).set(w.id, w))));
    void hold(backend.on("instance:info", (i) => setInstances((old) => new Map(old).set(i.location, i))));
    void hold(backend.on("pipeline:status", (s) => setLink(s)));
    void hold(backend.on("sync:error", (e) => setNotice(e.message)));
    void hold(
      backend.on("game:crashed", (c) => {
        if (c.rejoined) {
          setCrash(null);
          setToast(`VRChat closed without warning. Starting it again in ${c.worldName}.`);
        } else {
          setCrash({ location: c.location, worldName: c.worldName });
        }
      }),
    );
    void hold(
      backend.on("session:ended", () => {
        void backend.setDiscord(null).catch(() => {});
        setNotice("");
        setMe(null);
      }),
    );
    return () => {
      alive = false;
      stops.forEach((stop) => stop());
    };
  }, [me]);

  const list = useMemo(() => [...friends.values()], [friends]);
  const counts = useMemo(
    () => ({
      online: list.filter((f) => f.state === "online").length,
      active: list.filter((f) => f.state === "active").length,
      total: list.length,
    }),
    [list],
  );

  async function signOut() {
    // Discord stops showing where I am the moment I sign out.
    void backend.setDiscord(null).catch(() => {});
    await backend.logout().catch(() => {});
    setFriends(new Map());
    setFeed([]);
    setNotices([]);
    setView("around");
    setMe(null);
  }

  if (starting) {
    return <div className="splash">Checking your saved sign-in</div>;
  }
  if (!me) {
    return (
      <>
        {startProblem && (
          <div className="banner" role="alert">
            {startProblem} <button onClick={() => void restore()}>Try again</button>
          </div>
        )}
        <Login onSignedIn={setMe} />
      </>
    );
  }

  const railButton = (id: View, badge = 0) => (
    <button className={view === id ? "on" : ""} onClick={() => setView(id)} title={railHints[id]}>
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d={icons[id]} fillRule="evenodd" />
      </svg>
      <span>{railNames[id]}</span>
      {badge > 0 && (
        <b className="badge" aria-label={`${badge} waiting`}>
          {badge}
        </b>
      )}
    </button>
  );

  const searchable = view === "around" || view === "everyone";

  return (
    <div className={settings.activityShown ? "shell" : "shell shell-wide"}>
      <nav className="rail" aria-label="Sections">
        <img className="rail-logo" src={logo} alt="Anonymous VRX" />
        {railButton("around")}
        {railButton("here")}
        {railButton("history")}
        {railButton("everyone")}
        {railButton("favourites")}
        {railButton("uploads")}
        {railButton("events")}
        {railButton("search")}
        {railButton("inbox", notices.length)}
        <div className="rail-end">
          <span className={`link link-${link}`} title={linkWords[link] ?? link}>
            <i />
            {linkWords[link] ?? link}
          </span>
          {service && service.indicator !== "none" && (
            <span className={`service service-${service.indicator}`} title={`VRChat reports: ${service.description}`}>
              VRChat issues
            </span>
          )}
          {railButton("settings")}
          <button className="me" onClick={() => setStatusOpen(true)} title="Your status">
            <Avatar name={me.displayName} image={me.image} size={36} />
          </button>
        </div>
      </nav>

      <section className="stage">
        <header className="stage-head">
          <div>
            <h1>{tabTitle || titles[view]}</h1>
            <p className="tally">
              <span><b>{counts.online}</b> in VRChat</span>
              <span><b>{counts.active}</b> on the website or phone</span>
              <span><b>{counts.total}</b> friends</span>
              <button className="you" onClick={() => setView("here")} title="Open Here">
                <i className={myWorld ? "in" : ""} />
                {myWorld ? <>You are in <b>{tidy(myWorld)}</b></> : "You are not in a world"}
              </button>
            </p>
          </div>
          {searchable && (
            <input
              className="search"
              type="search"
              placeholder="Find a friend or a world"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              aria-label="Find a friend or a world"
            />
          )}
        </header>
        {backend.demo && <p className="demo-strip">Demo data. Open the desktop app to see your own friends.</p>}
        {update && (
          <div className="banner crash-note update-note" role="status">
            <span>{updating ? `${updating}. The app closes and opens again by itself.` : `Version ${update.version} of Anonymous VRX is out.`}</span>
            {!updating && (
              <>
                <button className="go" onClick={() => installUpdate(update.version)}>
                  Update now
                </button>
                <button onClick={() => setUpdate(null)}>Later</button>
              </>
            )}
          </div>
        )}
        {crash && (
          <div className="banner crash-note" role="alert">
            <span>VRChat closed without warning while you were in {crash.worldName}.</span>
            <button
              className="go"
              onClick={() => {
                const where = crash.location;
                setCrash(null);
                void backend
                  .launchInstance(where, settings.launchMode)
                  .then(() => setToast("Starting VRChat where you were."))
                  .catch((err) => setToast(`That did not work: ${errorText(err)}`));
              }}
            >
              Go back
            </button>
            <button onClick={() => setCrash(null)}>Not now</button>
          </div>
        )}
        {notice && (
          <p className="banner" role="alert">
            {notice} <button onClick={() => setNotice("")}>Dismiss</button>
          </p>
        )}
        {view === "around" && (
          <Around friends={list} worlds={worlds} favorites={favorites} instances={instances} query={query} onDone={setToast} />
        )}
        {view === "here" && <Here friends={friends} worlds={worlds} instances={instances} meId={me.id} tick={logTick} onDone={setToast} />}
        {view === "history" && <History friends={friends} worlds={worlds} meId={me.id} tick={logTick} onDone={setToast} />}
        {view === "everyone" && (
          <Everyone friends={list} friendMap={friends} worlds={worlds} favorites={favorites} noted={noted} feedTick={feedTick} query={query} onDone={setToast} />
        )}
        {view === "uploads" && <Uploads meId={me.id} onDone={setToast} />}
        {view === "favourites" && <Favourites friends={friends} favorites={favorites} meId={me.id} onDone={setToast} />}
        {view === "events" && <Events onDone={setToast} />}
        {view === "search" && <Search onDone={setToast} />}
        {view === "inbox" && <Inbox notices={notices} friends={friends} inWorld={inWorld} meId={me.id} onDone={setToast} />}
        {view === "settings" && (
          <SettingsPage
            settings={settings}
            change={change}
            fitted={fitted}
            picture={picture}
            setPicture={setPicture}
            me={me}
            favorites={favorites}
            onSignOut={() => void signOut()}
          />
        )}
      </section>

      {!settings.activityShown && (
        <button className="activity-back" onClick={() => change({ activityShown: true })} title="Show the Activity panel again">
          Activity
        </button>
      )}
      <Activity
        open={settings.activityShown}
        onPutAway={() => change({ activityShown: false })}
        feed={feed}
        worlds={worlds}
        friends={friends}
        hidden={settings.feedHidden}
        foldRepeats={settings.feedFold}
        onHide={(id, name) => {
          change({ feedHidden: { ...settings.feedHidden, [id]: name } });
          setToast(`${name} is left out of the feed. Bring them back in Settings.`);
        }}
      />

      {top?.kind === "profile" && (
        <ProfilePanel
          key={top.id}
          userId={top.id}
          meId={me.id}
          friends={friends}
          worlds={worlds}
          inWorld={inWorld}
          alertRule={settings.alertPeople?.[top.id]?.rule}
          setAlertRule={(rule, name) => {
            const next = { ...(settings.alertPeople ?? {}) };
            if (rule) next[top.id] = { name, rule };
            else delete next[top.id];
            change({ alertPeople: next });
          }}
          onBack={back}
          onClose={() => {
            setWindows([]);
            void loadNoted();
          }}
          onDone={setToast}
        />
      )}

      {top?.kind === "instance" && <InstancePanel key={top.id} location={top.id} friends={friends} onBack={back} onClose={() => setWindows([])} onDone={setToast} />}

      {top && top.kind !== "profile" && top.kind !== "instance" && (
        <DetailPanel key={top.kind + top.id} kind={top.kind} id={top.id} meId={me.id} onBack={back} onClose={() => setWindows([])} onDone={setToast} />
      )}


      <PictureViewer onDone={setToast} />

      {statusOpen && (
        <MyStatus
          me={me}
          onClose={() => setStatusOpen(false)}
          onSaved={(next, message) => {
            setMe(next);
            setStatusOpen(false);
            setToast(message);
          }}
        />
      )}

      <PersonMenu isFriend={(id) => friends.has(id)} inWorld={inWorld} openProfile={openProfile} onDone={setToast} />
      {toast && (
        <p className="toast" role="status">
          {toast}
        </p>
      )}

      {closing && (
        <div className="veil" onClick={() => setClosing(false)}>
          <div className="dialog" role="dialog" aria-modal="true" aria-labelledby="close-title" onClick={(e) => e.stopPropagation()}>
            <h2 id="close-title">Close Anonymous VRX?</h2>
            <p>
              Keep it running in the tray to go on recording friend activity and getting notifications. The icon
              sits by the clock, bottom right. If you quit, nothing is recorded until you start it again.
            </p>
            <label className="dialog-check">
              <input type="checkbox" checked={always} onChange={(e) => setAlways(e.target.checked)} />
              Do this every time
            </label>
            <div className="dialog-actions">
              <button onClick={() => setClosing(false)}>Cancel</button>
              <button onClick={() => finishClose("quit")}>Quit</button>
              <button className="go" autoFocus onClick={() => finishClose("tray")}>
                Keep running in the tray
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export function FriendChip({
  f,
  note,
  starred,
  showSince,
  tag,
  small,
}: {
  f: Friend;
  note?: string;
  starred?: boolean;
  showSince?: boolean;
  tag?: string;
  /** Name and picture only, for crowded places. The rest moves into the tooltip. */
  small?: boolean;
}) {
  const line = note ?? f.statusDescription;
  const words = showSince ? sinceWords(f.since) : "";
  const since = words && f.sinceAtLeast && words !== "just now" ? `${words}+` : words;
  if (small) {
    const extra = [line, since && `here ${since}`].filter(Boolean).join(", ");
    return (
      <li
        className="chip chip-small"
        data-person={f.id}
        data-person-name={f.displayName}
        title={extra ? `${f.displayName}: ${extra}` : f.displayName}
        role="button"
        tabIndex={0}
        onClick={() => openProfile(f.id)}
        onKeyDown={(e) => e.key === "Enter" && openProfile(f.id)}
      >
        <Avatar name={f.displayName} image={f.image} tone={statusTone(f)} size={24} />
        <span className="chip-name">
          {f.displayName}
          {starred && <span className="star"> ★</span>}
        </span>
      </li>
    );
  }
  return (
    <li className="chip" data-person={f.id} data-person-name={f.displayName} role="button" tabIndex={0} title="Open profile. Right-click for invites and a boop" onClick={() => openProfile(f.id)} onKeyDown={(e) => e.key === "Enter" && openProfile(f.id)}>
      <Avatar name={f.displayName} image={f.image} tone={statusTone(f)} />
      <span className="chip-text">
        <span className="chip-name">
          {f.displayName}
          {starred && <span className="star" title="Favourite"> ★</span>}
        </span>
        {line && <span className="chip-note">{line}</span>}
      </span>
      {since && (
        <time
          className="chip-since"
          dateTime={f.since}
          title={
            f.sinceAtLeast
              ? "They were already here when this app started, so it is at least this long"
              : "Time in this world"
          }
        >
          {since}
        </time>
      )}
      {tag && <span className="chip-since chip-tag">{tag}</span>}
    </li>
  );
}
