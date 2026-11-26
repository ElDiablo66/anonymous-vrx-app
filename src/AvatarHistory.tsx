import { useEffect, useState } from "react";
import { backend, errorText, type AvatarWorn } from "./backend";
import { clock, dayLabel, tidy } from "./bits";
import { openThing } from "./Detail";
import { HIDDEN, TagChips, TagEditor, useTags } from "./Mine";
import { openPicture } from "./Viewer";

export function spent(seconds: number): string {
  const minutes = Math.round(seconds / 60);
  if (minutes < 1) return "under a minute";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours} h ${minutes % 60} min`;
  return `${Math.floor(hours / 24)} days ${hours % 24} h`;
}

/** My wardrobe: every avatar I wore while the app was running, as a board to pick from or as a list with the numbers. */
export function AvatarHistory({ onDone }: { onDone: (message: string) => void }) {
  const [list, setList] = useState<AvatarWorn[] | null>(null);
  const [busy, setBusy] = useState("");
  const [order, setOrder] = useState<"recent" | "worn" | "name">("recent");
  const [look, setLook] = useState<"board" | "list">(() => {
    try {
      return localStorage.getItem("wardrobe-look") === "list" ? "list" : "board";
    } catch {
      return "board";
    }
  });
  const [words, setWords] = useState("");
  const [picked, setPicked] = useState<string[]>([]);
  const [editing, setEditing] = useState("");
  const tags = useTags();

  useEffect(() => {
    let alive = true;
    const load = () =>
      void backend
        .avatarHistory(300)
        .then((rows) => alive && setList(rows))
        .catch(() => alive && setList([]));
    load();
    let stop: (() => void) | undefined;
    // The listener may only be ready after this tab is closed; then it is taken off at once.
    void backend.on("avatars:changed", load).then((off) => (alive ? (stop = off) : off()));
    return () => {
      alive = false;
      stop?.();
    };
  }, []);

  async function act(id: string, job: () => Promise<unknown>, done: string) {
    setBusy(id);
    try {
      await job();
      onDone(done);
    } catch (err) {
      onDone(`That did not work: ${errorText(err)}`);
    } finally {
      setBusy("");
    }
  }

  const wearing = list?.[0]?.avatarId ?? "";
  const sorted =
    order === "worn" && list
      ? [...list].sort((a, b) => b.secondsInGame - a.secondsInGame)
      : order === "name" && list
        ? [...list].sort((a, b) => tidy(a.name).localeCompare(tidy(b.name)))
        : (list ?? []);
  // The tags on avatars in the wardrobe, most used first, to narrow the board with.
  const tagCounts = new Map<string, number>();
  for (const a of list ?? []) for (const t of tags[a.avatarId] ?? []) tagCounts.set(t, (tagCounts.get(t) ?? 0) + 1);
  const offered = [...tagCounts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  const wanted = picked.filter((t) => tagCounts.has(t));
  const q = words.trim().toLowerCase();
  const hides = (id: string) => (tags[id] ?? []).some((t) => t.toLowerCase() === HIDDEN);
  const hiddenOnes = (list ?? []).filter((a) => hides(a.avatarId)).length;
  // An avatar I tagged "hidden" stays out unless that tag is the one picked. The one whose tags I am editing stays put.
  const shown = sorted.filter(
    (a) =>
      a.avatarId === editing ||
      ((wanted.some((t) => t.toLowerCase() === HIDDEN) || !hides(a.avatarId)) &&
      (!q || tidy(a.name).toLowerCase().includes(q) || a.authorName.toLowerCase().includes(q) || (tags[a.avatarId] ?? []).some((t) => t.toLowerCase().includes(q))) &&
      wanted.every((t) => (tags[a.avatarId] ?? []).includes(t))),
  );
  const pickLook = (next: "board" | "list") => {
    setLook(next);
    try {
      localStorage.setItem("wardrobe-look", next);
    } catch {
      // The choice then lasts until the app closes.
    }
  };
  const total = (list ?? []).reduce((sum, a) => sum + a.secondsInGame, 0);
  // Each bar is that avatar's share of all the time spent in worlds, so the bars add up to one.
  const all = (list ?? []).reduce((sum, a) => sum + a.secondsInGame, 0);

  if (list === null) return <p className="empty">Reading your avatar history</p>;
  if (list.length === 0) {
    return (
      <p className="empty">
        Your wardrobe is empty for now. Every avatar you switch to while this app runs lands here, ready to wear again
        with one press, with how often and how long you wore it.
      </p>
    );
  }

  return (
    <>
      <div className="list-tools wardrobe-tools">
        <input type="search" value={words} onChange={(e) => setWords(e.target.value)} placeholder="Find by name, maker or tag" aria-label="Find an avatar in the wardrobe" />
        <div className="filters filters-flat" role="radiogroup" aria-label="Show as">
          <button role="radio" aria-checked={look === "board"} className={look === "board" ? "on" : ""} onClick={() => pickLook("board")}>
            Board
          </button>
          <button role="radio" aria-checked={look === "list"} className={look === "list" ? "on" : ""} onClick={() => pickLook("list")}>
            List
          </button>
        </div>
        <div className="filters filters-flat" role="radiogroup" aria-label="Order">
          <button role="radio" aria-checked={order === "recent"} className={order === "recent" ? "on" : ""} onClick={() => setOrder("recent")}>
            Latest first
          </button>
          <button role="radio" aria-checked={order === "worn"} className={order === "worn" ? "on" : ""} onClick={() => setOrder("worn")}>
            Most worn
          </button>
          <button role="radio" aria-checked={order === "name"} className={order === "name" ? "on" : ""} onClick={() => setOrder("name")}>
            By name
          </button>
        </div>
        <span className="tile-note">
          {list.length} {list.length === 1 ? "avatar" : "avatars"}, {spent(total)} worn in VRChat since this app started keeping track.
          {hiddenOnes > 0 ? ` ${hiddenOnes} tagged hidden ${hiddenOnes === 1 ? "is" : "are"} left out; pick that tag to see ${hiddenOnes === 1 ? "it" : "them"}.` : ""}
        </span>
      </div>
      {offered.length > 0 && (
        <div className="wardrobe-tags" role="group" aria-label="Only avatars with these tags">
          {offered.map(([t, n]) => {
            const on = wanted.includes(t);
            return (
              <button key={t} aria-pressed={on} className={on ? "on" : ""} onClick={() => setPicked(on ? wanted.filter((x) => x !== t) : [...wanted, t])}>
                {t}
                <small>{n}</small>
              </button>
            );
          })}
          {wanted.length > 0 && (
            <button className="wardrobe-clear" onClick={() => setPicked([])}>
              All avatars
            </button>
          )}
        </div>
      )}
      {shown.length === 0 && <p className="empty">No avatar in your wardrobe fits that. Clear the search or the tags to see them all.</p>}
      {look === "board" && shown.length > 0 && (
        <ul className="wardrobe">
          {shown.map((a) => {
            const name = tidy(a.name) || "Looking it up";
            const on = a.avatarId === wearing;
            return (
              <li key={a.avatarId} className={on ? "wardrobe-on" : ""}>
                <button className="wardrobe-pic" style={a.image ? { backgroundImage: `url("${a.image}")` } : undefined} onClick={() => openThing("avatar", a.avatarId)} title="Open this avatar" aria-label={`Open ${name}`}>
                  {on && <span className="mark mark-now">Wearing</span>}
                </button>
                <div className="wardrobe-text">
                  <b className="hide-name">{name}</b>
                  <small>
                    {a.lastInGame ? `Worn ${dayLabel(a.lastInGame).toLowerCase()}` : "Not worn in a world yet"}
                    {a.secondsInGame > 0 ? `, ${spent(a.secondsInGame)} in all` : ""}
                  </small>
                  <TagChips id={a.avatarId} tags={tags} />
                </div>
                <div className="wardrobe-actions">
                  <button
                    className={on ? "plain" : "plain go"}
                    disabled={on || busy === a.avatarId}
                    onClick={() => void act(a.avatarId, () => backend.wearAvatar(a.avatarId), `You are now wearing ${name}.`)}
                  >
                    {on ? "On now" : "Wear"}
                  </button>
                  <button className="plain" aria-expanded={editing === a.avatarId} onClick={() => setEditing(editing === a.avatarId ? "" : a.avatarId)} title="Your own tags, to sort the wardrobe with. Nobody else sees them">
                    Tags
                  </button>
                </div>
                {editing === a.avatarId && <TagEditor id={a.avatarId} tags={tags} onDone={onDone} />}
              </li>
            );
          })}
        </ul>
      )}
    {look === "list" && shown.length > 0 && (
    <ul className="fav-list avatar-history">
      {shown.map((a) => {
        const i = a.avatarId === wearing ? 0 : 1;
        const name = tidy(a.name) || "Looking it up";
        return (
          <li className="fav-item" key={a.avatarId}>
            <button className="pic-button" disabled={!a.image} onClick={() => openPicture(a.image, name)} title="Show the picture full size" aria-label={`Show ${name} full size`}>
              <span className="thumb" style={a.image ? { backgroundImage: `url("${a.image}")` } : undefined} />
            </button>
            <button className="row-link" onClick={() => openThing("avatar", a.avatarId)} title="Open this avatar">
              <span className="worn-text">
                <span className="tile-name">
                  {name}
                  {i === 0 && <span className="mark mark-now">Wearing</span>}
                </span>
                {a.authorName && <span className="tile-note">by {a.authorName}</span>}
                <span className="worn-facts">
                  <span className="fact">
                    <small>Time worn</small>
                    <b>{spent(a.secondsInGame)}</b>
                  </span>
                  <span className="fact" title="Stays in a world with this avatar on">
                    <small>Visits</small>
                    <b>{a.visits}</b>
                  </span>
                  <span className="fact" title={`Put on ${a.times === 1 ? "once" : `${a.times} times`}, last ${dayLabel(a.lastWorn).toLowerCase()} ${clock(a.lastWorn)}`}>
                    <small>Last worn</small>
                    <b>{a.lastInGame ? `${dayLabel(a.lastInGame)}, ${clock(a.lastInGame)}` : "Not in a world yet"}</b>
                  </span>
                </span>
                {(list?.length ?? 0) > 1 && all > 0 && (
                  <span className="wear-share" title="This avatar's share of all the time you spent in worlds">
                    <span className="wear-bar" aria-hidden="true">
                      <span style={{ width: `${Math.max(1, Math.round((a.secondsInGame / all) * 100))}%` }} />
                    </span>
                    <small>{Math.round((a.secondsInGame / all) * 100)}% of your time</small>
                  </span>
                )}
              </span>
            </button>
            <span className="row-actions">
              {i > 0 && (
                <button className="go" disabled={busy === a.avatarId} onClick={() => void act(a.avatarId, () => backend.wearAvatar(a.avatarId), `You are now wearing ${name}.`)}>
                  Wear
                </button>
              )}
              <button
                disabled={busy === a.avatarId}
                title="Keeps it in Favourites, under On this PC. There is no limit, and nobody else sees it."
                onClick={() =>
                  void act(
                    a.avatarId,
                    () => backend.addLocalFavorite({ kind: "avatar", itemId: a.avatarId, name: a.name, authorName: a.authorName, image: a.image }),
                    `${name} is kept on this PC.`,
                  )
                }
              >
                Keep on this PC
              </button>
            </span>
          </li>
        );
      })}
    </ul>
    )}
    </>
  );
}
