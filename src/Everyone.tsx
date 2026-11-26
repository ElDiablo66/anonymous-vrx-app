import { useEffect, useMemo, useRef, useState } from "react";
import { backend, errorText, type Favorites, type FeedItem, type Friend, type Moderation, type World } from "./backend";
import { Avatar, accessName, clock, dayLabel, daysAgo, deviceLine, statusTone, statusWords, tidy } from "./bits";
import { openProfile } from "./Profile";

const rank = { online: 0, active: 1, offline: 2 } as const;

export function whereText(f: Friend, worlds: Map<string, World>): string {
  if (f.state !== "online") return "";
  if (f.place.kind === "private") return "Private world";
  if (f.place.kind === "traveling") return "On the way somewhere";
  if (f.place.kind === "world" && f.place.worldId) {
    const name = tidy(worlds.get(f.place.worldId)?.name ?? "A world");
    return `${name}, ${accessName(f.place)}`;
  }
  return "";
}

function logLine(item: FeedItem): string {
  if (item.kind === "friendAdded") return "became your friend";
  if (item.kind === "friendRemoved") return "is no longer on your friend list";
  return `changed name from ${item.previous}`;
}

function FriendLog({ friends, tick }: { friends: Map<string, Friend>; tick: number }) {
  const [log, setLog] = useState<FeedItem[] | null>(null);

  useEffect(() => {
    let alive = true;
    void backend
      .friendLog(500)
      .then((list) => alive && setLog(list))
      .catch(() => alive && setLog([]));
    return () => {
      alive = false;
    };
  }, [tick]);

  if (log === null) return <p className="empty">Reading the friend log</p>;
  if (log.length === 0) {
    return (
      <p className="empty">
        Nothing yet. From now on this records every friend added, every friend lost and every name change, including
        ones that happen while the app is closed.
      </p>
    );
  }
  let lastDay = "";
  return (
    <ol className="inbox">
      {log.map((item) => {
        const day = dayLabel(item.at);
        const header = day !== lastDay ? <li className="day" key={`day-${item.id}`}>{day}</li> : null;
        lastDay = day;
        return [
          header,
          <li className={`notice notice-done log-${item.kind}`} key={item.id}>
            <Avatar name={item.displayName} image={friends.get(item.userId)?.image ?? ""} />
            <div className="notice-text">
              <p className="notice-title">
                <button className="name-link" onClick={() => openProfile(item.userId)}>
                  {item.displayName}
                </button>{" "}
                {logLine(item)}
              </p>
              <time dateTime={item.at}>{clock(item.at)}</time>
            </div>
            <span className={`outcome outcome-${item.kind === "friendAdded" ? "yes" : item.kind === "friendRemoved" ? "no" : "plain"}`}>
              {item.kind === "friendAdded" ? "Added" : item.kind === "friendRemoved" ? "Removed" : "Renamed"}
            </span>
          </li>,
        ];
      })}
    </ol>
  );
}

/** What was looked up about people on the blocked list, so a row is asked for once. */
const briefs = new Map<string, Friend>();
// Another account signed in: what was looked up belongs to the one before.
window.addEventListener("account:changed", () => briefs.clear());

/** One blocked or muted person. Their picture, rank and status are looked up when the row
 *  has been on screen for a moment, so a long list does not ask for everyone at once. */
