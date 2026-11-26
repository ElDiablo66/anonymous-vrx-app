import { useEffect, useState } from "react";
import { backend } from "./backend";
import { daysAgo } from "./bits";

/** What I know about worlds myself: my stars, and how often the game log saw me there. */
export interface WorldMarks {
  ratings: Record<string, number>;
  visits: Record<string, [number, string]>;
}

let known: WorldMarks | null = null;
const CHANGED = "marks:changed";

async function read(): Promise<WorldMarks> {
  const [ratings, visits] = await Promise.all([backend.ratings().catch(() => ({})), backend.worldVisits().catch(() => ({}))]);
  known = { ratings, visits };
  return known;
}

/** My stars and visits, read once and kept; every place that shows them changes together. */
export function useWorldMarks(): WorldMarks {
  const [marks, setMarks] = useState<WorldMarks>(known ?? { ratings: {}, visits: {} });
  useEffect(() => {
    let alive = true;
    const load = () => void read().then((m) => alive && setMarks(m));
    if (!known) load();
    window.addEventListener(CHANGED, load);
    return () => {
      alive = false;
      window.removeEventListener(CHANGED, load);
    };
  }, []);
  return marks;
}

/** Five stars to click. Clicking the star that is already the rating takes the rating away. */
export function Stars({ id, onDone }: { id: string; onDone: (message: string) => void }) {
  const marks = useWorldMarks();
  const now = marks.ratings[id] ?? 0;
  const [busy, setBusy] = useState(false);
  async function set(stars: number) {
    setBusy(true);
    try {
      await backend.setRating(id, stars === now ? 0 : stars);
      window.dispatchEvent(new Event(CHANGED));
    } catch {
      onDone("The rating was not saved.");
    } finally {
      setBusy(false);
    }
  }
  return (
    <span className="stars" role="radiogroup" aria-label="My rating">
      {[1, 2, 3, 4, 5].map((n) => (
        <button key={n} role="radio" aria-checked={now === n} aria-label={`${n} ${n === 1 ? "star" : "stars"}`} className={n <= now ? "on" : ""} disabled={busy} onClick={() => void set(n)}>
          ★
        </button>
      ))}
    </span>
  );
}

/** "Been here 3 times, last 5 days ago", or that the log has no visit. */
export function beenWords(visit: [number, string] | undefined): string {
  if (!visit) return "Not been here, as far as VRChat's log on this PC shows.";
  return `Been here ${visit[0] === 1 ? "once" : `${visit[0]} times`}, last ${daysAgo(visit[1])}.`;
}

/** Small marks for a list row: my stars and whether I have been there. */
export function MarkBadges({ id, marks }: { id: string; marks: WorldMarks }) {
  const stars = marks.ratings[id];
  const visit = marks.visits[id];
  if (!stars && !visit) return null;
  return (
    <span className="my-marks">
      {stars ? <span className="mark mark-stars" title="My rating">{"★".repeat(stars)}</span> : null}
      {visit ? <span className="mark" title={beenWords(visit)}>been here</span> : null}
    </span>
  );
}
