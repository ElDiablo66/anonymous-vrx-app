import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { backend, errorText, type Friend } from "./backend";
import { Avatar, daysAgo, statusTone } from "./bits";
import { openProfile } from "./Profile";

interface Known {
  pairs: Array<[string, string]>;
  asked: number;
  hidden: string[];
  at: string;
  running: boolean;
}

/** A circle of friends who mostly know each other, drawn as one district. */
interface District {
  index: number;
  name: string;
  colour: string;
  members: string[];
  x: number;
  y: number;
  r: number;
}

const FACE = 42;
const colours = ["#ff4fa3", "#2de2e6", "#ff9a3c", "#b18cff", "#5fe3a1", "#4fb8ff", "#ffd166", "#ff7a5c"];

/**
 * Finds circles of friends: everyone starts alone, then keeps taking the circle most of
 * their friends are in until nothing moves. Circles are numbered biggest first.
 */
function circles(ids: string[], near: Map<string, string[]>): Map<string, number> {
  const of = new Map(ids.map((id) => [id, id]));
  for (let round = 0; round < 12; round++) {
    let moved = false;
    for (const id of ids) {
      const votes = new Map<string, number>();
      for (const other of near.get(id) ?? []) votes.set(of.get(other)!, (votes.get(of.get(other)!) ?? 0) + 1);
      let best = of.get(id)!;
      let most = 0;
      for (const [label, n] of [...votes].sort(([a], [b]) => a.localeCompare(b))) {
        if (n > most) {
          most = n;
          best = label;
        }
      }
      if (best !== of.get(id)) {
        of.set(id, best);
        moved = true;
      }
    }
    if (!moved) break;
  }
  const sizes = new Map<string, number>();
  for (const label of of.values()) sizes.set(label, (sizes.get(label) ?? 0) + 1);
  const order = [...sizes].sort(([a, x], [b, y]) => y - x || a.localeCompare(b)).map(([label]) => label);
  return new Map(ids.map((id) => [id, order.indexOf(of.get(id)!)]));
}

/** How far apart seats are; a face with its name under it needs about this much. */
const STEP = 36;

/**
 * Seats for a district's members, best connected nearest the middle, and how big that makes
 * the district. Seats spiral outwards the way seeds sit in a sunflower, which fills a round
 * space evenly whatever the number, so a district is never bigger than its people need.
 */
function seatsFor(count: number): { seats: Array<{ x: number; y: number }>; r: number } {
  if (count === 2) return { seats: [{ x: -33, y: 0 }, { x: 33, y: 0 }], r: 82 };
  const seats = Array.from({ length: count }, (_, i) => {
    const far = STEP * Math.sqrt(i + 0.55);
    const turn = i * 2.39996323;
    return { x: Math.cos(turn) * far, y: Math.sin(turn) * far };
  });
  // Room past the outermost seat for the face, its name, and the ring itself.
  return { seats, r: STEP * Math.sqrt(count - 0.45) + 46 };
}

/**
 * Everything the picture needs: the districts with their place on the board, each friend's
 * seat, and how many friendships cross from one district to another.
 */
