import { useEffect, useRef, useState } from "react";
import { backend, errorText, type Friend, type HereNow, type Me } from "./backend";
import { parsePlace } from "./place";
import type { Settings } from "./settings";

export type StatusValue = "join me" | "active" | "ask me" | "busy";

/** One rule: when this is true, my status becomes that. */
export interface StatusRule {
  id: string;
  on: boolean;
  when: "time" | "access" | "world" | "friends";
  /** For `time`: from and to on the clock, "22:00". A span may run past midnight. */
  from: string;
  to: string;
  /** For `access`: the kind of instance I am in. */
  access: "invite" | "friends" | "group" | "public";
  /** For `world`: part of the world's name. */
  words: string;
  /** For `friends`: at least this many friends in my instance. */
  count: number;
  status: StatusValue;
  /** The status text to set. Empty leaves my own text alone. */
  text: string;
}

export const statusNames: Record<StatusValue, string> = {
  "join me": "Join me",
  active: "Online",
  "ask me": "Ask me",
  busy: "Do not disturb",
};

export const mostRules = 12;

export function newRule(): StatusRule {
  return { id: `r${Date.now().toString(36)}${Math.floor(Math.random() * 1296).toString(36)}`, on: true, when: "access", from: "23:00", to: "07:00", access: "invite", words: "", count: 3, status: "busy", text: "" };
}

const clock = (v: unknown, fallback: string): string => (typeof v === "string" && /^([01]\d|2[0-3]):[0-5]\d$/.test(v) ? v : fallback);

/** Rules as read from a settings file, with anything this version does not know put right. */
export function cleanRules(value: unknown): StatusRule[] {
  if (!Array.isArray(value)) return [];
  const seen = new Set<string>();
  const rules: StatusRule[] = [];
  for (const raw of value) {
    if (!raw || typeof raw !== "object") continue;
    const r = raw as Partial<StatusRule>;
    const base = newRule();
    const id = typeof r.id === "string" && r.id && !seen.has(r.id) ? r.id : base.id + rules.length;
    seen.add(id);
    rules.push({
      id,
      on: r.on !== false,
      when: (["time", "access", "world", "friends"] as const).includes(r.when as never) ? (r.when as StatusRule["when"]) : base.when,
      from: clock(r.from, base.from),
      to: clock(r.to, base.to),
      access: (["invite", "friends", "group", "public"] as const).includes(r.access as never) ? (r.access as StatusRule["access"]) : base.access,
      words: typeof r.words === "string" ? r.words.slice(0, 60) : "",
      count: typeof r.count === "number" && r.count >= 1 && r.count <= 80 ? Math.round(r.count) : base.count,
      status: r.status && r.status in statusNames ? r.status : base.status,
      text: typeof r.text === "string" ? r.text.slice(0, 32) : "",
    });
    if (rules.length >= mostRules) break;
  }
  return rules;
}

/** What the rules look at. */
export interface Scene {
  /** Minutes since midnight. */
  minutes: number;
  /** The kind of instance I am in, as `parsePlace` names it; null when I am in no world. */
  access: string | null;
  world: string;
  friendsHere: number;
}

const minutesOf = (clockText: string): number => Number(String(clockText ?? "").slice(0, 2)) * 60 + Number(String(clockText ?? "").slice(3, 5));

export function fits(rule: StatusRule, scene: Scene): boolean {
  if (!rule.on) return false;
  if (rule.when === "time") {
    const [from, to] = [minutesOf(rule.from), minutesOf(rule.to)];
    if (from === to) return false;
    return from < to ? scene.minutes >= from && scene.minutes < to : scene.minutes >= from || scene.minutes < to;
  }
  if (scene.access === null) return false;
  if (rule.when === "access") {
    if (rule.access === "invite") return scene.access === "invite" || scene.access === "invitePlus";
    if (rule.access === "friends") return scene.access === "friends" || scene.access === "friendsPlus";
    if (rule.access === "group") return scene.access.startsWith("group");
    return scene.access === "public";
  }
  if (rule.when === "world") {
    // Rules are cleaned when settings are read; a hand-edited one without words still must not stop the window.
    const words = String(rule.words ?? "").trim().toLowerCase();
    return words !== "" && scene.world.toLowerCase().includes(words);
  }
  return scene.friendsHere >= rule.count;
}

