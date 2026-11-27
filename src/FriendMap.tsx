import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { backend, errorText, type Friend, type Instance, type Visit, type World } from "./backend";
import { Avatar, accessName, regionName, statusTone, statusWords, tidy } from "./bits";
import { openThing } from "./Detail";
import { launched } from "./History";
import { parsePlace } from "./place";
import { openProfile } from "./Profile";
import { savedLaunchMode } from "./settings";
import { toggleWait, useWaiting } from "./Queue";

interface Room {
  location: string;
  worldId: string;
  people: Friend[];
}

const FACE = 48;
/** The room one face takes on a ring: the picture, and its name under it. */
const SEAT = 78;
const askFirst = new Set(["invite", "invitePlus"]);

/** How wide a bubble is: enough for its faces and their names, a little more the fuller the instance is. */
function width(friends: number, everyone: number): number {
  // Up to nine faces sit on one ring, which has to be long enough for all of them.
  const oneRing = 2 * ((friends * SEAT) / (2 * Math.PI) + FACE / 2 + 22) + 4;
  const forFaces = friends <= 1 ? 150 : friends <= 9 ? Math.max(210, oneRing) : 120 + (friends * SEAT) / Math.PI;
  return Math.round(Math.min(460, forFaces + Math.min(50, Math.sqrt(Math.max(0, everyone - friends)) * 8)));
}

/** Where each face sits in a bubble: one in the middle, more on a ring, many on two rings. */
function seats(count: number, size: number): Array<{ x: number; y: number }> {
  if (count === 1) return [{ x: 0, y: -6 }];
  const outer = size / 2 - FACE / 2 - 22;
  const fits = Math.max(2, Math.floor((2 * Math.PI * outer + 1) / SEAT));
  // A single face left over only goes to the middle when the ring is wide enough around it.
  const spare = count - Math.min(count, fits);
  const onOuter = spare === 1 && outer < SEAT ? count : Math.min(count, fits);
  const inside = count - onOuter;
  const ring = (n: number, radius: number, turn: number) =>
    Array.from({ length: n }, (_, i) => {
      const angle = turn + (i / n) * 2 * Math.PI;
      return { x: Math.cos(angle) * radius, y: Math.sin(angle) * radius - 6 };
    });
  return [...ring(onOuter, outer, -Math.PI / 2), ...(inside === 1 ? [{ x: 0, y: -6 }] : ring(inside, Math.max(0, outer - SEAT), -Math.PI / 2 + 0.5))];
}

function Face({ f, at, starred, look }: { f: Friend; at: { x: number; y: number }; starred: boolean; look: (id: string) => void }) {
  const status = f.statusDescription ? `${statusWords(f)}: ${f.statusDescription}` : statusWords(f);
  return (
    <button
      className={`map-face${starred ? " map-starred" : ""}`}
      style={{ translate: `${at.x}px ${at.y}px` }}
      title={`${f.displayName}. ${status}`}
      data-uid={f.id}
      onClick={() => openProfile(f.id)}
      onPointerEnter={() => look(f.id)}
      onPointerLeave={() => look("")}
      onFocus={() => look(f.id)}
      onBlur={() => look("")}
    >
      <Avatar name={f.displayName} image={f.image} tone={statusTone(f)} size={FACE} />
      <span className="map-name hide-me">{f.displayName}</span>
      {starred && (
        <span className="map-star" aria-label="A favourite">
          ★
        </span>
      )}
    </button>
  );
}

function Bubble({
  people,
  size,
  picture,
  tone,
  starred,
  order,
  mine,
  look,
  children,
}: {
  people: Friend[];
  size: number;
  picture?: string;
  tone: string;
  starred: Set<string>;
  order: number;
  /** I am in this instance myself. */
  mine?: boolean;
  look: (id: string) => void;
  children: ReactNode;
}) {
  // Faces past what two rings hold are counted, not drawn. Favourites are seated first.
  const most = 22;
  const sorted = [...people].sort((a, b) => Number(starred.has(b.id)) - Number(starred.has(a.id)));
  const shown = sorted.slice(0, most);
  const places = seats(shown.length, size);
  return (
    <li className={`map-room map-${tone}${mine ? " map-mine" : ""}`} style={{ "--order": order } as CSSProperties}>
      <div className="map-bubble" style={{ width: size, height: size, "--pic": picture ? `url("${picture}")` : "none" } as CSSProperties}>
        {mine && <span className="map-here">You are here</span>}
        {shown.map((f, i) => (
          <Face key={f.id} f={f} at={places[i]} starred={starred.has(f.id)} look={look} />
        ))}
        {mine && shown.length === 0 && <span className="map-alone">No friends with you</span>}
        {people.length > most && <span className="map-more">and {people.length - most} more</span>}
      </div>
      {children}
    </li>
  );
}

