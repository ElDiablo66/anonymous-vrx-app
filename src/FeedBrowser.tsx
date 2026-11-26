import { useEffect, useState } from "react";
import { backend, type FeedItem, type Friend, type World } from "./backend";
import { Avatar, clock, dayLabel } from "./bits";
import { filters, sentence } from "./Activity";
import { openProfile } from "./Profile";

const PAGE = 200;

/** The whole saved activity feed: search it, filter it, jump back to a day. */
export function FeedBrowser({ friends, worlds }: { friends: Map<string, Friend>; worlds: Map<string, World> }) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState("all");
  const [day, setDay] = useState("");
  const [items, setItems] = useState<FeedItem[] | null>(null);
  const [more, setMore] = useState(false);

  const kinds = filters.find((f) => f.id === filter)?.kinds ?? [];
  // The end of the chosen day on this PC's clock, as the UTC time the feed is stored in.
  const until = day ? new Date(`${day}T23:59:59.999`).toISOString() : undefined;
  const plain = !query.trim() && filter === "all" && !day;
  // Counts the times older history came over from VRCX; the list is read again each time.
  const [round, setRound] = useState(0);
  useEffect(() => {
    const again = () => setRound((n) => n + 1);
    window.addEventListener("history:imported", again);
    return () => window.removeEventListener("history:imported", again);
  }, []);

  useEffect(() => {
    let alive = true;
    const wait = setTimeout(() => {
      void backend
        .searchFeed(query, kinds, undefined, until, PAGE)
        .then((list) => {
          if (!alive) return;
          setItems(list);
          setMore(list.length === PAGE);
        })
        .catch(() => alive && setItems([]));
    }, 250);
    return () => {
      alive = false;
      clearTimeout(wait);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query, filter, day, round]);

  // New entries slide in at the top while nothing narrows the list.
  useEffect(() => {
    if (!plain) return;
    let alive = true;
    let stop: (() => void) | undefined;
    // The listener may only be ready after this effect ended; then it is taken off at once.
    void backend.on("feed:new", (item) => alive && setItems((old) => (old ? [item, ...old] : old))).then((off) => (alive ? (stop = off) : off()));
    return () => {
      alive = false;
      stop?.();
    };
  }, [plain]);

  async function loadOlder() {
    if (!items || items.length === 0) return;
    const older = await backend.searchFeed(query, kinds, items[items.length - 1].id, until, PAGE);
    setItems([...items, ...older]);
    setMore(older.length === PAGE);
  }

  let lastDay = "";
  return (
    <div className="feed-browser">
      <div className="list-tools">
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search names, status text or world names"
          aria-label="Search the feed"
        />
        <label className="tool">
          Up to
          <input type="date" value={day} onChange={(e) => setDay(e.target.value)} />
        </label>
        {!plain && (
          <button
            className="plain"
            onClick={() => {
              setQuery("");
              setFilter("all");
              setDay("");
            }}
          >
            Clear
          </button>
        )}
      </div>
      <div className="filters filters-flat" role="group" aria-label="Kinds of activity">
        {filters.map((f) => (
          <button key={f.id} className={f.id === filter ? "on" : ""} aria-pressed={f.id === filter} onClick={() => setFilter(f.id)}>
            {f.label}
          </button>
        ))}
      </div>

      {items === null ? (
        <p className="empty">Reading the feed</p>
      ) : items.length === 0 ? (
        <p className="empty">
          {plain
            ? "Nothing saved yet. Everything friends do while the app runs is kept here, on this PC."
            : "Nothing matches. Try fewer words or another day."}
        </p>
      ) : (
        <ol className="feed-list">
          {items.map((item) => {
            const d = dayLabel(item.at);
            const header = d !== lastDay ? <li className="day" key={`day-${item.id}`}>{d}</li> : null;
            lastDay = d;
            return [
              header,
              <li className={`entry entry-${item.kind}`} key={item.id}>
                <time dateTime={item.at}>{clock(item.at)}</time>
                <Avatar name={item.displayName} image={friends.get(item.userId)?.image ?? ""} size={28} />
                <div className="entry-body">
                  <p>{sentence(item, worlds)}</p>
                </div>
                <button className="plain" onClick={() => openProfile(item.userId)}>
                  Profile
                </button>
              </li>,
            ];
          })}
        </ol>
      )}
      {more && (
        <button className="older" onClick={() => void loadOlder()}>
          Show older
        </button>
      )}
    </div>
  );
}