/** The first rule that fits wins. */
export function firstFit(rules: StatusRule[], scene: Scene): StatusRule | null {
  return rules.find((r) => fits(r, scene)) ?? null;
}

type Pair = { status: string; text: string };

/** What the app remembers, per account, while a rule holds my status. */
interface Held {
  rule: string;
  /** The status I had before the first rule took over; null once I changed it myself. */
  own: Pair | null;
  /** What the rule set. */
  set: Pair;
  /** I changed my status myself while this rule fitted, so it is left alone until the rule is over. */
  paused: boolean;
  /** VRChat has not confirmed the rule's status yet: the call is under way, or it failed and is tried again. */
  pending?: boolean;
  /** When the rule's status was sent. */
  at?: number;
}

const KEY = "status-rules-held";
const LOG = "status-rules-log";

function everyone(): Record<string, Held> {
  try {
    const all = JSON.parse(localStorage.getItem(KEY) ?? "{}") as Record<string, Held>;
    return all && typeof all === "object" && !Array.isArray(all) ? all : {};
  } catch {
    return {};
  }
}

function held(me: string): Held | null {
  const h = everyone()[me];
  return h && typeof h.rule === "string" && h.set && typeof h.set.status === "string" ? h : null;
}

let fitting = "";
let who = "";
let sceneWords = "";

function tell() {
  window.dispatchEvent(new Event("status-rule:changed"));
}

/** Each account has its own record, so signing in as someone else and back loses nothing. */
function keep(me: string, next: Held | null) {
  try {
    const all = everyone();
    if (next) all[me] = next;
    else delete all[me];
    localStorage.setItem(KEY, JSON.stringify(all));
  } catch {
    // Without storage the rule still applies; only the way back after a restart is lost.
  }
  tell();
}

export interface RuleNote {
  at: number;
  text: string;
}

/** What the rules saw and did lately, newest first. It stays on this PC. */
export function ruleLog(): RuleNote[] {
  try {
    const list = JSON.parse(localStorage.getItem(LOG) ?? "[]") as RuleNote[];
    return Array.isArray(list) ? list.filter((n) => n && typeof n.at === "number" && typeof n.text === "string") : [];
  } catch {
    return [];
  }
}

function note(text: string) {
  try {
    const list = ruleLog();
    if (list[0]?.text === text) return;
    localStorage.setItem(LOG, JSON.stringify([{ at: Date.now(), text }, ...list].slice(0, 40)));
  } catch {
    // The log is a help, nothing depends on it.
  }
  tell();
}

export function clearRuleLog() {
  try {
    localStorage.removeItem(LOG);
  } catch {
    // Nothing to clear then.
  }
  tell();
}

const show = (p: Pair) => `${statusNames[p.status as StatusValue] ?? p.status}${p.text ? ` "${p.text}"` : ""}`;

/** For the parts of the window that show it: which rule fits right now, whether I overrode it, and where the app sees me. */
export function ruleNow(): { fit: string; paused: boolean; holding: boolean; scene: string } {
  const h = who ? held(who) : null;
  return { fit: fitting, paused: h !== null && h.paused && h.rule === fitting, holding: h !== null && !h.paused, scene: sceneWords };
}

/** Draws again whenever the rule state changes. */
export function useRuleNow() {
  const [, again] = useState(0);
  useEffect(() => {
    const on = () => again((n) => n + 1);
    window.addEventListener("status-rule:changed", on);
    return () => window.removeEventListener("status-rule:changed", on);
  }, []);
  return ruleNow();
}

/**
 * I set my status by hand in the app. While a rule fits, that rule is told to leave it alone
 * until it is over; this does not wait for the app to notice the change by itself.
 */