function plan(friends: Friend[], pairs: Array<[string, string]>, shape: number) {
  const byId = new Map(friends.map((f) => [f.id, f]));
  const ids = friends.map((f) => f.id).sort();
  const near = new Map<string, string[]>(ids.map((id) => [id, []]));
  const links: Array<[string, string]> = [];
  for (const [a, b] of pairs) {
    if (!near.has(a) || !near.has(b) || a === b) continue;
    near.get(a)!.push(b);
    near.get(b)!.push(a);
    links.push([a, b]);
  }
  const linked = ids.filter((id) => near.get(id)!.length > 0);
  const circle = circles(linked, near);
  const groups = new Map<number, string[]>();
  for (const id of linked) groups.set(circle.get(id)!, [...(groups.get(circle.get(id)!) ?? []), id]);

  const districts: District[] = [...groups.entries()]
    .sort(([a], [b]) => a - b)
    .map(([, members], index) => {
      members.sort((a, b) => near.get(b)!.length - near.get(a)!.length || (byId.get(a)?.displayName ?? "").localeCompare(byId.get(b)?.displayName ?? ""));
      const lead = byId.get(members[0])?.displayName ?? "A friend";
      const name = members.length === 2 ? `${lead} and ${byId.get(members[1])?.displayName ?? "a friend"}` : `${lead}'s circle`;
      return { index, name, colour: colours[index % colours.length], members, x: 0, y: 0, r: seatsFor(members.length).r };
    });

  // Districts stand in rows, biggest first, like blocks on a street map.
  const gap = 56;
  // The rows are as long as it takes for the town to have the shape of its stage (`shape`
  // is the stage's width over its height), so a wide stage is filled from side to side.
  const ground = districts.reduce((sum, d) => sum + (2 * d.r + gap) ** 2, 0);
  const widest = Math.max(2 * (districts[0]?.r ?? 0) + 2 * gap, Math.sqrt(ground * shape) * 1.08);
  let x = gap;
  let y = gap;
  let row = 0;
  for (const d of districts) {
    if (x + 2 * d.r + gap > widest && x > gap) {
      x = gap;
      y += row + gap;
      row = 0;
    }
    d.x = x + d.r;
    d.y = y + d.r;
    x += 2 * d.r + gap;
    row = Math.max(row, 2 * d.r);
  }
  // Rows are centred on the tallest block in them.
  const rows = new Map<number, District[]>();
  for (const d of districts) rows.set(d.y - d.r, [...(rows.get(d.y - d.r) ?? []), d]);
  for (const [top, inRow] of rows) {
    const tall = Math.max(...inRow.map((d) => d.r));
    for (const d of inRow) d.y = top + tall;
  }
  const width = Math.max(...districts.map((d) => d.x + d.r), 600) + gap;
  const height = Math.max(...districts.map((d) => d.y + d.r), 300) + gap;

  const seat = new Map<string, { x: number; y: number; district: number }>();
  for (const d of districts) {
    const { seats } = seatsFor(d.members.length);
    d.members.forEach((id, i) => seat.set(id, { x: d.x + seats[i].x, y: d.y + seats[i].y, district: d.index }));
  }
  const bridges = new Map<string, number>();
  for (const [a, b] of links) {
    const [p, q] = [seat.get(a)!.district, seat.get(b)!.district];
    if (p !== q) bridges.set(`${Math.min(p, q)}-${Math.max(p, q)}`, (bridges.get(`${Math.min(p, q)}-${Math.max(p, q)}`) ?? 0) + 1);
  }
  // Someone with friends in other districts holds the web together.
  const reach = new Map<string, number>();
  for (const id of linked) reach.set(id, new Set(near.get(id)!.map((o) => seat.get(o)!.district).filter((d) => d !== seat.get(id)!.district)).size);
  const loners = ids.filter((id) => near.get(id)!.length === 0);
  return { byId, near, links, districts, seat, bridges, reach, loners, width, height };
}

const endWords: Record<string, string> = {
  done: "Done.",
  stopped: "Stopped. What was found so far is kept.",
  slow: "VRChat asked to slow down, so it stopped. What was found so far is kept; try again later.",
  signedOut: "You are signed out, so it stopped.",
  network: "VRChat could not be reached, so it stopped. What was found so far is kept.",
  refused: "VRChat turned the question down, so it stopped.",
};

/**
 * Which of my friends are friends with each other, drawn as a town: every circle of friends
 * is a district with its people in it, and friendships that cross between two districts
 * are a bridge. A rail beside it lists the districts and says who holds them together.
 */
