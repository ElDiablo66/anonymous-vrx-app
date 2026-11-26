import { useEffect, useMemo, useRef, useState } from "react";
import { backend, errorText, type Friend } from "./backend";
import { daysAgo } from "./bits";
import { openProfile } from "./Profile";

interface Known {
  pairs: Array<[string, string]>;
  asked: number;
  hidden: string[];
  at: string;
  running: boolean;
}

interface Spot {
  id: string;
  x: number;
  y: number;
  links: number;
  circle: number;
}

const colours = ["#ff8a4c", "#8fd45f", "#5b8cff", "#f0596b", "#b18cff", "#2de2e6", "#ffd166", "#ff7ac8"];

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

/**
 * Lays friends out so that friends of each other end up close: every two push each other
 * away, a friendship pulls its two ends together, and everything leans towards the middle.
 * The same friends always land in the same place.
 */
function layOut(ids: string[], pairs: Array<[string, string]>): Spot[] {
  const index = new Map(ids.map((id, i) => [id, i]));
  const near = new Map<string, string[]>(ids.map((id) => [id, []]));
  const links: Array<[number, number]> = [];
  for (const [a, b] of pairs) {
    const [i, j] = [index.get(a), index.get(b)];
    if (i === undefined || j === undefined) continue;
    links.push([i, j]);
    near.get(a)!.push(b);
    near.get(b)!.push(a);
  }
  const n = ids.length;
  const spread = 60 * Math.sqrt(n);
  const x = ids.map((_, i) => Math.cos(i * 2.399963) * spread * Math.sqrt((i + 1) / n));
  const y = ids.map((_, i) => Math.sin(i * 2.399963) * spread * Math.sqrt((i + 1) / n));
  const rounds = n > 600 ? 90 : n > 250 ? 180 : 320;
  const ideal = 70;
  for (let round = 0; round < rounds; round++) {
    const heat = 34 * (1 - round / rounds) + 1;
    const dx = new Float64Array(n);
    const dy = new Float64Array(n);
    for (let i = 0; i < n; i++) {
      for (let j = i + 1; j < n; j++) {
        let ux = x[i] - x[j];
        let uy = y[i] - y[j];
        let d2 = ux * ux + uy * uy;
        if (d2 < 0.01) {
          ux = 0.1 * (i - j);
          uy = 0.1;
          d2 = ux * ux + uy * uy;
        }
        const push = (ideal * ideal) / d2;
        dx[i] += ux * push;
        dy[i] += uy * push;
        dx[j] -= ux * push;
        dy[j] -= uy * push;
      }
    }
    for (const [i, j] of links) {
      const ux = x[i] - x[j];
      const uy = y[i] - y[j];
      const pull = Math.sqrt(ux * ux + uy * uy) / ideal;
      dx[i] -= ux * pull;
      dy[i] -= uy * pull;
      dx[j] += ux * pull;
      dy[j] += uy * pull;
    }
    for (let i = 0; i < n; i++) {
      // Friends with nobody in common drift to the edge instead of flying off.
      dx[i] -= x[i] * 0.45;
      dy[i] -= y[i] * 0.45;
      const length = Math.hypot(dx[i], dy[i]) || 1;
      const step = Math.min(length, heat);
      x[i] += (dx[i] / length) * step;
      y[i] += (dy[i] / length) * step;
    }
  }
  const circle = circles(ids, near);
  return ids.map((id, i) => ({ id, x: x[i], y: y[i], links: near.get(id)!.length, circle: near.get(id)!.length ? circle.get(id)! : -1 }));
}

const endWords: Record<string, string> = {
  done: "Done.",
  stopped: "Stopped. What was found so far is kept.",
  slow: "VRChat asked to slow down, so it stopped. What was found so far is kept; try again later.",
  signedOut: "You are signed out, so it stopped.",
  network: "VRChat could not be reached, so it stopped. What was found so far is kept.",
};

/**
 * Which of my friends are friends with each other, as a web: a dot per friend, a line per
 * friendship between two of them, circles of friends in one colour.
 */
