import { Children, type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { backend, errorText, type Favorites, type Friend, type Instance, type Visit, type World } from "./backend";
import { accessName, deviceLine, deviceWords, regionName, tidy } from "./bits";
import { WayBack } from "./History";
import { openThing } from "./Detail";
import { parsePlace } from "./place";
import { FriendChip } from "./App";
import { whereText } from "./Everyone";
import { FriendMap } from "./FriendMap";
import { FriendWeb } from "./FriendWeb";

interface Room {
  location: string;
  worldId: string;
  people: Friend[];
}

const byName = (a: Friend, b: Friend) => a.displayName.localeCompare(b.displayName);

const askFirst = new Set(["invite", "invitePlus"]);

/** Past this many friends in one room, chips shrink to name and picture. */
const ROOMY = 6;
/** A crowded room or a long list shows this many until asked for the rest. */
const ROOM_CAP = 12;
const LIST_CAP = 30;

const UNIT = 4;
const GAP = 12;

/**
 * Lays cards out like bricks. Cards keep their reading order, left to right,
 * and each sits directly under the card above it, whatever the heights are.
 */
function Bricks({ className, children }: { className: string; children: ReactNode }) {
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const fit = () => {
      const tight = document.documentElement.dataset.cards === "tight";
      for (const cell of Array.from(el.children) as HTMLElement[]) {
        if (!tight) {
          cell.style.gridRowEnd = "";
          continue;
        }
        const inner = cell.firstElementChild as HTMLElement | null;
        if (!inner) continue;
        cell.style.gridRowEnd = `span ${Math.ceil((inner.offsetHeight + GAP) / UNIT)}`;
      }
    };
    fit();
    const watcher = new ResizeObserver(fit);
    watcher.observe(el);
    for (const cell of Array.from(el.children)) if (cell.firstElementChild) watcher.observe(cell.firstElementChild);
    return () => watcher.disconnect();
  });
  return (
    <div className={`bricks ${className}`} ref={box}>
      {Children.map(children, (child) => child && <div className="brick">{child}</div>)}
    </div>
  );
}

/** A list of chips that shows the first few and a button for the rest. */
function ChipList({ people, cap, children }: { people: Friend[]; cap: number; children: (f: Friend) => ReactNode }) {
  const [all, setAll] = useState(false);
  const shown = all ? people : people.slice(0, cap);
  return (
    <ul className="chips">
      {shown.map(children)}
      {people.length > cap && (
        <li>
          <button className="chips-more" onClick={() => setAll(!all)}>
            {all ? "Show fewer" : `Show all ${people.length}`}
          </button>
        </li>
      )}
    </ul>
  );
}