function BlockedRow({ m, friend, busy, onUndo }: { m: Moderation; friend?: Friend; busy: boolean; onUndo: () => void }) {
  const [brief, setBrief] = useState<Friend | undefined>(friend ?? briefs.get(m.userId));
  const row = useRef<HTMLLIElement>(null);

  useEffect(() => {
    if (brief || !row.current) return;
    let alive = true;
    let wait = 0;
    const watch = new IntersectionObserver((seen) => {
      window.clearTimeout(wait);
      if (!seen.some((s) => s.isIntersecting)) return;
      wait = window.setTimeout(() => {
        watch.disconnect();
        void backend
          .personBrief(m.userId)
          .then((b) => {
            briefs.set(m.userId, b);
            if (alive) setBrief(b);
          })
          .catch(() => {});
      }, 400);
    });
    watch.observe(row.current);
    return () => {
      alive = false;
      window.clearTimeout(wait);
      watch.disconnect();
    };
  }, [brief, m.userId]);

  const name = brief?.displayName || m.name || "Someone";
  return (
    <li className="notice notice-done" ref={row}>
      <Avatar name={name} image={brief?.image ?? ""} tone={brief ? statusTone(brief) : undefined} />
      <div className="notice-text">
        <p className="notice-title">
          <button className={`name-link${brief?.rank ? ` rank-name rank-${brief.rank.replace(/ /g, "")}` : ""}`} title={brief?.rank || undefined} onClick={() => openProfile(m.userId)}>
            {name}
          </button>
          {brief?.vrcPlus && <span className="mark" title="VRChat+ supporter">VRC+</span>}
          {brief?.adult && <span className="mark" title="Age verified as 18+">18+</span>}
        </p>
        <time dateTime={m.at || undefined}>
          {brief ? (brief.state === "active" ? deviceLine(brief.platform) : statusWords(brief)) : ""}
          {brief?.statusDescription ? <span className="who">, {brief.statusDescription}</span> : null}
          {brief && m.at ? " · " : ""}
          {m.at ? `${m.kind === "block" ? "Blocked" : "Muted"} since ${dayLabel(m.at)}` : ""}
        </time>
      </div>
      <button className="plain" disabled={busy} onClick={onUndo}>
        {m.kind === "block" ? "Unblock" : "Unmute"}
      </button>
    </li>
  );
}

/** Everyone I blocked or muted, with a way back for each. */
function Blocked({ friends, onDone }: { friends: Map<string, Friend>; onDone: (message: string) => void }) {
  const [rows, setRows] = useState<Moderation[] | null>(null);
  const [problem, setProblem] = useState("");
  const [busy, setBusy] = useState("");
  const [only, setOnly] = useState<"all" | "block" | "mute">("all");

  useEffect(() => {
    let alive = true;
    void backend
      .moderations(true)
      .then((list) => alive && setRows(list))
      .catch((err) => alive && setProblem(errorText(err)));
    // A block or mute changed from a profile: read the list again.
    const again = (e: Event) => {
      const what = (e as CustomEvent<{ userId: string; kind: string; on: boolean }>).detail;
      // Something taken back leaves the list at once, without waiting for VRChat to agree.
      if (what && !what.on) {
        setRows((old) => (old ? old.filter((r) => !(r.userId === what.userId && r.kind === what.kind)) : old));
        return;
      }
      void backend
        .moderations(false)
        .then((list) => alive && setRows(list))
        .catch(() => {});
    };
    window.addEventListener("moderation:changed", again);
    return () => {
      alive = false;
      window.removeEventListener("moderation:changed", again);
    };
  }, []);

  async function undo(m: Moderation) {
    const id = m.kind + m.userId;
    setBusy(id);
    try {
      await backend.setModeration(m.userId, m.kind, false);
      setRows((old) => (old ? old.filter((r) => r.kind + r.userId !== id) : old));
      // A profile of theirs may be open on top of this list.
      window.dispatchEvent(new CustomEvent("moderation:changed", { detail: { userId: m.userId, kind: m.kind, on: false } }));
      onDone(`${m.name} is no longer ${m.kind === "block" ? "blocked" : "muted"}.`);
    } catch (err) {
      onDone(`That did not work: ${errorText(err)}`);
    } finally {
      setBusy("");
    }
  }

  if (problem) {
    return (
      <p className="banner" role="alert">
        {problem}
      </p>
    );
  }
  if (rows === null) return <p className="empty">Asking VRChat who you blocked and muted</p>;
  if (rows.length === 0) return <p className="empty">Nobody is blocked or muted. You can block or mute someone from their profile.</p>;
  const count = { block: rows.filter((r) => r.kind === "block").length, mute: rows.filter((r) => r.kind === "mute").length };
  // The two kinds get a filter each when there is something of both.
  const active = only !== "all" && count[only] === 0 ? "all" : only;
  return (
    <>
      {count.block > 0 && count.mute > 0 && (
        <div className="filters filters-flat" role="group" aria-label="Blocked or muted">
          {(
            [
              ["all", "Everything"],
              ["block", "Blocked"],
              ["mute", "Muted"],
            ] as const
          ).map(([id, label]) => (
            <button key={id} className={id === active ? "on" : ""} aria-pressed={id === active} onClick={() => setOnly(id)}>
              {label}
              {id !== "all" && <span className="chip-count">{count[id]}</span>}
            </button>
          ))}
        </div>
      )}
      {(["block", "mute"] as const).map((kind) => {
        const mine = rows.filter((r) => r.kind === kind);
        if (mine.length === 0 || (active !== "all" && active !== kind)) return null;
        return (
          <section key={kind}>
            <h2 className="blocked-title">
              {kind === "block" ? "Blocked" : "Muted"}, {mine.length}
            </h2>
            <ol className="inbox">
              {mine.map((m) => (
                <BlockedRow key={m.kind + m.userId} m={m} friend={friends.get(m.userId)} busy={busy === m.kind + m.userId} onUndo={() => void undo(m)} />
              ))}
            </ol>
          </section>
        );
      })}
    </>
  );
}