export function FriendWeb({ friends, onDone }: { friends: Friend[]; onDone: (message: string) => void }) {
  const [known, setKnown] = useState<Known | null>(null);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [busy, setBusy] = useState(false);
  const [hover, setHover] = useState("");
  const [picked, setPicked] = useState("");
  const [view, setView] = useState<{ x: number; y: number; w: number; h: number } | null>(null);
  const svg = useRef<SVGSVGElement>(null);
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

  async function fetchAll() {
    setBusy(true);
    setProgress({ done: 0, total: friends.length });
    try {
      const ran = await backend.fetchMutuals();
      onDone(`${endWords[ran.ended] ?? "Done."} Asked about ${ran.done} of ${ran.total} friends.`);
    } catch (err) {
      onDone(`That did not work: ${errorText(err)}`);
    } finally {
      setBusy(false);
      setProgress(null);
      void load();
    }
  }

  const byId = useMemo(() => new Map(friends.map((f) => [f.id, f])), [friends]);
  const idList = useMemo(() => friends.map((f) => f.id).sort().join(","), [friends]);
  const spots = useMemo(() => {
    if (!known || known.pairs.length === 0) return [];
    return layOut(idList ? idList.split(",") : [], known.pairs);
  }, [known, idList]);
  const at = useMemo(() => new Map(spots.map((s) => [s.id, s])), [spots]);
  const lines = useMemo(() => (known?.pairs ?? []).filter(([a, b]) => at.has(a) && at.has(b)), [known, at]);
  const whole = useMemo(() => {
    if (spots.length === 0) return { x: -300, y: -200, w: 600, h: 400 };
    // The picture opens on the friends who know someone; the rest sit around them, a scroll away.
    const shown = spots.some((s) => s.links > 0) ? spots.filter((s) => s.links > 0) : spots;
    const xs = shown.map((s) => s.x);
    const ys = shown.map((s) => s.y);
    const [x0, x1, y0, y1] = [Math.min(...xs) - 90, Math.max(...xs) + 160, Math.min(...ys) - 60, Math.max(...ys) + 60];
    return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
  }, [spots]);
  useEffect(() => setView(null), [whole]);
  const box = view ?? whole;
  // Dots and names keep their size on screen however far the picture is zoomed.
  const unit = box.w / 1000;

  // Names are written for the best connected, and for whoever is pointed at or picked.
  const named = useMemo(() => new Set([...spots].sort((a, b) => b.links - a.links).slice(0, 10).filter((s) => s.links > 0).map((s) => s.id)), [spots]);
  const focus = hover || picked;
  const nextTo = useMemo(() => {
    const out = new Set<string>();
    if (focus) for (const [a, b] of lines) if (a === focus || b === focus) out.add(a === focus ? b : a);
    return out;
  }, [focus, lines]);

  function zoom(e: React.WheelEvent) {
    const el = svg.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const factor = e.deltaY > 0 ? 1.18 : 1 / 1.18;
    const w = Math.min(whole.w * 3, Math.max(120, box.w * factor));
    const h = (w / box.w) * box.h;
    const px = box.x + ((e.clientX - r.left) / r.width) * box.w;
    const py = box.y + ((e.clientY - r.top) / r.height) * box.h;
    setView({ x: px - ((e.clientX - r.left) / r.width) * w, y: py - ((e.clientY - r.top) / r.height) * h, w, h });
  }

  function goTo(id: string) {
    setPicked(id);
    const s = at.get(id);
    if (s) setView({ x: s.x - 260, y: s.y - 170, w: 520, h: 340 });
  }

  const linked = spots.filter((s) => s.links > 0).length;
  const sorted = useMemo(() => [...friends].sort((a, b) => a.displayName.localeCompare(b.displayName)), [friends]);

  return (
    <div className="web-wrap">
      <div className="web-top">
        <div className="web-facts">
          {known && known.asked > 0 ? (
            <p>
              <b>{linked}</b> of your <b>{friends.length}</b> friends know at least one other friend of yours. <b>{lines.length}</b> friendships between them.
              {known.at && ` Asked ${daysAgo(known.at)}.`}
              {known.hidden.length > 0 && ` ${known.hidden.length} keep their friends hidden.`}
            </p>
          ) : (
            <p>
              This draws which of your friends are friends with each other. The app has to ask VRChat about each friend, one at a time with a pause in
              between, so {friends.length} friends take about {Math.max(1, Math.round((friends.length * 1.5) / 60))} min. It only happens when you press the
              button, and the answer is kept on this PC.
            </p>
          )}
        </div>
        <div className="web-actions">
          {spots.length > 0 && (
            <select value={picked} onChange={(e) => (e.target.value ? goTo(e.target.value) : (setPicked(""), setView(null)))} aria-label="Go to a friend">
              <option value="">Go to a friend</option>
              {sorted.map((f) => (
                <option key={f.id} value={f.id}>
                  {f.displayName}
                </option>
              ))}
            </select>
          )}
          {view && (
            <button className="plain" onClick={() => setView(null)}>
              Show all
            </button>
          )}
          {busy ? (
            <button className="plain" onClick={() => void backend.stopMutuals()}>
              Stop
            </button>
          ) : (
            <button className={known && known.asked > 0 ? "plain" : "plain go"} disabled={friends.length < 2} onClick={() => void fetchAll()}>
              {known && known.asked > 0 ? "Ask again" : "Ask VRChat"}
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

      {known && known.asked > 0 && spots.length === 0 && !busy && <p className="empty">None of your friends are friends with each other, as far as VRChat lets the app see.</p>}

      {spots.length > 0 && (
        <svg
          ref={svg}
          className={`web${focus ? " web-focused" : ""}`}
          viewBox={`${box.x} ${box.y} ${box.w} ${box.h}`}
          role="img"
          aria-label="Your friends, joined by a line where two of them are friends with each other"
          onWheel={zoom}
          onPointerDown={(e) => {
            drag.current = { x: e.clientX, y: e.clientY, moved: false };
          }}
          onPointerMove={(e) => {
            const d = drag.current;
            const el = svg.current;
            if (!d || !el || e.buttons !== 1) return;
            const r = el.getBoundingClientRect();
            const [mx, my] = [e.clientX - d.x, e.clientY - d.y];
            if (Math.abs(mx) + Math.abs(my) < 3 && !d.moved) return;
            drag.current = { x: e.clientX, y: e.clientY, moved: true };
            setView({ ...box, x: box.x - (mx / r.width) * box.w, y: box.y - (my / r.height) * box.h });
          }}
          onPointerUp={() => {
            window.setTimeout(() => (drag.current = null), 0);
          }}
        >
          <g className="web-lines">
            {lines.map(([a, b]) => {
              const [p, q] = [at.get(a)!, at.get(b)!];
              return <line key={`${a}:${b}`} x1={p.x} y1={p.y} x2={q.x} y2={q.y} className={focus && (a === focus || b === focus) ? "on" : ""} />;
            })}
          </g>
          {spots.map((s) => {
            const f = byId.get(s.id);
            const name = f?.displayName ?? "A friend";
            const size = (s.links === 0 ? 3 : 5 + Math.sqrt(s.links) * 2.4) * unit;
            const lit = s.id === focus || nextTo.has(s.id);
            const label = named.has(s.id) || lit;
            return (
              <g
                key={s.id}
                className={`web-dot${lit ? " on" : ""}${s.links === 0 ? " web-alone" : ""}`}
                transform={`translate(${s.x} ${s.y})`}
                onPointerEnter={() => setHover(s.id)}
                onPointerLeave={() => setHover("")}
                onClick={() => {
                  if (!drag.current?.moved) openProfile(s.id);
                }}
              >
                <title>{`${name}: friends with ${s.links} of your friends`}</title>
                <circle r={size} fill={s.circle < 0 ? "currentColor" : colours[s.circle % colours.length]} />
                {label && (
                  <text x={size + 5 * unit} y={4 * unit} fontSize={12 * unit} className="hide-me">
                    {name}
                  </text>
                )}
              </g>
            );
          })}
        </svg>
      )}
      {spots.length > 0 && <p className="setting-hint web-hint">Drag to move, scroll to zoom, point at a friend to see who they know, press to open the profile. A colour is a circle of friends who mostly know each other.</p>}
    </div>
  );
}
