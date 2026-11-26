import { useEffect, useRef, useState } from "react";
import { backend, errorText, type PhotoInfo } from "./backend";
import { clock, dayLabel, tidy } from "./bits";
import { openThing } from "./Detail";

const PAGE = 48;

/** The photos VRChat's camera saved on this PC, each with the world it was taken in and who was there. */
export function Photos({ onDone }: { onDone: (message: string) => void }) {
  const [photos, setPhotos] = useState<PhotoInfo[] | null>(null);
  const [total, setTotal] = useState(0);
  const [problem, setProblem] = useState("");
  const [busy, setBusy] = useState(false);
  const [open, setOpen] = useState<PhotoInfo | null>(null);
  /** How many photos there are before anything narrows the list. */
  const [everything, setEverything] = useState(0);
  const [typed, setTyped] = useState("");
  /** What is being searched for: the typed text once typing has paused. */
  const [query, setQuery] = useState("");
  const [until, setUntil] = useState("");
  /** Counts the searches, so a slow answer to an older one is not shown over a newer one. */
  const asked = useRef(0);

  async function load(skip: number) {
    const mine = ++asked.current;
    setBusy(true);
    setProblem("");
    try {
      const page = await backend.photos(PAGE, skip, query, until);
      if (mine !== asked.current) return;
      setTotal(page.total);
      setEverything(page.everything);
      setPhotos((had) => (skip === 0 || !had ? page.photos : [...had, ...page.photos]));
    } catch (err) {
      if (mine === asked.current) setProblem(errorText(err));
    } finally {
      if (mine === asked.current) setBusy(false);
    }
  }

  // Searching waits until typing has paused, then starts the list again from the top.
  useEffect(() => {
    const wait = window.setTimeout(() => setQuery(typed.trim()), 300);
    return () => window.clearTimeout(wait);
  }, [typed]);

  useEffect(() => {
    void load(0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query, until]);

  const narrowed = query !== "" || until !== "";
  const tools = (
    <div className="list-tools photo-tools">
      <input
        type="search"
        value={typed}
        onChange={(e) => setTyped(e.target.value)}
        placeholder="Find photos by who was there, or by world"
        aria-label="Find photos by who was there, or by world"
      />
      <label className="tool">
        Up to
        <input type="date" value={until} onChange={(e) => setUntil(e.target.value)} aria-label="Only photos up to this day" />
      </label>
      {narrowed && (
        <button
          className="plain"
          onClick={() => {
            setTyped("");
            setQuery("");
            setUntil("");
          }}
        >
          Show every photo
        </button>
      )}
    </div>
  );

  useEffect(() => {
    if (!open) return;
    const key = (e: KeyboardEvent) => e.key === "Escape" && setOpen(null);
    document.addEventListener("keydown", key);
    return () => document.removeEventListener("keydown", key);
  }, [open]);

  if (problem) {
    return (
      <p className="banner" role="alert">
        {problem}
      </p>
    );
  }
  if (photos === null) return <p className="empty">Looking through your VRChat photos</p>;
  if (photos.length === 0 && !narrowed) {
    return <p className="empty">No VRChat photos found in your Pictures folder. Photos taken with the camera in VRChat show up here.</p>;
  }
  if (photos.length === 0) {
    return (
      <div className="photos">
        {tools}
        <p className="empty">
          {query ? `No photo has "${query}" as its world or among the people in the instance` : "No photos"}
          {until ? ` up to ${until}` : ""}. Photos from before this app was installed have no world or people, so they cannot be found this way.
        </p>
      </div>
    );
  }

  let lastDay = "";
  return (
    <div className="photos">
      {tools}
      <p className="tile-note">
        {narrowed ? `${total} of ${everything} photos match.` : `${total} ${total === 1 ? "photo" : "photos"} in Pictures, VRChat.`} The world and the people come
        from this app's own history, so photos from before it was installed have none. A person is found when they were in the instance as the photo
        was taken, whether or not they are in the picture. Your files are not changed.
      </p>
      <ul className="photo-grid">
        {photos.map((p) => {
          const day = dayLabel(p.takenAt);
          const header = day !== lastDay ? <li className="day photo-day" key={`day-${p.path}`}>{day}</li> : null;
          lastDay = day;
          return [
            header,
            <li key={p.path}>
              <button className="photo" onClick={() => setOpen(p)} title="Show this photo">
                <img src={p.url} alt="" loading="lazy" decoding="async" />
                <span className="photo-line">
                  <span className="photo-world">{p.worldName ? tidy(p.worldName) : "World not recorded"}</span>
                  <span className="tile-note">
                    {clock(p.takenAt)}
                    {p.people.length > 0 && `, with ${p.people.length}`}
                  </span>
                </span>
              </button>
            </li>,
          ];
        })}
      </ul>
      {photos.length < total && (
        <button className="plain" disabled={busy} onClick={() => void load(photos.length)}>
          {busy ? "Loading" : `Show more (${total - photos.length} left)`}
        </button>
      )}

      {open && (
        <div className="veil" onClick={() => setOpen(null)}>
          <div className="photo-open" role="dialog" aria-modal="true" aria-label="Photo" onClick={(e) => e.stopPropagation()}>
            <img src={open.url} alt="" />
            <aside>
              <h3>{open.worldName ? tidy(open.worldName) : "World not recorded"}</h3>
              <p className="tile-note">
                {dayLabel(open.takenAt)}, {clock(open.takenAt)}
                {open.width > 0 && `, ${open.width} by ${open.height}`}
              </p>
              <div className="wayback">
                {open.worldId && <button onClick={() => openThing("world", open.worldId)}>Open the world</button>}
                <button
                  onClick={() =>
                    void backend.showPhoto(open.path).then(
                      () => undefined,
                      (err) => onDone(`That did not work: ${errorText(err)}`),
                    )
                  }
                >
                  Show in folder
                </button>
                <button onClick={() => setOpen(null)}>Close</button>
              </div>
              {open.people.length > 0 && (
                <>
                  <h3>In the instance then</h3>
                  <ul className="chips">
                    {open.people.map((who) => (
                      <li className="chip chip-small chip-open" key={who.userId || who.displayName} data-person={who.userId || undefined} data-person-name={who.displayName}>
                        <button
                          className="chip-find"
                          title="Show every photo taken while they were in the instance"
                          onClick={() => {
                            setOpen(null);
                            setUntil("");
                            setTyped(who.displayName);
                            setQuery(who.displayName);
                          }}
                        >
                          <span className="chip-name">{who.displayName}</span>
                        </button>
                      </li>
                    ))}
                  </ul>
                </>
              )}
              <p className="tile-note">{open.file}</p>
            </aside>
          </div>
        </div>
      )}
    </div>
  );
}
