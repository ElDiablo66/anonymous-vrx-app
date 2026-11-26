import { useEffect, useMemo, useRef, useState } from "react";
import { backend, errorText, type CalendarEvent } from "./backend";
import { clock, dayLabel, tidy } from "./bits";
import { openThing } from "./Detail";
import { usePic } from "./Pic";

type Which = "mine" | "following" | "featured";

const lists: Array<[Which, string]> = [
  ["mine", "My groups"],
  ["following", "Following"],
  ["featured", "Featured"],
];

const none: Record<Which, string> = {
  mine: "None of your groups has an event this month.",
  following: "You follow no events this month. Press Follow on an event and it is listed here, with a reminder before it starts.",
  featured: "VRChat features no events this month.",
};

const accessWords: Record<string, string> = { public: "Open to everyone", group: "Group members" };

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

function EventPicture({ url }: { url: string }) {
  const shown = usePic(url);
  return <span className="thumb event-thumb" style={url ? { backgroundImage: `url("${shown}")` } : undefined} />;
}

/** Group events by month: the ones of my groups, the ones I follow, and VRChat's featured ones. */
export function Events({ onDone }: { onDone: (message: string) => void }) {
  const [which, setWhich] = useState<Which>("mine");
  const [month, setMonth] = useState(() => monthOf(new Date()));
  const [events, setEvents] = useState<CalendarEvent[] | null>(null);
  const [more, setMore] = useState(false);
  const [problem, setProblem] = useState("");
  const [past, setPast] = useState(false);
  const [busy, setBusy] = useState("");
  /** Counts the questions, so a slow answer for another month is not shown over this one. */
  const asked = useRef(0);

  function load(fresh: boolean) {
    const mine = ++asked.current;
    setProblem("");
    if (!fresh) setEvents(null);
    void backend
      .calendar(which, month, fresh)
      .then((page) => {
        if (mine !== asked.current) return;
        setEvents(page.events);
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
      .filter((e) => past || Date.parse(e.endsAt || e.startsAt) >= now)
      .sort((a, b) => a.startsAt.localeCompare(b.startsAt));
  }, [events, past]);
  const hidden = (events?.length ?? 0) - shown.length;

  let lastDay = "";
  return (
    <div className="table-wrap">
      <div className="tabs" role="tablist" aria-label="Events">
        {lists.map(([id, label]) => (
          <button key={id} role="tab" aria-selected={which === id} className={which === id ? "on" : ""} onClick={() => setWhich(id)}>
            {label}
          </button>
        ))}
      </div>
      <div className="list-tools event-tools">
        <span className="month-nav">
          <button className="plain" aria-label="The month before" title="The month before" onClick={() => setMonth(shifted(month, -1))}>
            ‹
          </button>
          <b>{monthName(month)}</b>
          <button className="plain" aria-label="The month after" title="The month after" onClick={() => setMonth(shifted(month, 1))}>
            ›
          </button>
        </span>
        {month !== monthOf(new Date()) && (
          <button className="plain" onClick={() => setMonth(monthOf(new Date()))}>
            This month
          </button>
        )}
        <label className="tool">
          <input type="checkbox" checked={past} onChange={(e) => setPast(e.target.checked)} />
          Show the ones that are over
        </label>
        <button className="plain" title="Asks VRChat again now. Otherwise a list is asked for at most once in ten minutes" onClick={() => load(true)}>
          Refresh
        </button>
      </div>

      {problem && (
        <p className="banner" role="alert">
          VRChat did not send the events: {problem}
        </p>
      )}
      {events === null && <p className="empty">Asking VRChat for the events of {monthName(month)}</p>}
      {events !== null && !problem && shown.length === 0 && (
        <p className="empty">{hidden > 0 ? `All ${hidden} ${hidden === 1 ? "event" : "events"} of this month are over. Tick the box above to see them.` : none[which]}</p>
      )}

      <ol className="events">
        {shown.map((e) => {
          const day = dayLabel(e.startsAt);
          const header = day !== lastDay ? <li className="day" key={`day-${e.id}`}>{day}</li> : null;
          lastDay = day;
          const now = Date.now();
          const live = Date.parse(e.startsAt) <= now && now < Date.parse(e.endsAt || e.startsAt);
          const over = Date.parse(e.endsAt || e.startsAt) < now;
          const soon = untilWords(e.startsAt);
          return [
            header,
            <li className={`event${over ? " event-over" : ""}`} key={e.id}>
              <span className="event-time">
                <time dateTime={e.startsAt}>{clock(e.startsAt)}</time>
                {e.endsAt && <span>to {clock(e.endsAt)}</span>}
              </span>
              <EventPicture url={e.image} />
              <span className="event-main">
                <span className="event-title">
                  {tidy(e.title) || "Unnamed event"}
                  {live && <span className="mark mark-live">On now</span>}
                  {!live && soon && Date.parse(e.startsAt) - now < 36 * 3_600_000 && <span className="mark">{soon}</span>}
                </span>
                <span className="event-sub">
                  <button className="name-link" onClick={() => openThing("group", e.groupId)} title="Open the group">
                    {e.groupName || "A group"}
                  </button>
                  {e.category && <span>{e.category}</span>}
                  <span>{accessWords[e.access] ?? e.access}</span>
                  {e.interested > 0 && (
                    <span>
                      {e.interested.toLocaleString()} interested{e.friends > 0 ? `, ${e.friends} of them ${e.friends === 1 ? "a friend" : "friends"}` : ""}
                    </span>
                  )}
                </span>
                {e.description && <span className="event-about">{e.description}</span>}
              </span>
              <span className="event-actions">
                <button
                  className={e.following ? "plain" : "plain go"}
                  disabled={busy === e.id}
                  title={e.following ? "Stop following this event" : "Follow it: it is listed under Following and you get a reminder before it starts"}
                  onClick={() => void follow(e)}
                >
                  {e.following ? "Following" : "Follow"}
                </button>
              </span>
            </li>,
          ];
        })}
      </ol>
      {more && <p className="tile-note">VRChat has more than 100 events for this month; the first 100 are shown.</p>}
    </div>
  );
}
