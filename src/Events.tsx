import { useEffect, useMemo, useRef, useState } from "react";
import { backend, errorText, type CalendarEvent } from "./backend";
import { clock, dayLabel, tidy } from "./bits";
import { openThing } from "./Detail";
import { usePic } from "./Pic";
import { live } from "./settings";

type Which = "discover" | "mine" | "following" | "featured";

const lists: Array<[Which, string]> = [
  ["mine", "My groups"],
  ["discover", "All of VRChat"],
  ["following", "Following"],
  ["featured", "Featured"],
];

const none: Record<Which, string> = {
  discover: "VRChat lists no events that are on now or coming up.",
  mine: "None of your groups has an event this month.",
  following: "You follow no events this month. Press Follow on an event and it is listed here, with a reminder before it starts.",
  featured: "VRChat features no events this month.",
};

const accessWords: Record<string, string> = { public: "Open to everyone", group: "Group members" };

/** "Saturday, 28 November" for a day key such as "2026-11-28". */
function dayWords(key: string): string {
  const [y, m, d] = key.split("-").map(Number);
  return new Date(y, m - 1, d).toLocaleDateString([], { weekday: "long", day: "numeric", month: "long" });
}

/** How long an event runs, in the fewest words: "1 h 10 min". Empty when it has no end. */
function lengthWords(e: CalendarEvent): string {
  const minutes = Math.round((Date.parse(e.endsAt) - Date.parse(e.startsAt)) / 60_000);
  if (!(minutes > 0)) return "";
  const h = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return h ? (rest ? `${h} h ${rest} min` : `${h} h`) : `${rest} min`;
}

/** "2026-11" for a date, in the PC's own time. */
export function monthOf(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
}

function shifted(month: string, by: number): string {
  const [y, m] = month.split("-").map(Number);
  return monthOf(new Date(y, m - 1 + by, 1));
}

function monthName(month: string): string {
  const [y, m] = month.split("-").map(Number);
  return new Date(y, m - 1, 1).toLocaleDateString([], { month: "long", year: "numeric" });
}

/** How long until something starts, in the fewest words: "in 25 min", "in 3 h", "in 2 days". */
function untilWords(iso: string): string {
  const minutes = Math.round((Date.parse(iso) - Date.now()) / 60_000);
  if (minutes < 1) return "";
  if (minutes < 90) return `in ${minutes} min`;
  if (minutes < 36 * 60) return `in ${Math.round(minutes / 60)} h`;
  return `in ${Math.round(minutes / 1440)} days`;
}

/** The day a week starts on, as JavaScript counts days: 0 is Sunday. */
function firstWeekday(): number {
  const pick = live().weekStart;
  if (pick === "mon") return 1;
  if (pick === "sun") return 0;
  if (pick === "sat") return 6;
  try {
    const locale = new Intl.Locale(navigator.language) as Intl.Locale & { getWeekInfo?: () => { firstDay: number }; weekInfo?: { firstDay: number } };
    const first = (locale.getWeekInfo?.() ?? locale.weekInfo)?.firstDay;
    // Windows counts Monday as 1 and Sunday as 7.
    if (typeof first === "number") return first % 7;
  } catch {
    // An unknown language setting falls back to Monday.
  }
  return 1;
}

/** "2026-11-25" for a date, in the PC's own time. */
function dayKey(d: Date): string {
  return `${monthOf(d)}-${String(d.getDate()).padStart(2, "0")}`;
}

/** One month as a grid of days. A day with events carries their number; pressing it shows only that day. */
function MonthGrid({ month, counts, picked, pick }: { month: string; counts: Map<string, number>; picked: string; pick: (day: string) => void }) {
  const [y, m] = month.split("-").map(Number);
  const first = firstWeekday();
  const lead = (new Date(y, m - 1, 1).getDay() - first + 7) % 7;
  const days = new Date(y, m, 0).getDate();
  const today = dayKey(new Date());
  const names = Array.from({ length: 7 }, (_, n) => new Date(2024, 0, 7 + first + n).toLocaleDateString([], { weekday: "short" }));
  return (
    <div className="month-grid" role="grid" aria-label="Days of the month">
      {names.map((n) => (
        <span key={n} className="month-grid-head" role="columnheader">
          {n}
        </span>
      ))}
      {Array.from({ length: lead }, (_, n) => (
        <span key={`gap-${n}`} />
      ))}
      {Array.from({ length: days }, (_, n) => {
        const key = `${month}-${String(n + 1).padStart(2, "0")}`;
        const count = counts.get(key) ?? 0;
        return (
          <button
            key={key}
            role="gridcell"
            className={`${key === today ? "today" : ""}${key === picked ? " on" : ""}${count ? " has" : ""}`}
            disabled={!count}
            aria-pressed={key === picked}
            title={count ? `${count} ${count === 1 ? "event" : "events"}` : undefined}
            onClick={() => pick(key === picked ? "" : key)}
          >
            {n + 1}
            {count > 0 && <i>{count}</i>}
          </button>
        );
      })}
    </div>
  );
}

