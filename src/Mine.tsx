import { useEffect, useState } from "react";
import { backend } from "./backend";
import { daysAgo } from "./bits";

/** What I know about worlds myself: my stars, and how often the game log saw me there. */
export interface WorldMarks {
  ratings: Record<string, number>;
  visits: Record<string, [number, string]>;
}

let known: WorldMarks | null = null;
// Another account signed in: the stars and visits read so far belong to the one before.
window.addEventListener("account:changed", () => {
  known = null;
});
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

/** The tag that takes something out of my lists until I ask for the hidden ones. */
export const HIDDEN = "hidden";
const isHidden = (tags: string[] | undefined) => !!tags?.some((t) => t.toLowerCase() === HIDDEN);

let knownTags: Record<string, string[]> | null = null;
const TAGS_CHANGED = "tags:changed";
window.addEventListener("account:changed", () => {
  knownTags = null;
});

/** My tags on avatars and worlds, read once and kept; every list that shows them changes together. */
export function useTags(): Record<string, string[]> {
  const [tags, setTags] = useState<Record<string, string[]>>(knownTags ?? {});
  useEffect(() => {
    let alive = true;
    const load = () =>
      void backend
        .tags()
        .then((t) => {
          knownTags = t;
          if (alive) setTags(t);
        })
        .catch(() => {});
    if (!knownTags) load();
    window.addEventListener(TAGS_CHANGED, load);
    return () => {
      alive = false;
      window.removeEventListener(TAGS_CHANGED, load);
    };
  }, []);
  return tags;
}

async function saveTags(id: string, next: string[], onDone: (message: string) => void) {
  try {
    await backend.setTags(id, next);
    window.dispatchEvent(new Event(TAGS_CHANGED));
  } catch (err) {
    onDone(`The tags were not saved: ${(err as { message?: string })?.message ?? "something went wrong"}`);
  }
}

/** The tags on one thing as small labels. Nothing is drawn when it has none. */
export function TagChips({ id, tags }: { id: string; tags: Record<string, string[]> }) {
  const mine = tags[id];
  if (!mine?.length) return null;
  return (
    <span className="my-marks">
      {mine.map((t) => (
        <span key={t} className={`mark${t.toLowerCase() === HIDDEN ? " mark-hidden" : ""}`}>
          {t}
        </span>
      ))}
    </span>
  );
}

/** Add and remove tags on one thing. Tags I used before are offered while typing. */
export function TagEditor({ id, tags, onDone }: { id: string; tags: Record<string, string[]>; onDone: (message: string) => void }) {
  const mine = tags[id] ?? [];
  const [text, setText] = useState("");
  const every = [...new Set(Object.values(tags).flat())].filter((t) => !mine.some((m) => m.toLowerCase() === t.toLowerCase())).sort((a, b) => a.localeCompare(b));
  const hidden = isHidden(mine);
  const listId = `tags-${id}`;
  return (
    <div className="tag-editor">
      <span className="my-marks">
        {mine.length === 0 && <span className="tile-note">No tags yet.</span>}
        {mine.map((t) => (
          <span key={t} className={`mark tag-on${t.toLowerCase() === HIDDEN ? " mark-hidden" : ""}`}>
            {t}
            <button aria-label={`Remove the tag ${t}`} title="Remove this tag" onClick={() => void saveTags(id, mine.filter((m) => m !== t), onDone)}>
              ×
            </button>
          </span>
        ))}
      </span>
      <form
        className="tag-add"
        onSubmit={(e) => {
          e.preventDefault();
          const add = text.split(",").map((t) => t.trim()).filter(Boolean);
          if (!add.length) return;
          setText("");
          void saveTags(id, [...mine, ...add], onDone);
        }}
      >
        <input value={text} maxLength={60} list={listId} placeholder="Add a tag, for example club or sleep" aria-label="Add a tag" onChange={(e) => setText(e.target.value)} />
        <datalist id={listId}>
          {every.map((t) => (
            <option key={t} value={t} />
          ))}
        </datalist>
        <button className="plain go" disabled={!text.trim()}>
          Add
        </button>
        <button
          type="button"
          className="plain"
          title={hidden ? "Show it in my lists again" : "Take it out of my lists. It is still on VRChat and in the Hidden filter"}
          onClick={() => void saveTags(id, hidden ? mine.filter((m) => m.toLowerCase() !== HIDDEN) : [...mine, HIDDEN], onDone)}
        >
          {hidden ? "Unhide" : "Hide"}
        </button>
      </form>
    </div>
  );
}

/** What the tag filter can be set to besides a tag's own name. */
export const ALL_TAGS = "";
export const UNTAGGED = "~untagged";

/** True when the thing belongs in the list for the chosen filter. Hidden things only show under Hidden. */
export function passesTag(id: string, tags: Record<string, string[]>, filter: string): boolean {
  const mine = tags[id] ?? [];
  if (filter.toLowerCase() === HIDDEN) return isHidden(mine);
  if (isHidden(mine)) return false;
  if (filter === ALL_TAGS) return true;
  if (filter === UNTAGGED) return mine.length === 0;
  return mine.some((t) => t.toLowerCase() === filter.toLowerCase());
}

/** A row of filters: everything, each tag in use among `ids` with its count, untagged, hidden. */
export function TagFilter({ ids, tags, value, set }: { ids: string[]; tags: Record<string, string[]>; value: string; set: (next: string) => void }) {
  const counts = new Map<string, number>();
  let untagged = 0;
  let hidden = 0;
  for (const id of ids) {
    const mine = tags[id] ?? [];
    if (isHidden(mine)) {
      hidden += 1;
      continue;
    }
    if (mine.length === 0) untagged += 1;
    for (const t of mine) counts.set(t, (counts.get(t) ?? 0) + 1);
  }
  if (counts.size === 0 && hidden === 0) return null;
  const names = [...counts.keys()].sort((a, b) => a.localeCompare(b));
  const pill = (id: string, label: string, n?: number) => (
    <button key={id} className={value === id ? "on" : ""} aria-pressed={value === id} onClick={() => set(id)}>
      {label}
      {n !== undefined && <span className="chip-count">{n}</span>}
    </button>
  );
  return (
    <div className="filters filters-flat" role="group" aria-label="My tags">
      {pill(ALL_TAGS, "Everything")}
      {names.map((t) => pill(t, t, counts.get(t)))}
      {untagged > 0 && pill(UNTAGGED, "No tag", untagged)}
      {hidden > 0 && pill(HIDDEN, "Hidden", hidden)}
    </div>
  );
}