export function Around({
  friends,
  worlds,
  favorites,
  instances,
  query,
  here,
  onDone,
}: {
  /** The instance I am in myself, when I am in one. */
  here: Visit | null;
  friends: Friend[];
  worlds: Map<string, World>;
  favorites: Favorites;
  instances: Map<string, Instance>;
  query: string;
  onDone: (message: string) => void;
}) {
  const [showOffline, setShowOffline] = useState(false);
  // Cards, or the same people as a picture. The choice is kept.
  const [view, setView] = useState<"cards" | "map" | "web">(() => {
    try {
      // The browser demo can ask for the map by address ("?view=map").
      const asked = new URLSearchParams(location.search).get("view") ?? localStorage.getItem("around-view");
      return asked === "map" || asked === "web" ? asked : "cards";
    } catch {
      return "cards";
    }
  });
  const pickView = (next: "cards" | "map" | "web") => {
    setView(next);
    try {
      localStorage.setItem("around-view", next);
    } catch {
      // Without storage the choice lasts until the window closes.
    }
  };
  const [busy, setBusy] = useState("");

  async function getIn(room: Room) {
    const host = room.people[0];
    setBusy(room.location);
    try {
      await backend.requestInvite(host.id);
      onDone(`Asked ${host.displayName} for an invite. Their answer arrives in your Inbox.`);
    } catch (err) {
      onDone(`That did not work: ${errorText(err)}`);
    } finally {
      setBusy("");
    }
  }
  const q = query.trim().toLowerCase();

  const groups = useMemo(() => {
    const worldName = (id: string | null) => (id ? tidy(worlds.get(id)?.name ?? "") : "");
    const matches = (f: Friend) =>
      !q || f.displayName.toLowerCase().includes(q) || worldName(f.place.worldId).toLowerCase().includes(q);

    const rooms = new Map<string, Room>();
    const hidden: Friend[] = [];
    const moving: Friend[] = [];
    const web: Friend[] = [];
    const phone: Friend[] = [];
    const elsewhere: Friend[] = [];
    const offline: Friend[] = [];
    for (const f of friends) {
      if (!matches(f)) continue;
      if (f.state === "offline") offline.push(f);
      else if (f.state === "active") (f.platform === "web" ? web : deviceWords(f.platform) ? phone : elsewhere).push(f);
      else if (f.place.kind === "world" && f.place.worldId) {
        const room = rooms.get(f.location) ?? { location: f.location, worldId: f.place.worldId, people: [] };
        room.people.push(f);
        rooms.set(f.location, room);
      } else if (f.place.kind === "traveling") moving.push(f);
      else hidden.push(f);
    }
    const sorted = [...rooms.values()].sort(
      (a, b) => b.people.length - a.people.length || worldName(a.worldId).localeCompare(worldName(b.worldId)),
    );
    sorted.forEach((r) => r.people.sort(byName));
    return {
      rooms: sorted,
      hidden: hidden.sort(byName),
      moving: moving.sort(byName),
      web: web.sort(byName),
      phone: phone.sort(byName),
      elsewhere: elsewhere.sort(byName),
      offline: offline.sort(byName),
    };
  }, [friends, worlds, q]);

  const [openGroups, setOpenGroups] = useState<Set<string>>(new Set());
  const starred = useMemo(() => new Set(Object.keys(favorites.friends)), [favorites]);
  const pinned = useMemo(() => {
    return favorites.groups
      .map((group) => {
        const members = friends.filter((f) => favorites.friends[f.id] === group.key);
        const up = members
          .filter((f) => f.state !== "offline" && (!q || f.displayName.toLowerCase().includes(q)))
          .sort((a, b) => Number(a.state === "active") - Number(b.state === "active") || byName(a, b));
        const down = members.filter((f) => f.state === "offline" && (!q || f.displayName.toLowerCase().includes(q))).sort(byName);
        return { ...group, up, down, away: down.length };
      })
      .filter((g) => g.up.length + g.away > 0);
  }, [favorites, friends, q]);

  const nobody =
    groups.rooms.length + groups.hidden.length + groups.moving.length + groups.web.length + groups.phone.length + groups.elsewhere.length + groups.offline.length === 0;

  if (nobody) {
    return (
      <p className="empty">
        {q ? `Nobody matches "${query.trim()}".` : "Your friend list is still loading, or it is empty."}
      </p>
    );
  }

  return (
    <div className="around">
      <div className="segments around-view" role="radiogroup" aria-label="How friends are shown">
        <button role="radio" aria-checked={view === "cards"} className={view === "cards" ? "on" : ""} onClick={() => pickView("cards")}>
          Cards
        </button>
        <button role="radio" aria-checked={view === "map"} className={view === "map" ? "on" : ""} onClick={() => pickView("map")}>
          Map
        </button>
        <button role="radio" aria-checked={view === "web"} className={view === "web" ? "on" : ""} onClick={() => pickView("web")} title="Which of your friends are friends with each other">
          Friend web
        </button>
      </div>
      {view === "web" && <FriendWeb friends={friends} onDone={onDone} />}
      {view === "map" && (
        <FriendMap
          rooms={groups.rooms}
          hidden={groups.hidden}
          moving={groups.moving}
          away={[...groups.web, ...groups.phone, ...groups.elsewhere]}
          worlds={worlds}
          instances={instances}
          starred={starred}
          here={here}
          onDone={onDone}
        />
      )}
      <div className="around-cards" hidden={view !== "cards"}>
      {pinned.length > 0 && (
        <Bricks className="pinned">
          {pinned.map((g) => (
            <div className="pinned-group" key={g.key}>
              <h2>
                <span className="star">★</span> {g.name}
              </h2>
              {g.up.length > 0 ? (
                <ul className="chips">
                  {g.up.map((f) => (
                    <FriendChip
                      f={f}
                      key={f.id}
                      note={f.state === "active" ? deviceLine(f.platform) : whereText(f, worlds)}
                    />
                  ))}
                </ul>
              ) : (
                <p className="pinned-none">Nobody from this group is around.</p>
              )}
              {g.away > 0 && (
                <>
                  <button
                    className="pinned-away"
                    aria-expanded={openGroups.has(g.key)}
                    title={openGroups.has(g.key) ? "Hide the offline ones" : "Show everyone in this group"}
                    onClick={() =>
                      setOpenGroups((old) => {
                        const next = new Set(old);
                        if (next.has(g.key)) next.delete(g.key);
                        else next.add(g.key);
                        return next;
                      })
                    }
                  >
                    {g.away} offline, {openGroups.has(g.key) ? "hide" : "show"}
                  </button>
                  {openGroups.has(g.key) && (
                    <ul className="chips pinned-offline">
                      {g.down.map((f) => (
                        <FriendChip f={f} key={f.id} note="Offline" />
                      ))}
                    </ul>
                  )}
                </>
              )}
            </div>
          ))}
        </Bricks>
      )}
      <Bricks className="rooms">
      {groups.rooms.map((room) => {
        const world = worlds.get(room.worldId);
        const place = room.people[0].place;
        const count = instances.get(room.location);
        const ask = askFirst.has(place.access ?? "");
        return (
          <article className="room" key={room.location}>
            <div className="room-pic" style={world?.image ? { backgroundImage: `url("${world.image}")` } : undefined}>
              <span className="room-count" aria-label={`${room.people.length} friends here`}>
                {room.people.length}
              </span>
            </div>
            <div className="room-body">
              <h2>
                <button className="title-link" onClick={() => openThing("world", room.worldId)} title="Open this world">
                  {world ? tidy(world.name) : "Looking up this world"}
                </button>
              </h2>
              <p className="room-meta">
                <span className={`access access-${place.access}`}>{accessName(place)}</span>
                {regionName(place) && <span>{regionName(place)}</span>}
                {count && (
                  <span className={count.full ? "room-full" : ""}>
                    {count.capacity > 0 ? `${count.users} of ${count.capacity} people` : `${count.users} people`}
                    {count.full ? ", full" : ""}
                  </span>
                )}
                {place.instance && <span title="Instance number">#{place.instance}</span>}
              </p>
              <ChipList people={room.people} cap={ROOM_CAP}>
                {(f) => (
                  <FriendChip
                    f={f}
                    key={f.id}
                    starred={f.id in favorites.friends}
                    showSince
                    small={room.people.length > ROOMY}
                  />
                )}
              </ChipList>
            </div>
            <div className="room-foot">
              {ask ? (
                <button
                  className="room-go"
                  disabled={busy === room.location}
                  title={`Sends ${room.people[0].displayName} a request to invite you`}
                  onClick={() => void getIn(room)}
                >
                  Ask <span className="who">{room.people[0].displayName}</span> for an invite
                </button>
              ) : (
                <WayBack location={room.location} onDone={onDone} />
              )}
            </div>
          </article>
        );
      })}
      </Bricks>

      {groups.moving.length > 0 && (
        <section className="band">
          <h2>On the way somewhere</h2>
          <ul className="chips">
            {groups.moving.map((f) => {
              const target = parsePlace(f.travelingTo);
              const to = target.kind === "world" && target.worldId ? worlds.get(target.worldId)?.name : "";
              return <FriendChip f={f} key={f.id} note={to ? `Heading to ${tidy(to)}` : target.kind === "private" ? "Heading to a private world" : undefined} />;
            })}
          </ul>
        </section>
      )}
      {groups.hidden.length > 0 && (
        <section className="band">
          <h2>In private worlds</h2>
          <ChipList people={groups.hidden} cap={LIST_CAP}>{(f) => <FriendChip f={f} key={f.id} />}</ChipList>
        </section>
      )}
      {groups.web.length > 0 && (
        <section className="band">
          <h2>On the website</h2>
          <ChipList people={groups.web} cap={LIST_CAP}>{(f) => <FriendChip f={f} key={f.id} />}</ChipList>
        </section>
      )}
      {groups.phone.length > 0 && (
        <section className="band">
          <h2>On the phone app</h2>
          <ChipList people={groups.phone} cap={LIST_CAP}>{(f) => <FriendChip f={f} key={f.id} tag={deviceWords(f.platform)} />}</ChipList>
        </section>
      )}
      {groups.elsewhere.length > 0 && (
        <section className="band">
          <h2>On the website or phone</h2>
          <ul className="chips">{groups.elsewhere.map((f) => <FriendChip f={f} key={f.id} />)}</ul>
        </section>
      )}
      {groups.offline.length > 0 && (
        <section className="band band-dim">
          <h2>
            <button className="fold" aria-expanded={showOffline || !!q} onClick={() => setShowOffline((v) => !v)}>
              Offline, {groups.offline.length}
            </button>
          </h2>
          {(showOffline || q) && (
            <ChipList people={groups.offline} cap={60}>{(f) => <FriendChip f={f} key={f.id} small />}</ChipList>
          )}
        </section>
      )}
      </div>
    </div>
  );
}