function EventPicture({ url }: { url: string }) {
  const shown = usePic(url);
  return <span className="thumb event-thumb" style={url ? { backgroundImage: `url("${shown}")` } : undefined} />;
}

/** Group events by month: the ones of my groups, the ones I follow, and VRChat's featured ones. */
export function Events({ onDone }: { onDone: (message: string) => void }) {
  const [which, setWhich] = useState<Which>("mine");
  // What is on across VRChat has no months: it is the ones on now, then the ones coming up.
  const byMonth = which !== "discover";
  const [month, setMonth] = useState(() => monthOf(new Date()));
  const [events, setEvents] = useState<CalendarEvent[] | null>(null);
  const [more, setMore] = useState(false);
  const [problem, setProblem] = useState("");
  const [past, setPast] = useState(false);
  const [busy, setBusy] = useState("");
  /** A day picked in the month grid ("2026-11-25"), or empty for the whole month. */
  const [onlyDay, setOnlyDay] = useState("");
  useEffect(() => setOnlyDay(""), [which, month, past]);
  /** Counts the questions, so a slow answer for another month is not shown over this one. */
  const asked = useRef(0);

  function load(fresh: boolean) {
    const mine = ++asked.current;
    setProblem("");
    if (!fresh) setEvents(null);
    // VRChat does not always say on a list whether I follow an event, so the list of the
    // ones I follow is read alongside (it is kept, so this is rarely a second question).
    const followed = which === "following" ? Promise.resolve(null) : backend.calendar("following", month, fresh).catch(() => null);
    void Promise.all([backend.calendar(which, byMonth ? month : "", fresh), followed])
      .then(([page, mineToo]) => {
        if (mine !== asked.current) return;
        const ids = new Set((mineToo?.events ?? []).map((e) => e.id));
        setEvents(page.events.map((e) => (which === "following" || ids.has(e.id) ? { ...e, following: true } : e)));
        setMore(page.more);
      })
      .catch((err) => {
        if (mine !== asked.current) return;
        setEvents([]);
        setProblem(errorText(err));
      });
  }

  useEffect(() => {
    load(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [which, month]);

  async function follow(e: CalendarEvent) {
    setBusy(e.id);
    try {
      await backend.followEvent(e.groupId, e.id, !e.following);
      setEvents((old) => (old ? old.map((x) => (x.id === e.id ? { ...x, following: !e.following } : x)).filter((x) => which !== "following" || x.following) : old));
      onDone(e.following ? `You no longer follow ${e.title}.` : `You follow ${e.title}. It is under Following, and Windows tells you before it starts.`);
      window.dispatchEvent(new Event("events:changed"));
    } catch (err) {
      onDone(`That did not work: ${errorText(err)}`);
    } finally {
      setBusy("");
    }
  }

  const shown = useMemo(() => {
    const now = Date.now();
    return (events ?? [])
      // VRChat also sends the days around a month; an event belongs to the month it starts in.
      .filter((e) => !byMonth || monthOf(new Date(e.startsAt)) === month)
      .filter((e) => !byMonth || past || !!onlyDay || Date.parse(e.endsAt || e.startsAt) >= now)
      .filter((e) => !byMonth || !onlyDay || dayKey(new Date(e.startsAt)) === onlyDay)
      .sort((a, b) => a.startsAt.localeCompare(b.startsAt));
  }, [events, past, month, byMonth, onlyDay]);
  const perDay = useMemo(() => {
    const counts = new Map<string, number>();
    for (const e of events ?? []) {
      const d = new Date(e.startsAt);
      if (monthOf(d) === month) counts.set(dayKey(d), (counts.get(dayKey(d)) ?? 0) + 1);
    }
    return counts;
  }, [events, month]);
  const inMonth = byMonth ? (events ?? []).filter((e) => monthOf(new Date(e.startsAt)) === month) : [];
  const overCount = inMonth.filter((e) => Date.parse(e.endsAt || e.startsAt) < Date.now()).length;
  const hidden = byMonth && !onlyDay ? inMonth.length - shown.length : 0;

  let lastDay = "";
  const heading = !byMonth
    ? "On now and coming up"
    : onlyDay
      ? dayWords(onlyDay)
      : past
        ? `All of ${monthName(month)}`
        : month === monthOf(new Date())
          ? "Coming up this month"
          : monthName(month);
  return (
    <div className="table-wrap events-screen">
      <div className="tabs" role="tablist" aria-label="Events">
        {lists.map(([id, label]) => (
          <button key={id} role="tab" aria-selected={which === id} className={which === id ? "on" : ""} onClick={() => setWhich(id)}>
            {label}
          </button>
        ))}
      </div>

      <div className={`events-layout${byMonth ? "" : " events-wide"}`}>
        {byMonth && (
          <aside className="events-side" aria-label="Month">
            <div className="month-nav">
              <button className="plain" aria-label="The month before" title="The month before" onClick={() => setMonth(shifted(month, -1))}>
                ‹
              </button>
              <b>{monthName(month)}</b>
              <button className="plain" aria-label="The month after" title="The month after" onClick={() => setMonth(shifted(month, 1))}>
                ›
              </button>
            </div>
            <MonthGrid month={month} counts={perDay} picked={onlyDay} pick={setOnlyDay} />
            {events !== null && (
              <p className="events-count">
                {inMonth.length === 0
                  ? "No events this month."
                  : `${inMonth.length} ${inMonth.length === 1 ? "event" : "events"} this month${overCount > 0 ? `, ${overCount} already over` : ""}. Press a day to see only that day.`}
              </p>
            )}
            <label className="setting events-past">
              <span>
                <span className="setting-name">Show the ones that are over</span>
              </span>
              <input type="checkbox" role="switch" checked={past} onChange={(e) => setPast(e.target.checked)} />
            </label>
            <div className="events-side-actions">
              {month !== monthOf(new Date()) && (
                <button className="plain" onClick={() => setMonth(monthOf(new Date()))}>
                  This month
                </button>
              )}
              <button className="plain" title="Asks VRChat again now. Otherwise a list is asked for at most once in ten minutes" onClick={() => load(true)}>
                Refresh
              </button>
            </div>
          </aside>
        )}

        <section className="events-main" aria-label={heading}>
          <header className="events-head">
            <h2>{heading}</h2>
            {events !== null && shown.length > 0 && (
              <span className="events-tally">
                {shown.length} {shown.length === 1 ? "event" : "events"}
              </span>
            )}
            {onlyDay && (
              <button className="plain" onClick={() => setOnlyDay("")}>
                Show the whole month
              </button>
            )}
            {!byMonth && (
              <button className="plain" title="Asks VRChat again now. Otherwise a list is asked for at most once in ten minutes" onClick={() => load(true)}>
                Refresh
              </button>
            )}
          </header>

          {problem && (
            <p className="banner" role="alert">
              VRChat did not send the events: {problem}
            </p>
          )}
          {events === null && <p className="empty">{byMonth ? `Asking VRChat for the events of ${monthName(month)}` : "Asking VRChat what is on"}</p>}
          {events !== null && !problem && shown.length === 0 && (
            <div className="events-none">
              <p>{hidden > 0 ? `All ${hidden} ${hidden === 1 ? "event" : "events"} of this month are over.` : none[which]}</p>
              {hidden > 0 && (
                <button className="plain go" onClick={() => setPast(true)}>
                  Show them
                </button>
              )}
            </div>
          )}

          <ol className="events">
            {shown.map((e) => {
              const day = dayLabel(e.startsAt);
              const header =
                !onlyDay && day !== lastDay ? (
                  <li className="day" key={`day-${e.id}`}>
                    {day}
                  </li>
                ) : null;
              lastDay = day;
              const now = Date.now();
              const live = Date.parse(e.startsAt) <= now && now < Date.parse(e.endsAt || e.startsAt);
              const over = Date.parse(e.endsAt || e.startsAt) < now;
              const soon = untilWords(e.startsAt);
              const length = lengthWords(e);
              return [
                header,
                <li className={`event${over ? " event-over" : ""}${live ? " event-live" : ""}`} key={e.id}>
                  <EventPicture url={e.image} />
                  <div className="event-main">
                    <p className="event-when">
                      <time dateTime={e.startsAt}>{clock(e.startsAt)}</time>
                      {e.endsAt && <span>to {clock(e.endsAt)}</span>}
                      {length && <span className="event-length">{length}</span>}
                      {live && <span className="mark mark-live">On now</span>}
                      {over && <span className="mark">Over</span>}
                      {!live && !over && soon && Date.parse(e.startsAt) - now < 36 * 3_600_000 && <span className="mark">{soon}</span>}
                    </p>
                    <h3 className="event-title">{tidy(e.title) || "Unnamed event"}</h3>
                    <p className="event-sub">
                      <button className="name-link" onClick={() => openThing("group", e.groupId)} title="Open the group">
                        {e.groupName || "A group"}
                      </button>
                      {e.category && <span className="event-chip">{e.category}</span>}
                      <span className="event-chip">{accessWords[e.access] ?? e.access}</span>
                      {e.interested > 0 && (
                        <span>
                          {e.interested.toLocaleString()} interested{e.friends > 0 ? `, ${e.friends} of them ${e.friends === 1 ? "a friend" : "friends"}` : ""}
                        </span>
                      )}
                    </p>
                    {e.description && <p className="event-about">{e.description}</p>}
                  </div>
                  <div className="event-actions">
                    <button
                      className={e.following ? "plain" : "plain go"}
                      disabled={busy === e.id}
                      title={e.following ? "Stop following this event" : "Follow it: it is listed under Following and you get a reminder before it starts"}
                      onClick={() => void follow(e)}
                    >
                      {e.following ? "Unfollow" : "Follow"}
                    </button>
                  </div>
                </li>,
              ];
            })}
          </ol>
          {more && <p className="tile-note">{byMonth ? "VRChat has more than 100 events for this month; the first 100 are shown." : "VRChat has more coming up; the next 100 are shown."}</p>}
        </section>
      </div>
    </div>
  );
}
