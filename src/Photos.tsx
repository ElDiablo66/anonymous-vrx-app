import { useEffect, useState } from "react";
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

  async function load(skip: number) {
    setBusy(true);
    setProblem("");
    try {
      const page = await backend.photos(PAGE, skip);
      setTotal(page.total);
      setPhotos((had) => (skip === 0 || !had ? page.photos : [...had, ...page.photos]));
    } catch (err) {
      setProblem(errorText(err));
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    void load(0);
  }, []);

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
  if (photos.length === 0) {
    return <p className="empty">No VRChat photos found in your Pictures folder. Photos taken with the camera in VRChat show up here.</p>;
  }

  let lastDay = "";
  return (
    <div className="photos">
      <p className="tile-note">
        {total} {total === 1 ? "photo" : "photos"} in Pictures, VRChat. The world and the people come from this app's own history, so photos from before
        it was installed have none. Your files are not changed.
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
                      <li className="chip chip-small" key={who.userId || who.displayName} data-person={who.userId || undefined} data-person-name={who.displayName}>
                        <span className="chip-name">{who.displayName}</span>
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