function statusSetByHand(me: string, mine: Pair) {
  byHand = { ...mine, at: Date.now() };
  if (!fitting) return;
  const h = held(me);
  keep(me, { rule: fitting, own: null, set: h?.set ?? mine, paused: true });
  note(`You set ${show(mine)} yourself in the app. The rules leave it alone until none fits any more.`);
}
/** What I last set by hand in the app, for a minute: long enough for the engine to see it once. */
let byHand: (Pair & { at: number }) | null = null;
/** A change of the rules' own that is on its way to VRChat. */
let ruleCall: Promise<void> | null = null;
/** A change by hand is on its way to VRChat; the rules wait. */
let handBusy = false;

/**
 * Saves a status I picked in the app. It goes out after any change of the rules that is
 * still on its way, never alongside it, so mine is the one VRChat ends up with.
 */
export async function saveStatusByHand(me: string, mine: Pair) {
  handBusy = true;
  try {
    if (ruleCall) await ruleCall.catch(() => {});
    await backend.setStatus(mine.status, mine.text);
    statusSetByHand(me, mine);
  } finally {
    handBusy = false;
  }
}

/** VRChat is asked to change my status at most this often, however fast the rules flip. VRChat's
 *  own word to makers of outside apps is one request a minute for the same thing. */
const GAP = 60_000;
/** A rule has to fit for this long before it acts, so walking through a world for a moment changes nothing. */
const SETTLE = 4_000;
/** With no world open, no rule has to fit for this long before my status goes back. Loading into the next world is not "no rule". */
const LEAVE = 45_000;
/** A change VRChat could not be reached for is tried again after this long. */
const AGAIN = 120_000;
/** For this long after a change, a slightly different text coming back is VRChat's spelling of mine, not a change of my own. */
const ECHO = 60_000;

const plain = (text: string) => text.trim().replace(/\s+/g, " ");
const alike = (a: Pair, b: Pair) => a.status === b.status && plain(a.text) === plain(b.text);

const accessWords: Record<string, string> = {
  public: "a Public instance",
  friendsPlus: "a Friends+ instance",
  friends: "a Friends instance",
  invitePlus: "an Invite+ instance",
  invite: "an Invite instance",
  group: "a group instance",
  groupPlus: "a Group+ instance",
  groupPublic: "a Group public instance",
};

/**
 * Keeps my status in step with the rules. It acts only when the rule that fits changes: it
 * remembers the status I had, sets the rule's, and puts mine back when no rule fits any more.
 * A status I set myself in the meantime, here or in VRChat, is left alone until that rule is over.
 */
