import { openProfile } from "./Profile";
import { useState, type ReactNode } from "react";
import type { FeedItem, Friend, World, FriendDay } from "./backend";
import { Avatar, clock, dayLabel, tidy } from "./bits";

function placeName(item: FeedItem, worlds: Map<string, World>): string {
  if (item.location === "private") return "a private world";
  if (item.worldId) return tidy(worlds.get(item.worldId)?.name ?? "a world");
  return "";
}

const statusKey = (status: string) => ({ "join me": "join", active: "online", "ask me": "ask", busy: "busy" })[status] ?? "offline";

export function sentence(item: FeedItem, worlds: Map<string, World>): ReactNode {
  const who = <b className="who">{item.displayName}</b>;
  const place = placeName(item, worlds);
  switch (item.kind) {
    case "online":
      return <>{who} came online{place && <> in <i>{place}</i></>}</>;
    case "offline":
      return <>{who} left VRChat{place && <>, last seen in <i>{place}</i></>}</>;
    case "location":
      return <>{who} went to <i>{place || "another world"}</i></>;
    case "status":
      return (
        <>
          {who} switched from <i className={`tone-dot tone-${statusKey(item.previous)}`} /> {item.previous} to{" "}
          <i className={`tone-dot tone-${statusKey(item.detail)}`} /> {item.detail}
        </>
      );
    case "statusText":
      return item.detail ? <>{who} wrote <span className="who">"{item.detail}"</span></> : <>{who} cleared their status text</>;
    case "name":
      return <><b className="who">{item.previous}</b> is now called {who}</>;
    case "avatar":
      return <>{who} changed avatar</>;
    case "friendAdded":
      return <>{who} is now your friend</>;
    case "friendRemoved":
      return <>{who} is no longer on your friend list</>;
  }
}

/** "You and Mossy became friends a year ago today", or when that day comes. */
function friendDayLine(d: FriendDay) {
  const years = d.years === 1 ? "a year" : `${d.years} years`;
  const who = <b className="who">{d.name}</b>;
  if (d.inDays === 0) {
    return (
      <>
        You and {who} became friends {years} ago today.
      </>
    );
  }
  const day = new Date(`${d.on}T12:00:00`);
  // Past six days a weekday alone would read as today's, so the date comes with it.
  const when = d.inDays === 1 ? "Tomorrow" : d.inDays < 7 ? `On ${day.toLocaleDateString([], { weekday: "long" })}` : `On ${day.toLocaleDateString([], { weekday: "long", day: "numeric", month: "long" })}`;
  return (
    <>
      {when} it is {years} since you and {who} became friends.
    </>
  );
}

export const filters: Array<{ id: string; label: string; kinds: FeedItem["kind"][] | null }> = [
  { id: "all", label: "Everything", kinds: null },
  { id: "presence", label: "Online and offline", kinds: ["online", "offline"] },
  { id: "moves", label: "World changes", kinds: ["location"] },
  { id: "status", label: "Status", kinds: ["status", "statusText", "name", "avatar"] },
  { id: "friends", label: "Friend list", kinds: ["friendAdded", "friendRemoved"] },
];

interface Row {
  item: FeedItem;
  /** Older moves by the same friend that came straight before this one. */
  earlier: FeedItem[];
}

/** Folds a run of world changes by one friend into the newest of them. */
export function foldMoves(feed: FeedItem[], fold: boolean): Row[] {
  const rows: Row[] = [];
  for (const item of feed) {
    const last = rows[rows.length - 1];
    if (fold && last && item.kind === "location" && last.item.kind === "location" && last.item.userId === item.userId) {
      last.earlier.push(item);
    } else {
      rows.push({ item, earlier: [] });
    }
  }
  return rows;
}