export function Everyone({
  friends,
  friendMap,
  worlds,
  favorites,
  noted,
  feedTick,
  query,
  onDone,
}: {
  onDone: (message: string) => void;
  friends: Friend[];
  friendMap: Map<string, Friend>;
  worlds: Map<string, World>;
  favorites: Favorites;
  /** User ids with a note of mine. */
  noted: Set<string>;
  feedTick: number;
  query: string;
}) {
  const [tab, setTab] = useState<"list" | "log" | "blocked">("list");
  const [sort, setSort] = useState<"status" | "name" | "place" | "together" | "quiet">("status");
  const [lastSeen, setLastSeen] = useState<Record<string, string>>({});
  const [hidePrivate, setHidePrivate] = useState(false);
  const [together, setTogether] = useState<Record<string, number>>({});
  const q = query.trim().toLowerCase();

  useEffect(() => {
    let alive = true;
    void backend
      .togetherCounts()
      .then((counts) => alive && setTogether(counts))
      .catch(() => {});
    void backend
      .lastSeen()
      .then((seen) => alive && setLastSeen(seen))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [feedTick]);

  const rows = useMemo(() => {
    const byStatus = (a: Friend, b: Friend) =>
      rank[a.state] - rank[b.state] ||
      Number(b.id in favorites.friends) - Number(a.id in favorites.friends) ||
      a.displayName.localeCompare(b.displayName);
    // Friends in public-ish worlds first, grouped by world; private and offline after.
    const placeKey = (f: Friend) => (f.state === "online" && f.place.kind === "world" ? whereText(f, worlds) : "~");
    const order: Record<typeof sort, (a: Friend, b: Friend) => number> = {
      status: byStatus,
      name: (a, b) => a.displayName.localeCompare(b.displayName),
      place: (a, b) => placeKey(a).localeCompare(placeKey(b)) || byStatus(a, b),
      together: (a, b) => (together[b.id] ?? 0) - (together[a.id] ?? 0) || byStatus(a, b),
      // Longest quiet first: people never seen lead, then the oldest sighting. Whoever is on now goes last.
      quiet: (a, b) => quietKey(a).localeCompare(quietKey(b)) || a.displayName.localeCompare(b.displayName),
    };
    function quietKey(f: Friend): string {
      if (f.state !== "offline") return "9";
      return lastSeen[f.id] ?? "0";
    }
    return friends
      .filter((f) => !hidePrivate || !(f.state === "online" && f.place.kind === "private"))
      .filter((f) => !q || f.displayName.toLowerCase().includes(q) || whereText(f, worlds).toLowerCase().includes(q))
      .sort(order[sort]);
  }, [friends, worlds, favorites, q, sort, hidePrivate, together, lastSeen]);

  return (
    <div className="table-wrap">
      <div className="tabs" role="tablist" aria-label="Everyone">
        <button role="tab" aria-selected={tab === "list"} className={tab === "list" ? "on" : ""} onClick={() => setTab("list")}>
          Friend list
        </button>
        <button role="tab" aria-selected={tab === "log"} className={tab === "log" ? "on" : ""} onClick={() => setTab("log")}>
          Friend log
        </button>
        <button role="tab" aria-selected={tab === "blocked"} className={tab === "blocked" ? "on" : ""} onClick={() => setTab("blocked")}>
          Blocked and muted
        </button>
      </div>
      {tab === "list" && (
        <div className="list-tools">
          <label className="tool">
            Sort by
            <select value={sort} onChange={(e) => setSort(e.target.value as typeof sort)}>
              <option value="status">Online first</option>
              <option value="place">World</option>
              <option value="name">Name</option>
              <option value="together">Times together</option>
              <option value="quiet">Longest quiet first</option>
            </select>
          </label>
          <label className="tool">
            <input type="checkbox" checked={hidePrivate} onChange={(e) => setHidePrivate(e.target.checked)} />
            Hide friends in private worlds
          </label>
        </div>
      )}
      {tab === "blocked" ? (
        <Blocked friends={friendMap} onDone={onDone} />
      ) : tab === "log" ? (
        <FriendLog friends={friendMap} tick={feedTick} />
      ) : rows.length === 0 ? (
        <p className="empty">{q ? `Nobody matches "${query.trim()}".` : "No friends to show yet."}</p>
      ) : (
        <>
          <table className="people">
            <thead>
              <tr>
                <th>Friend</th>
                <th>Status</th>
                <th>Where</th>
                <th title="Visits you shared, from VRChat's log files on this PC">Together</th>
                <th title="When this app last saw them online or in a world with you. It only knows what happened while it was recording">Last seen</th>
              </tr>
            </thead>
            <tbody>
              {rows.slice(0, 600).map((f) => (
                <tr key={f.id} data-person={f.id} data-person-name={f.displayName} className={f.state === "offline" ? "dim" : ""} onClick={() => openProfile(f.id)} title="Open profile. Right-click for invites and a boop">
                  <td>
                    <span className="person">
                      <Avatar name={f.displayName} image={f.image} tone={statusTone(f)} />
                      <span className={f.rank ? `rank-name rank-${f.rank.replace(/ /g, "")}` : ""} title={f.rank || undefined}>
                        {f.displayName}
                      </span>
                      {f.vrcPlus && <span className="mark" title="VRChat+ supporter">VRC+</span>}
                      {f.adult && <span className="mark" title="Age verified as 18+">18+</span>}
                      {f.id in favorites.friends && <span className="star" title="Favourite">★</span>}
                      {noted.has(f.id) && <span className="has-note" title="You wrote a note about them">note</span>}
                    </span>
                  </td>
                  <td>
                    <span className={`status-word tone-${statusTone(f)}`}>
                      <i className="tone-dot" />
                      {f.state === "active" ? deviceLine(f.platform) : statusWords(f)}
                    </span>
                    {f.statusDescription && <span className="note">{f.statusDescription}</span>}
                  </td>
                  <td>{whereText(f, worlds)}</td>
                  <td className="num">{together[f.id] ? together[f.id] : ""}</td>
                  <td>{f.state !== "offline" ? "now" : lastSeen[f.id] ? daysAgo(lastSeen[f.id]) : "not on record"}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {rows.length > 600 && <p className="empty">Showing the first 600. Search to narrow it down.</p>}
        </>
      )}
    </div>
  );
}