export function useStatusRules(me: Me | null, setMe: (update: (old: Me | null) => Me | null) => void, settings: Settings, here: HereNow | null, friends: Map<string, Friend>, onDone: (message: string) => void) {
  const [tick, setTick] = useState(0);
  who = me?.id ?? "";
  // The clock matters for time rules, for the wait before my status goes back, and for trying again what did not go through.
  const ticking = me !== null && (settings.statusRulesOn || held(me.id) !== null);
  useEffect(() => {
    if (!ticking) return;
    const id = window.setInterval(() => setTick((n) => n + 1), 15_000);
    return () => window.clearInterval(id);
  }, [ticking]);

  const date = new Date();
  const scene: Scene = {
    minutes: date.getHours() * 60 + date.getMinutes(),
    access: here ? parsePlace(here.visit.location).access : null,
    world: here?.visit.worldName ?? "",
    friendsHere: here ? here.people.filter((p) => p.leftAt === null && friends.has(p.userId)).length : 0,
  };
  const words = here ? `${scene.world || "a world"}, ${accessWords[scene.access ?? ""] ?? "an instance of a kind the app does not know"}, ${scene.friendsHere} ${scene.friendsHere === 1 ? "friend" : "friends"} here` : "no world. VRChat's log shows none open";
  const fit = me && settings.statusRulesOn ? firstFit(settings.statusRules, scene) : null;
  const target = fit?.id ?? "";
  const noneSince = useRef(Date.now());
  if (fitting !== target || sceneWords !== words) {
    if (fitting !== target && target === "") noneSince.current = Date.now();
    fitting = target;
    sceneWords = words;
    window.setTimeout(tell, 0);
  }

  const working = useRef(false);
  const lastCall = useRef(0);
  const failed = useRef({ mark: "", at: 0, told: false });
  const lastTarget = useRef<string | null>(null);
  const hiddenTold = useRef(false);
  /** My status as the engine last saw it, and what it last set itself. */
  const seen = useRef<(Pair & { id: string }) | null>(null);
  const sent = useRef<Pair | null>(null);
  const latest = useRef({ me, fit, setMe, onDone, rules: settings.statusRules, words, on: settings.statusRulesOn, inWorld: here !== null });
  latest.current = { me, fit, setMe, onDone, rules: settings.statusRules, words, on: settings.statusRulesOn, inWorld: here !== null };
  const wanted = fit ? `${fit.status}|${fit.text}` : "";

  useEffect(() => {
    if (!me) return;
    const act = async () => {
      const { me, fit, setMe, onDone, rules, words, on, inWorld } = latest.current;
      if (!me || working.current || handBusy) return;
      const target = fit?.id ?? "";
      const name = fit ? `Rule ${rules.findIndex((r) => r.id === fit.id) + 1}` : "";
      const now: Pair = { status: me.status, text: me.statusDescription };
      // Another rule, or none: what failed before is tried afresh.
      if (lastTarget.current !== target) {
        lastTarget.current = target;
        failed.current = { mark: "", at: 0, told: false };
        note(fit ? `${name} fits. The app sees you in ${words}.` : on ? `No rule fits. The app sees you in ${words}.` : "The rules are switched off.");
      }
      const before = held(me.id);
      let h = before;
      // A status that changed since the last look, and not by the rules or by the app's own status window: I did that, in VRChat or on the website.
      const was = seen.current;
      seen.current = { id: me.id, ...now };
      const changed = was !== null && was.id === me.id && !alike(was, now);
      const mine = (sent.current !== null && alike(sent.current, now)) || (byHand !== null && Date.now() - byHand.at < 60_000 && alike(byHand, now));
      // Seen once, a hand-set status has done its job here.
      if (byHand !== null && (alike(byHand, now) || Date.now() - byHand.at >= 60_000)) byHand = null;
      if (h?.pending && changed && !mine && !alike(now, h.set)) {
        // The rule's own change never got through, and meanwhile I picked a status myself.
        h = { ...h, own: null, paused: true, pending: false };
        keep(me.id, h);
        note(`Your status became ${show(now)} and the rules did not do that. The rules leave it alone until none fits any more.`);
      }
      if (h?.pending && alike(now, h.set)) {
        // The change did go through, though its answer never arrived.
        h = { ...h, pending: false, at: Date.now() };
        keep(me.id, h);
      }
      if (h && !h.paused && !h.pending && !alike(now, h.set)) {
        if (me.status === h.set.status && Date.now() - (h.at ?? 0) < ECHO) {
          // VRChat keeps the text a little differently from how it was sent.
          h = { ...h, set: { ...h.set, text: me.statusDescription } };
        } else {
          h = { ...h, own: null, paused: true };
          note(`Your status became ${show(now)} and the rules did not do that. The rule leaves it alone until it is over.`);
        }
        keep(me.id, h);
      } else if (!h && fit && changed && !mine) {
        // Changed by me just as a rule was about to take over: the rule stays out of it.
        h = { rule: target, own: null, set: now, paused: true };
        keep(me.id, h);
        note(`Your status became ${show(now)} and the rules did not do that. ${name} leaves it alone until it is over.`);
      }

      let next: Pair;
      let after: Held | null;
      let say: string;
      if (!fit) {
        if (!h) return;
        // Between two worlds no rule fits for a moment, and that is not an end. In a world
        // where no rule fits, it is one, and my status goes back without the long wait.
        if (on && !inWorld && Date.now() - noneSince.current < LEAVE) return;
        if (!h.own || h.paused || !(h.own.status in statusNames)) {
          note(h.paused ? "The rule is over. Your status stays as you set it." : "The rule is over. There was no status of yours to put back.");
          return keep(me.id, null);
        }
        next = h.own;
        after = null;
        say = `No status rule fits any more. Your status is back to ${statusNames[next.status as StatusValue]}.`;
      } else {
        if (h && h.paused) {
          // A rule I overrode keeps its hands off, and so does the next one right after it.
          if (h.rule !== target) keep(me.id, { ...h, rule: target });
          return;
        }
        const own = h && h.own ? h.own : now;
        if (!(own.status in statusNames)) {
          // Invisible, for one: a status the app could not put back afterwards.
          if (!hiddenTold.current) {
            onDone("Status rules wait while your status is one the app cannot put back, such as invisible.");
            note(`${name} fits, but your status is "${own.status}", which the app could not put back. Nothing changed.`);
          }
          hiddenTold.current = true;
          return;
        }
        hiddenTold.current = false;
        next = { status: fit.status, text: fit.text.trim() || own.text };
        if (h && h.rule === target && !h.pending && alike(h.set, next)) return;
        after = { rule: target, own, set: next, paused: false, at: Date.now() };
        say = `A status rule fits. Your status is now ${statusNames[fit.status]}.`;
      }
      if (alike(now, next)) return keep(me.id, after);
      const mark = `${target}|${next.status}|${next.text}`;
      if (failed.current.mark === mark && Date.now() - failed.current.at < AGAIN) return;
      working.current = true;
      lastCall.current = Date.now();
      // Written down first: if the answer gets lost, the status I had is still known.
      if (after) keep(me.id, { ...after, pending: true });
      try {
        sent.current = next;
        const call = backend.setStatus(next.status, next.text);
        ruleCall = call;
        await call.finally(() => {
          if (ruleCall === call) ruleCall = null;
        });
        failed.current = { mark: "", at: 0, told: false };
        // A status I set by hand while this was on its way wins: its record stays as it is.
        if (held(me.id)?.paused) {
          note("You set your status yourself while a rule was changing it. Yours stays.");
          return;
        }
        keep(me.id, after ? { ...after, at: Date.now() } : null);
        seen.current = { id: me.id, ...next };
        setMe((old) => (old && old.id === me.id ? { ...old, status: next.status, statusDescription: next.text } : old));
        note(fit ? `${name} set your status to ${show(next)}. Before it was ${show(now)}.` : `Put your status back to ${show(next)}.`);
        onDone(say);
      } catch (err) {
        const kind = err && typeof err === "object" && "kind" in err ? String((err as { kind: unknown }).kind) : "";
        const later = kind === "network" || kind === "rateLimited";
        if (!later) {
          // VRChat said no. Trying again would get the same answer, so the record goes back to what it was.
          keep(me.id, after ? before : null);
        }
        const told = failed.current.mark === mark && failed.current.told;
        failed.current = { mark, at: later ? Date.now() : Number.MAX_SAFE_INTEGER, told: true };
        note(`Setting ${show(next)} did not work: ${errorText(err)}${later ? ". The app tries again in two minutes." : ""}`);
        if (!told) onDone(later ? `The status change did not reach VRChat: ${errorText(err)}. The app tries again in two minutes.` : `The status rule did not go through: ${errorText(err)}`);
      } finally {
        working.current = false;
      }
    };
    const wait = Math.max(SETTLE, GAP - (Date.now() - lastCall.current));
    const id = window.setTimeout(() => void act(), wait);
    return () => window.clearTimeout(id);
  }, [me?.id, me?.status, me?.statusDescription, target, wanted, tick]); // eslint-disable-line react-hooks/exhaustive-deps
}