export function FriendWeb({ friends, onDone }: { friends: Friend[]; onDone: (message: string) => void }) {
  const [known, setKnown] = useState<Known | null>(null);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [busy, setBusy] = useState(false);
  const [hover, setHover] = useState("");
  const [picked, setPicked] = useState("");
  const [zone, setZone] = useState(-1);
  const [find, setFind] = useState("");
  const [view, setView] = useState({ x: 0, y: 0, k: 1 });
  const stage = useRef<HTMLDivElement>(null);
  const drag = useRef<{ x: number; y: number; moved: boolean } | null>(null);

  const load = () =>
    backend
      .mutuals()
      .then((k) => {
        setKnown(k);
        setBusy(k.running);
      })
      .catch(() => setKnown({ pairs: [], asked: 0, hidden: [], at: "", running: false }));
  useEffect(() => void load(), []);
  useEffect(() => {
    let stop: (() => void) | undefined;
    let gone = false;
    void backend.on("mutuals:progress", (p) => setProgress(p)).then((off) => (gone ? off() : (stop = off)));
    return () => {
      gone = true;
      stop?.();
    };
  }, []);

  async function ask() {
    setBusy(true);
    setProgress({ done: 0, total: friends.length });
    try {
      // A run that was stopped part way picks up where it left off; a finished one starts over.
      const ran = await backend.fetchMutuals(!known || known.asked >= friends.length);
      onDone(`${endWords[ran.ended] ?? "Done."} Asked about ${ran.done} of ${ran.total} friends.${ran.problem ? ` ${ran.problem}` : ""}`);
    } catch (err) {
      onDone(`That did not work: ${errorText(err)}`);
    } finally {
      setBusy(false);
      setProgress(null);
      void load();
    }
  }

  const idList = useMemo(() => friends.map((f) => f.id).sort().join(","), [friends]);
  // The stage's own size, kept in step by an observer. It is as wide as its column and as
  // tall as the list beside it, so the two end on one line.
  const [size, setSize] = useState({ w: 0, h: 0 });
  const shape = size.w > 0 && size.h > 0 ? Math.round((size.w / size.h) * 5) / 5 : 1.6;
  const town = useMemo(() => plan(friends, known?.pairs ?? [], shape), [known, idList, shape]); // eslint-disable-line react-hooks/exhaustive-deps
  const { byId, near, districts, seat, bridges, reach, loners } = town;

  const shownNow = districts.length > 0;
  useEffect(() => {
    const el = stage.current;
    if (!el) return;
    const read = () => setSize((old) => (old.w === el.clientWidth && old.h === el.clientHeight ? old : { w: el.clientWidth, h: el.clientHeight }));
    read();
    const watch = new ResizeObserver(read);
    watch.observe(el);
    return () => watch.disconnect();
  }, [shownNow]);
  // The whole town in view, a little bigger than life at most.
  const fit = () => {
    if (size.w <= 0 || size.h <= 0 || districts.length === 0) return;
    const k = Math.min(size.w / town.width, size.h / town.height, 1.3);
    setView({ k, x: (size.w - town.width * k) / 2, y: (size.h - town.height * k) / 2 });
  };
  // The wheel zooms the town and nothing else; the page behind it stays where it is.
  const shown = districts.length > 0;
  useEffect(() => {
    const el = stage.current;
    if (!el) return;
    const turn = (e: WheelEvent) => {
      e.preventDefault();
      moved.current = true;
      const r = el.getBoundingClientRect();
      // The whole window can be zoomed in Settings; the pointer comes in zoomed pixels.
      const zoom = r.width / el.offsetWidth || 1;
      const [px, py] = [(e.clientX - r.left) / zoom, (e.clientY - r.top) / zoom];
      setView((v) => {
        const k = Math.min(2.4, Math.max(0.25, v.k * (e.deltaY > 0 ? 1 / 1.15 : 1.15)));
        return { k, x: px - ((px - v.x) / v.k) * k, y: py - ((py - v.y) / v.k) * k };
      });
    };
    el.addEventListener("wheel", turn, { passive: false });
    return () => el.removeEventListener("wheel", turn);
  }, [shown]);
  /** True once I have dragged or zoomed; from then on the town stays where I put it. */
  const moved = useRef(false);
  // Fitted again whenever the stage or the town changes size, unless I have moved it myself.
  useLayoutEffect(() => {
    if (!moved.current) fit();
  }, [size.w, size.h, town.width, town.height, districts.length]); // eslint-disable-line react-hooks/exhaustive-deps

  function goTo(id: string) {
    const s = seat.get(id);
    const el = stage.current;
    setPicked(id);
    setFind("");
    if (!s || !el) return;
    setZone(-1);
    const k = Math.max(view.k, 1);
    setView({ k, x: el.clientWidth / 2 - s.x * k, y: el.clientHeight / 2 - s.y * k });
  }

  function showDistrict(d: District) {
    const el = stage.current;
    setZone(zone === d.index ? -1 : d.index);
    setPicked("");
    if (!el || zone === d.index) return;
    const k = Math.min(el.clientWidth / (2 * d.r + 160), el.clientHeight / (2 * d.r + 160), 1.4);
    setView({ k, x: el.clientWidth / 2 - d.x * k, y: el.clientHeight / 2 - d.y * k });
  }

  const focus = hover || picked;
  const knows = useMemo(() => new Set(focus ? near.get(focus) ?? [] : []), [focus, near]);
  const friendships = town.links.length;
  const top = districts[0]?.members[0];
  const holders = useMemo(() => [...reach.entries()].filter(([, n]) => n > 0).sort(([a, x], [b, y]) => y - x || near.get(b)!.length - near.get(a)!.length).slice(0, 6), [reach, near]);
  const matches = find.trim() ? friends.filter((f) => f.displayName.toLowerCase().includes(find.trim().toLowerCase())).slice(0, 7) : [];
  const chosen = picked ? byId.get(picked) : undefined;
  const nameOf = (id: string) => byId.get(id)?.displayName ?? "A friend";
  const asked = !!known && known.asked > 0;

  return (
    <div className="town-wrap">
      <div className="town-head">
        {asked ? (
          <dl className="town-facts">
            <div>
              <dt>Circles</dt>
              <dd>{districts.length}</dd>
            </div>
            <div>
              <dt>Friendships among your friends</dt>
              <dd>{friendships.toLocaleString()}</dd>
            </div>
            <div>
              <dt>Knows the most of them</dt>
              <dd className="town-fact-name hide-me">{top ? nameOf(top) : "Nobody yet"}</dd>
            </div>
            <div>
              <dt>Know none of the others</dt>
              <dd>{loners.length}</dd>
            </div>
          </dl>
        ) : (
          <p className="town-intro">
            This draws which of your friends are friends with each other. The app has to ask VRChat about each friend, one at a time with a pause in
            between, so {friends.length} friends take about {Math.max(1, Math.round((friends.length * 0.8) / 60))} min. It only happens when you press the
            button, and the answer stays on this PC.
          </p>
        )}
        <div className="town-actions">
          {asked && known?.at && <span className="town-when">Asked {daysAgo(known.at)}</span>}
          {busy ? (
            <button className="plain" onClick={() => void backend.stopMutuals()}>
              Stop
            </button>
          ) : (
            <button className={asked ? "plain" : "plain go"} disabled={friends.length < 2} onClick={() => void ask()}>
              {asked ? "Ask again" : "Ask VRChat"}
            </button>
          )}
        </div>
      </div>

      {busy && (
        <div className="web-progress" role="status">
          <div className="cache-meter">
            <span style={{ width: `${progress && progress.total ? Math.max(2, (progress.done / progress.total) * 100) : 2}%` }} />
          </div>
          <span>{progress ? `Asked about ${progress.done} of ${progress.total} friends` : "Asking"}</span>
        </div>
      )}

      {asked && districts.length === 0 && !busy && (
        <p className="empty">
          {known && known.hidden.length >= known.asked
            ? "VRChat gave no answer about any of your friends. Press Ask again; if it stays like this, the note at the bottom of the window says what VRChat answered."
            : "None of your friends are friends with each other, as far as VRChat lets the app see."}
        </p>
      )}

      {districts.length > 0 && (
        <div className="town-layout">
          <div
            className={`town-stage${focus ? " town-focused" : ""}${zone >= 0 ? " town-zoned" : ""}`}
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
          >
            <div className="town-world" style={{ width: town.width, height: town.height, transform: `translate(${view.x}px, ${view.y}px) scale(${view.k})` }}>
              <svg className="town-lines" width={town.width} height={town.height} aria-hidden="true">
                {[...bridges.entries()].map(([key, count]) => {
                  const [a, b] = key.split("-").map((n) => districts[Number(n)]);
                  const length = Math.hypot(b.x - a.x, b.y - a.y) || 1;
                  const [ux, uy] = [(b.x - a.x) / length, (b.y - a.y) / length];
                  const [x1, y1, x2, y2] = [a.x + ux * a.r, a.y + uy * a.r, b.x - ux * b.r, b.y - uy * b.r];
                  // The bridge bows a little, so two bridges between neighbours do not lie on each other.
                  const [mx, my] = [(x1 + x2) / 2 - uy * 34, (y1 + y2) / 2 + ux * 34];
                  return (
                    <g key={key} className={`town-bridge${zone >= 0 && (a.index === zone || b.index === zone) ? " on" : ""}`}>
                      <path d={`M${x1} ${y1} Q${mx} ${my} ${x2} ${y2}`} strokeWidth={2 + Math.min(8, count * 1.2)} />
                      <text x={(x1 + 2 * mx + x2) / 4} y={(y1 + 2 * my + y2) / 4 + 4}>
                        {count}
                      </text>
                    </g>
                  );
                })}
                {town.links.map(([a, b]) => {
                  const [p, q] = [seat.get(a)!, seat.get(b)!];
                  const lit = !!focus && (a === focus || b === focus);
                  // Friendships inside a district are always drawn; ones that cross only for the friend looked at.
                  if (p.district !== q.district && !lit) return null;
                  return <line key={`${a}:${b}`} x1={p.x} y1={p.y} x2={q.x} y2={q.y} className={lit ? "on" : ""} style={{ stroke: lit ? undefined : districts[p.district].colour }} />;
                })}
              </svg>

              {districts.map((d) => (
                <div
                  key={d.index}
                  className={`town-district${zone === d.index ? " on" : ""}`}
                  style={{ left: d.x - d.r, top: d.y - d.r, width: 2 * d.r, height: 2 * d.r, "--c": d.colour } as CSSProperties}
                >
                  <button className="town-plate hide-me" onClick={() => !drag.current?.moved && showDistrict(d)} title="Look at this circle">
                    {d.name}
                    <i>{d.members.length}</i>
                  </button>
                </div>
              ))}

              {districts.flatMap((d) =>
                d.members.map((id) => {
                  const f = byId.get(id);
                  const s = seat.get(id)!;
                  const lit = id === focus || knows.has(id);
                  return (
                    <button
                      key={id}
                      className={`town-face${lit ? " on" : ""}${id === picked ? " picked" : ""}${zone >= 0 && s.district !== zone ? " off" : ""}${(reach.get(id) ?? 0) > 0 ? " town-holder" : ""}`}
                      style={{ left: s.x, top: s.y, "--c": d.colour } as CSSProperties}
                      title={`${nameOf(id)}: friends with ${near.get(id)!.length} of your friends`}
                      onPointerEnter={() => setHover(id)}
                      onPointerLeave={() => setHover("")}
                      onFocus={() => setHover(id)}
                      onBlur={() => setHover("")}
                      onClick={() => !drag.current?.moved && setPicked(picked === id ? "" : id)}
                    >
                      <Avatar name={nameOf(id)} image={f?.image ?? ""} tone={f ? statusTone(f) : undefined} size={FACE} />
                      <span className="town-name hide-me">{nameOf(id)}</span>
                    </button>
                  );
                }),
              )}
            </div>

            <div className="town-zoom">
              <button className="plain" onClick={() => setView((v) => ({ ...v, k: Math.min(2.4, v.k * 1.2) }))} aria-label="Closer" title="Closer">
                +
              </button>
              <button className="plain" onClick={() => setView((v) => ({ ...v, k: Math.max(0.25, v.k / 1.2) }))} aria-label="Further away" title="Further away">
                −
              </button>
              <button
                className="plain"
                onClick={() => {
                  setZone(-1);
                  moved.current = false;
                  fit();
                }}
              >
                Whole town
              </button>
            </div>
          </div>

          <aside className="town-rail" aria-label="Circles and people">
            <div className="town-find">
              <input type="search" value={find} placeholder="Find a friend" aria-label="Find a friend" onChange={(e) => setFind(e.target.value)} />
              {matches.length > 0 && (
                <ul>
                  {matches.map((f) => (
                    <li key={f.id}>
                      <button onClick={() => goTo(f.id)}>
                        <Avatar name={f.displayName} image={f.image} size={26} />
                        <span className="hide-me">{f.displayName}</span>
                        {!seat.has(f.id) && <small>knows none of the others</small>}
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </div>

            {chosen && (
              <section className="town-card town-person">
                <header>
                  <Avatar name={chosen.displayName} image={chosen.image} tone={statusTone(chosen)} size={48} />
                  <div>
                    <b className="hide-me">{chosen.displayName}</b>
                    <span>
                      {seat.has(chosen.id)
                        ? `Friends with ${near.get(chosen.id)!.length} of your friends, in ${districts[seat.get(chosen.id)!.district].name}`
                        : "Knows none of your other friends"}
                    </span>
                  </div>
                </header>
                {(near.get(chosen.id)?.length ?? 0) > 0 && (
                  <ul className="town-people">
                    {[...near.get(chosen.id)!].sort((a, b) => nameOf(a).localeCompare(nameOf(b))).map((id) => (
                      <li key={id}>
                        <button onClick={() => goTo(id)} style={{ "--c": districts[seat.get(id)!.district].colour } as CSSProperties}>
                          <i aria-hidden="true" />
                          <span className="hide-me">{nameOf(id)}</span>
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
                <div className="town-card-actions">
                  <button className="plain go" onClick={() => openProfile(chosen.id)}>
                    Open profile
                  </button>
                  <button className="plain" onClick={() => setPicked("")}>
                    Done
                  </button>
                </div>
              </section>
            )}

            <section className="town-card">
              <h3>Circles</h3>
              <ul className="town-list">
                {districts.map((d) => (
                  <li key={d.index}>
                    <button className={zone === d.index ? "on" : ""} onClick={() => showDistrict(d)} style={{ "--c": d.colour } as CSSProperties}>
                      <i aria-hidden="true" />
                      <span className="hide-me">{d.name}</span>
                      <small>{d.members.length}</small>
                    </button>
                  </li>
                ))}
              </ul>
            </section>

            {holders.length > 0 && (
              <section className="town-card">
                <h3>Hold circles together</h3>
                <p className="town-note">Friends who know people in more than one circle. They wear a white ring on the map.</p>
                <ul className="town-list">
                  {holders.map(([id, n]) => (
                    <li key={id}>
                      <button onClick={() => goTo(id)} style={{ "--c": districts[seat.get(id)!.district].colour } as CSSProperties}>
                        <i aria-hidden="true" />
                        <span className="hide-me">{nameOf(id)}</span>
                        <small>{n + 1} circles</small>
                      </button>
                    </li>
                  ))}
                </ul>
              </section>
            )}

            {(loners.length > 0 || (known?.hidden.length ?? 0) > 0) && (
              <section className="town-card">
                <h3>Not on the map</h3>
                <p className="town-note">
                  {loners.length > 0 && `${loners.length} ${loners.length === 1 ? "friend knows" : "friends know"} none of your other friends. `}
                  {(known?.hidden.length ?? 0) > 0 && `${known!.hidden.length} keep their friends hidden, so only what others said about them shows.`}
                </p>
              </section>
            )}
          </aside>
        </div>
      )}
      {districts.length > 0 && (
        <p className="setting-hint town-hint">
          Drag to move, scroll to zoom. Point at a friend to light up who they know; press them for the list. A number on a bridge counts the friendships
          between two circles.
        </p>
      )}
    </div>
  );
}