/** Two friends, and in how many of my stays I saw both. */
interface Pair {
  a: string;
  b: string;
  times: number;
}

const legend: Array<[string, string]> = [
  ["public", "Public"],
  ["friends", "Friends, Friends+"],
  ["invite", "Invite, Invite+"],
  ["group", "Group"],
  ["private", "Private or not in a world"],
];

/**
 * Who is with whom, as a picture: one bubble per instance with the friends in it, bigger the
 * more of them there are, then one bubble each for private worlds, people between worlds and
 * people who are signed in without being in the game.
 */
export function FriendMap({
  rooms,
  hidden,
  moving,
  away,
  worlds,
  instances,
  starred,
  here,
  onDone,
}: {
  rooms: Room[];
  hidden: Friend[];
  moving: Friend[];
  /** Signed in on the website or a phone, not in the game. */
  away: Friend[];
  worlds: Map<string, World>;
  instances: Map<string, Instance>;
  /** Friends in one of my favourite groups. */
  starred: Set<string>;
  /** The instance I am in myself, when I am in one. */
  here: Visit | null;
  onDone: (message: string) => void;
}) {
  const [busy, setBusy] = useState("");
  const waiting = useWaiting();
  const [showAway, setShowAway] = useState(false);

  // Lines between friends I often saw in the same instance. Which pairs comes from my own
  // history; where the lines run is measured from the faces once they are on screen.
  const [linesOn, setLinesOn] = useState(() => {
    try {
      return localStorage.getItem("map-lines") !== "off";
    } catch {
      return true;
    }
  });
  const ids = useMemo(() => [...rooms.flatMap((r) => r.people), ...hidden, ...moving].map((f) => f.id).sort().join(","), [rooms, hidden, moving]);
  const [pairs, setPairs] = useState<Pair[]>([]);
  useEffect(() => {
    if (!linesOn || !ids.includes(",")) {
      setPairs([]);
      return;
    }
    let alive = true;
    // A moment's wait, so friends coming and going in a burst ask the history once.
    const later = window.setTimeout(() => {
      void backend
        .pairs(ids.split(","))
        .then((p) => alive && setPairs(p))
        .catch(() => {});
    }, 400);
    return () => {
      alive = false;
      window.clearTimeout(later);
    };
  }, [ids, linesOn]);
  const [focus, setFocus] = useState("");
  // The map can be dragged and zoomed like the friend web.
  const [view, setView] = useState({ x: 0, y: 0, k: 1 });
  const stage = useRef<HTMLDivElement>(null);
  const drag = useRef<{ x: number; y: number; moved: boolean } | null>(null);
  const wrap = useRef<HTMLDivElement>(null);
  // Everything in view: a map taller than its stage is drawn smaller, never bigger than life.
  // The stage is as tall as the bubbles need. Only a map taller than the stage may be is drawn smaller.
  const fitMap = () => {
    const [frame, world] = [stage.current, wrap.current];
    if (!frame || !world) return;
    const room = frame.clientHeight - 56;
    const k = world.offsetHeight <= room ? 1 : Math.max(0.4, room / world.offsetHeight);
    const [x, y] = [((1 - k) * world.offsetWidth) / 2, Math.max(0, (room - world.offsetHeight * k) / 2)];
    // The same answer as before changes nothing, so an observer that fires again does not redraw.
    setView((v) => (v.k === k && v.x === x && v.y === y ? v : { k, x, y }));
  };
  // Changes whenever the bubbles on the map do, the stage coming and going included.
  const shape = `${rooms.map((r) => r.people.length).join(",")}|${hidden.length}|${moving.length}|${showAway ? away.length : 0}`;
  // The wheel zooms the map and nothing else; the page behind it stays where it is.
  useEffect(() => {
    const el = stage.current;
    if (!el) return;
    const turn = (e: WheelEvent) => {
      e.preventDefault();
      moved.current = true;
      const r = el.getBoundingClientRect();
      const zoom = r.width / el.offsetWidth || 1;
      const [px, py] = [(e.clientX - r.left) / zoom, (e.clientY - r.top) / zoom];
      setView((v) => {
        const k = Math.min(2.2, Math.max(0.4, v.k * (e.deltaY > 0 ? 1 / 1.15 : 1.15)));
        return { k, x: px - ((px - v.x) / v.k) * k, y: py - ((py - v.y) / v.k) * k };
      });
    };
    el.addEventListener("wheel", turn, { passive: false });
    return () => el.removeEventListener("wheel", turn);
  }, [shape]);
  useLayoutEffect(fitMap, [shape]); // eslint-disable-line react-hooks/exhaustive-deps
  /** True once I have dragged or zoomed; from then on the map stays where I put it. */
  const moved = useRef(false);
  useEffect(() => {
    const world = wrap.current;
    if (!world) return;
    const watch = new ResizeObserver(() => {
      if (!moved.current) fitMap();
    });
    watch.observe(world);
    if (stage.current) watch.observe(stage.current);
    return () => watch.disconnect();
  }, [shape]); // eslint-disable-line react-hooks/exhaustive-deps
  const [lines, setLines] = useState<Array<Pair & { x1: number; y1: number; x2: number; y2: number }>>([]);
  useLayoutEffect(() => {
    const box = wrap.current;
    if (!box || pairs.length === 0) {
      setLines([]);
      return;
    }
    const measure = () => {
      // Where a face sits on the map itself, worked out from the layout: its place in its
      // bubble plus the bubble's place on the map. Zooming, dragging and the bubbles growing
      // in do not change these numbers, so the lines never drift.
      const centre = (id: string) => {
        const face = box.querySelector<HTMLElement>(`[data-uid="${CSS.escape(id)}"]`);
        if (!face) return null;
        const [tx, ty] = (face.style.translate || "0px 0px").split(" ").map((n) => parseFloat(n) || 0);
        let x = face.offsetLeft + face.offsetWidth / 2 + tx;
        let y = face.offsetTop + ty + FACE / 2;
        for (let up = face.offsetParent as HTMLElement | null; up && up !== box; up = up.offsetParent as HTMLElement | null) {
          x += up.offsetLeft;
          y += up.offsetTop;
        }
        return { x, y };
      };
      const out = [];
      for (const p of pairs) {
        const [a, b] = [centre(p.a), centre(p.b)];
        if (!a || !b) continue;
        // Each line stops at the edge of the two faces.
        const length = Math.hypot(b.x - a.x, b.y - a.y);
        if (length < FACE + 8) continue;
        const cut = (FACE / 2 + 3) / length;
        out.push({ ...p, x1: a.x + (b.x - a.x) * cut, y1: a.y + (b.y - a.y) * cut, x2: b.x - (b.x - a.x) * cut, y2: b.y - (b.y - a.y) * cut });
      }
      setLines(out);
    };
    measure();
    // Pictures and fonts arriving can still nudge the layout a moment later.
    const settled = window.setTimeout(measure, 900);
    const watch = new ResizeObserver(measure);
    watch.observe(box);
    return () => {
      window.clearTimeout(settled);
      watch.disconnect();
    };
  }, [pairs, showAway, rooms, hidden, moving]);
  const most = lines.reduce((top, l) => Math.max(top, l.times), 1);
  const toggleLines = () => {
    setLinesOn((on) => {
      try {
        localStorage.setItem("map-lines", on ? "off" : "on");
      } catch {
        // Without storage the choice lasts until the window closes.
      }
      return !on;
    });
  };
  // My own instance gets a bubble too when no friend is in it with me.
  const alone = here && !rooms.some((r) => r.location === here.location) ? here : null;

  async function go(room: Room, ask: boolean) {
    setBusy(room.location);
    try {
      if (ask) {
        await backend.requestInvite(room.people[0].id);
        onDone(`Asked ${room.people[0].displayName} for an invite. Their answer arrives in your Inbox.`);
      } else {
        onDone(launched[await backend.launchInstance(room.location, savedLaunchMode())]);
      }
    } catch (err) {
      onDone(`That did not work: ${errorText(err)}`);
    } finally {
      setBusy("");
    }
  }

  if (rooms.length + hidden.length + moving.length + away.length === 0 && !alone) return <p className="empty">Nobody is around right now. Friends show up here as bubbles once they are in VRChat.</p>;
  const inGame = rooms.reduce((n, r) => n + r.people.length, 0) + hidden.length + moving.length;
  const names = new Map([...rooms.flatMap((r) => r.people), ...hidden, ...moving].map((f) => [f.id, f.displayName]));
  const together = pairs.slice(0, 6).map((p) => ({ ...p, names: [names.get(p.a) ?? "A friend", names.get(p.b) ?? "a friend"] }));
  let order = 0;
  return (
    <div className="map-wrap">
      <dl className="town-facts">
        <div>
          <dt>Friends in VRChat</dt>
          <dd>{inGame}</dd>
        </div>
        <div>
          <dt>Instances you can see</dt>
          <dd>{rooms.length}</dd>
        </div>
        <div>
          <dt>In private worlds</dt>
          <dd>{hidden.length}</dd>
        </div>
        <div>
          <dt>Biggest gathering</dt>
          <dd className="town-fact-name hide-me">{rooms[0] ? `${tidy(worlds.get(rooms[0].worldId)?.name ?? "") || "A world"}, ${rooms[0].people.length}` : "Nobody together"}</dd>
        </div>
      </dl>

      <div className="town-layout">
      <div
        className="map-stage"
        ref={stage}
        onPointerDown={(e) => {
          drag.current = { x: e.clientX, y: e.clientY, moved: false };
        }}
        onPointerMove={(e) => {
          const d = drag.current;
          if (!d || e.buttons !== 1) return;
          const [mx, my] = [e.clientX - d.x, e.clientY - d.y];
          if (!d.moved && Math.abs(mx) + Math.abs(my) < 4) return;
          drag.current = { x: e.clientX, y: e.clientY, moved: true };
          moved.current = true;
          setView((v) => ({ ...v, x: v.x + mx, y: v.y + my }));
        }}
        onPointerUp={() => window.setTimeout(() => (drag.current = null), 0)}
        onClickCapture={(e) => {
          // Letting go after a drag is not a press on whatever was under the pointer.
          if (drag.current?.moved) {
            e.stopPropagation();
            e.preventDefault();
          }
        }}
      >
      <div className="map-world" ref={wrap} style={{ transform: `translate(${view.x}px, ${view.y}px) scale(${view.k})` }}>
      {lines.length > 0 && (
        <svg className={`map-lines${focus ? " map-focused" : ""}`} aria-hidden="true">
          {lines.map((l) => (
            <line
              key={`${l.a}:${l.b}`}
              x1={l.x1}
              y1={l.y1}
              x2={l.x2}
              y2={l.y2}
              className={focus && (l.a === focus || l.b === focus) ? "on" : ""}
              strokeWidth={1.2 + (l.times / most) * 3.2}
            />
          ))}
        </svg>
      )}
      <ul className="friend-map" aria-label="Friends by the instance they are in">
        {alone && (
          <Bubble people={[]} size={150} picture={worlds.get(alone.worldId)?.image} tone={alone.place.access || "public"} starred={starred} order={order++} mine look={setFocus}>
            <button className="map-label" onClick={() => openThing("instance", alone.location)} title="Open this instance">
              <b>{tidy(alone.worldName) || "A world"}</b>
              <span>{[accessName(alone.place), regionName(alone.place)].filter(Boolean).join(", ")}</span>
            </button>
          </Bubble>
        )}
        {rooms.map((room) => {
          const world = worlds.get(room.worldId);
          const instance = instances.get(room.location);
          const everyone = instance?.users ?? 0;
          const place = parsePlace(room.location);
          const name = tidy(world?.name ?? "") || "A world";
          const waiting = instance?.queueEnabled && instance.queueSize > 0 ? `${instance.queueSize} waiting` : "";
          const facts = [accessName(place), regionName(place), everyone > 0 ? `${room.people.length} of ${everyone} here are friends` : "", waiting].filter(Boolean);
          return (
            <Bubble
              key={room.location}
              people={room.people}
              size={width(room.people.length, everyone)}
              picture={world?.image}
              tone={place.access || "public"}
              starred={starred}
              order={order++}
              mine={here?.location === room.location}
              look={setFocus}
            >
              <button className="map-label" onClick={() => openThing("instance", room.location)} title="Open this instance">
                <b>{name}</b>
                <span>{facts.join(", ")}</span>
              </button>
            </Bubble>
          );
        })}
        {hidden.length > 0 && (
          <Bubble people={hidden} size={width(hidden.length, hidden.length)} tone="private" starred={starred} order={order++} look={setFocus}>
            <span className="map-label">
              <b>Private worlds</b>
              <span>
                {hidden.length} {hidden.length === 1 ? "friend" : "friends"}. VRChat does not say who is with whom.
              </span>
            </span>
          </Bubble>
        )}
        {moving.length > 0 && (
          <Bubble people={moving} size={width(moving.length, moving.length)} tone="moving" starred={starred} order={order++} look={setFocus}>
            <span className="map-label">
              <b>On the way</b>
              <span>Between worlds right now</span>
            </span>
          </Bubble>
        )}
        {showAway && away.length > 0 && (
          <Bubble people={away} size={width(away.length, away.length)} tone="away" starred={starred} order={order++} look={setFocus}>
            <span className="map-label">
              <b>Not in the game</b>
              <span>Signed in on the website or a phone</span>
            </span>
          </Bubble>
        )}
      </ul>
      </div>
      <div className="town-zoom">
        <button className="plain" onClick={() => ((moved.current = true), setView((v) => ({ ...v, k: Math.min(2.2, v.k * 1.2) })))} aria-label="Closer" title="Closer">
          +
        </button>
        <button className="plain" onClick={() => ((moved.current = true), setView((v) => ({ ...v, k: Math.max(0.4, v.k / 1.2) })))} aria-label="Further away" title="Further away">
          −
        </button>
        <button
          className="plain"
          onClick={() => {
            moved.current = false;
            fitMap();
          }}
        >
          Whole map
        </button>
      </div>
      </div>

      <aside className="town-rail" aria-label="Instances and key">
        {rooms.length > 0 && (
          <section className="town-card">
            <h3>Instances</h3>
            <ul className="map-rooms">
              {rooms.map((room) => {
                const instance = instances.get(room.location);
                const place = parsePlace(room.location);
                const ask = askFirst.has(place.access ?? "");
                const mine = here?.location === room.location;
                return (
                  <li key={room.location} className={`map-${place.access || "public"}`}>
                    <button className="map-room-name" onClick={() => openThing("instance", room.location)} title="Open this instance">
                      <i aria-hidden="true" />
                      <span>
                        <b className="hide-me">{tidy(worlds.get(room.worldId)?.name ?? "") || "A world"}</b>
                        <small>
                          {accessName(place)}, {room.people.length} {room.people.length === 1 ? "friend" : "friends"}
                          {instance?.users ? ` of ${instance.users}` : ""}
                          {instance?.queueEnabled && instance.queueSize > 0 ? `, ${instance.queueSize} waiting` : ""}
                        </small>
                      </span>
                    </button>
                    {mine ? (
                      <span className="mark mark-live">You</span>
                    ) : (
                      (() => {
                        const full = !ask && instance?.full === true;
                        const waits = waiting.some((w) => w.location === room.location);
                        const worldName = tidy(worlds.get(room.worldId)?.name ?? "") || "A world";
                        return (
                          <button
                            className={ask || full ? "plain" : "plain go"}
                            disabled={busy === room.location}
                            title={ask ? `Asks ${room.people[0].displayName} for an invite` : waits ? "You are waiting for room here. Press to stop" : full ? "This instance is full. Press to be told when it has room" : "Starts VRChat in this instance, or moves you there"}
                            onClick={() => {
                              if (!full && !waits) return void go(room, ask);
                              setBusy(room.location);
                              void toggleWait(room.location, worldName, waits, onDone).finally(() => setBusy(""));
                            }}
                          >
                            {ask ? "Ask" : waits ? "Waiting" : full ? "Full, wait" : "Join"}
                          </button>
                        );
                      })()
                    )}
                  </li>
                );
              })}
            </ul>
          </section>
        )}

        {together.length > 0 && (
          <section className="town-card">
            <h3>Often together</h3>
            <p className="town-note">Friends on the map now whom you saw in the same instance most often.</p>
            <ul className="map-pairs">
              {together.map((p) => (
                <li key={`${p.a}:${p.b}`} onPointerEnter={() => setFocus(p.a)} onPointerLeave={() => setFocus("")}>
                  <span className="hide-name">
                    {p.names[0]} and {p.names[1]}
                  </span>
                  <small>{p.times} times</small>
                </li>
              ))}
            </ul>
          </section>
        )}

        <section className="town-card">
          <h3>Key</h3>
          <ul className="map-legend" aria-label="What the ring colours mean">
            {legend.map(([tone, words]) => (
              <li key={tone} className={`map-${tone}`}>
                <i aria-hidden="true" />
                {words}
              </li>
            ))}
            <li>
              <span className="map-star" aria-hidden="true">
                ★
              </span>
              A favourite
            </li>
          </ul>
          <label className="map-toggle" title="A line joins two friends you saw in the same instance at least three times, thicker the more often. From your own world history">
            <span>Lines between friends often together</span>
            <input type="checkbox" role="switch" checked={linesOn} onChange={toggleLines} />
          </label>
          {away.length > 0 && (
            <label className="map-toggle">
              <span>Show the {away.length} on the website or phone</span>
              <input type="checkbox" role="switch" checked={showAway} onChange={(e) => setShowAway(e.target.checked)} />
            </label>
          )}
        </section>
      </aside>
      </div>
    </div>
  );
}