export function Activity({
  feed: everything,
  worlds,
  friends,
  hidden,
  foldRepeats,
  onHide,
  open: panelOpen,
  onPutAway,
  days = [],
}: {
  /** Friends whose friendship day is today or in the next few days. */
  days?: FriendDay[];
  /** False while the panel is put away. */
  open: boolean;
  onPutAway: () => void;
  feed: FeedItem[];
  worlds: Map<string, World>;
  friends: Map<string, Friend>;
  /** Friends left out of the feed: user id to name. */
  hidden: Record<string, string>;
  foldRepeats: boolean;
  onHide: (userId: string, name: string) => void;
}) {
  const [filter, setFilter] = useState("all");
  const [open, setOpen] = useState<Set<number>>(new Set());
  const kinds = filters.find((f) => f.id === filter)?.kinds;
  const shown = everything.filter((item) => !(item.userId in hidden) && (!kinds || kinds.includes(item.kind)));
  const rows = foldMoves(shown, foldRepeats);
  const hiddenCount = Object.keys(hidden).length;
  let lastDay = "";

  const toggle = (id: number) =>
    setOpen((old) => {
      const next = new Set(old);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  return (
    <aside className="activity" aria-label="Activity" hidden={!panelOpen}>
      <div className="activity-head">
        <h2>Activity</h2>
        <button className="activity-away" onClick={onPutAway} title="Put the Activity panel away. A button at the top right brings it back">
          Hide
        </button>
      </div>
      <div className="filters" role="group" aria-label="Show in activity">
        {filters.map((f) => (
          <button key={f.id} className={f.id === filter ? "on" : ""} aria-pressed={f.id === filter} onClick={() => setFilter(f.id)}>
            {f.label}
          </button>
        ))}
      </div>
      {(filter === "all" || filter === "friends") && days.some((d) => !(d.userId in hidden)) && (
        <section className="friend-days" aria-label="Friendship days">
          {days
            .filter((d) => !(d.userId in hidden))
            .map((d) => (
              <button key={d.userId} className={`friend-day${d.inDays === 0 ? " today" : ""}`} onClick={() => openProfile(d.userId)} title="Open their profile">
                <Avatar name={d.name} image={d.image} size={36} />
                <span>{friendDayLine(d)}</span>
              </button>
            ))}
        </section>
      )}
      {hiddenCount > 0 && (
        <p className="feed-note">
          {hiddenCount === 1 ? "1 friend is" : `${hiddenCount} friends are`} left out. Bring them back in Settings.
        </p>
      )}
      {rows.length === 0 ? (
        <p className="empty">
          {everything.length === 0
            ? "Nothing yet. Changes show up here as friends come online, move between worlds or update their status, and they stay saved on this PC."
            : "Nothing of this kind yet."}
        </p>
      ) : (
        <ol>
          {rows.map(({ item, earlier }) => {
            const day = dayLabel(item.at);
            const header = day !== lastDay ? <li className="day" key={`day-${item.id}`}>{day}</li> : null;
            lastDay = day;
            const expanded = open.has(item.id);
            return [
              header,
              <li className={`entry entry-${item.kind}`} key={item.id}>
                <time dateTime={item.at}>{clock(item.at)}</time>
                <Avatar name={item.displayName} image={friends.get(item.userId)?.image ?? ""} size={28} />
                <div className="entry-body">
                  <p>{sentence(item, worlds)}</p>
                  {earlier.length > 0 && (
                    <button className="entry-more" aria-expanded={expanded} onClick={() => toggle(item.id)}>
                      {expanded ? "Hide" : "Show"} {earlier.length} earlier {earlier.length === 1 ? "move" : "moves"}
                    </button>
                  )}
                  {expanded && (
                    <ul className="entry-earlier">
                      {earlier.map((old) => (
                        <li key={old.id}>
                          <time dateTime={old.at}>{clock(old.at)}</time> {placeName(old, worlds) || "another world"}
                        </li>
                      ))}
                    </ul>
                  )}
                </div>
                <button
                  className="entry-hide"
                  title={`Leave ${item.displayName} out of the activity feed. Nothing is deleted; bring them back in Settings.`}
                  aria-label={`Leave ${item.displayName} out of the activity feed`}
                  onClick={() => onHide(item.userId, item.displayName)}
                >
                  Hide
                </button>
              </li>,
            ];
          })}
        </ol>
      )}
    </aside>
  );
}
