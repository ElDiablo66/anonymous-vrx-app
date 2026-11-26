import { useEffect, useState } from "react";
import { backend, errorText, type AvatarWorn } from "./backend";
import { clock, dayLabel, tidy } from "./bits";
import { openThing } from "./Detail";
import { openPicture } from "./Viewer";

export function spent(seconds: number): string {
  const minutes = Math.round(seconds / 60);
  if (minutes < 1) return "under a minute";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours} h ${minutes % 60} min`;
  return `${Math.floor(hours / 24)} days ${hours % 24} h`;
}

/** Every avatar I switched to while the app was running. */
export function AvatarHistory({ onDone }: { onDone: (message: string) => void }) {
  const [list, setList] = useState<AvatarWorn[] | null>(null);
  const [busy, setBusy] = useState("");
  const [order, setOrder] = useState<"recent" | "worn">("recent");

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
  const shown = order === "worn" && list ? [...list].sort((a, b) => b.secondsInGame - a.secondsInGame) : (list ?? []);
  const total = (list ?? []).reduce((sum, a) => sum + a.secondsInGame, 0);
  const most = (list ?? []).reduce((top, a) => Math.max(top, a.secondsInGame), 0);

  if (list === null) return <p className="empty">Reading your avatar history</p>;
  if (list.length === 0) {
    return (
      <p className="empty">
        Nothing yet. Every avatar you switch to while this app runs is listed here, with how often and how long you
        wore it.
      </p>
    );
  }

  return (
    <>
      <div className="list-tools">
        <div className="choices" role="radiogroup" aria-label="Order">
          <button role="radio" aria-checked={order === "recent"} className={order === "recent" ? "on" : ""} onClick={() => setOrder("recent")}>
            Latest first
          </button>
          <button role="radio" aria-checked={order === "worn"} className={order === "worn" ? "on" : ""} onClick={() => setOrder("worn")}>
            Most worn
          </button>
        </div>
        <span className="tile-note">
          {list.length} {list.length === 1 ? "avatar" : "avatars"}, {spent(total)} worn in VRChat since this app started keeping track.
        </span>
      </div>
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
                <span className="wear-bar" aria-hidden="true">
                  <span style={{ width: `${most > 0 ? Math.max(1, Math.round((a.secondsInGame / most) * 100)) : 0}%` }} />
                </span>
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
    </>
  );
}
