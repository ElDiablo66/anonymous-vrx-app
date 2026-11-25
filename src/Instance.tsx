import { useEffect, useState } from "react";
import { backend, errorText, type Friend } from "./backend";
import { Avatar, accessName, regionName, statusTone, tidy } from "./bits";
import { openThing } from "./Detail";
import { WayBack } from "./History";
import { parsePlace } from "./place";
import { openProfile } from "./Profile";
import { openPicture } from "./Viewer";

type Json = Record<string, unknown>;
const str = (v: unknown): string => (typeof v === "string" ? v : "");
const num = (v: unknown): number => (typeof v === "number" ? v : 0);

/** One instance of a world: where it is, how full, and who of my friends is in it. */
export function InstancePanel({
  location,
  friends,
  onBack,
  onClose,
  onDone,
}: {
  location: string;
  friends: Map<string, Friend>;
  onBack?: () => void;
  onClose: () => void;
  onDone: (message: string) => void;
}) {
  const place = parsePlace(location);
  const worldId = place.worldId ?? "";
  const [world, setWorld] = useState<Json | null>(null);
  const [info, setInfo] = useState<Json | null>(null);
  const [problem, setProblem] = useState("");

  useEffect(() => {
    let alive = true;
    setWorld(null);
    setInfo(null);
    setProblem("");
    void backend
      .thing("world", worldId)
      .then((w) => alive && setWorld((w ?? {}) as Json))
      .catch(() => alive && setWorld({}));
    void backend
      .thing("instance", location)
      .then((i) => alive && setInfo((i ?? {}) as Json))
      .catch((err) => {
        if (!alive) return;
        setInfo({});
        setProblem(errorText(err));
      });
    return () => {
      alive = false;
    };
  }, [location, worldId]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const name = tidy(str(world?.name)) || "World";
  const image = str(world?.imageUrl) || str(world?.thumbnailImageUrl);
  const here = [...friends.values()]
    .filter((f) => f.state === "online" && f.location === location)
    .sort((a, b) => a.displayName.localeCompare(b.displayName));
  const count = num(info?.n_users) || num(info?.userCount);
  const capacity = num(info?.capacity) || num(world?.capacity);
  // VRChat lists everyone only to people who are inside the instance themselves.
  const everyone = Array.isArray(info?.users) ? (info.users as Json[]).filter((u) => u && typeof u === "object") : [];
  const others = everyone.filter((u) => !friends.has(str(u.id)));

  return (
    <div className="veil" onClick={onClose}>
      <div className="profile" role="dialog" aria-modal="true" aria-label={`${name}, instance`} onClick={(e) => e.stopPropagation()}>
        <aside className="profile-side">
          <button
            className="big-pic"
            style={image ? { backgroundImage: `url("${image}")` } : undefined}
            disabled={!image}
            title="Show the picture full size"
            aria-label="Show the picture full size"
            onClick={() => openPicture(image, name)}
          />
          <h2 className="hide-me">{name}</h2>
          <p className="profile-status">
            <button className="name-link" onClick={() => openThing("world", worldId)}>
              Open the world
            </button>
          </p>
          <ul className="badges">
            <li className={`access access-${place.access}`}>{accessName(place)}</li>
            {regionName(place) && <li>{regionName(place)}</li>}
            {place.instance && <li>#{place.instance}</li>}
          </ul>
          <WayBack location={location} onDone={onDone} />
        </aside>

        <div className="profile-main">
          <div className="profile-top">
            <div className="tabs" role="tablist" aria-label="Instance">
              <button role="tab" aria-selected className="on">
                Who is here
              </button>
            </div>
            <span className="window-nav">
              {onBack && <button onClick={onBack}>Back</button>}
              <button onClick={onClose}>Close</button>
            </span>
          </div>
          <div className="profile-body">
            <section>
              <h3>People</h3>
              <dl className="facts">
                <div>
                  <dt>In the instance</dt>
                  <dd>{info === null ? "Asking VRChat" : count ? `${count}${capacity ? ` of ${capacity}` : ""}` : "VRChat did not say"}</dd>
                </div>
                <div>
                  <dt>Your friends here</dt>
                  <dd>{here.length}</dd>
                </div>
              </dl>
              {problem && <p className="profile-fine">VRChat did not share the head count: {problem}</p>}
            </section>

            <section>
              <h3>Friends here</h3>
              {here.length === 0 ? (
                <p className="profile-fine">None of your friends are in this instance right now.</p>
              ) : (
                <ul className="chips">
                  {here.map((f) => (
                    <li className="chip chip-friend chip-open" data-person={f.id} data-person-name={f.displayName} role="button" tabIndex={0} key={f.id} onClick={() => openProfile(f.id)} onKeyDown={(e) => e.key === "Enter" && openProfile(f.id)}>
                      <Avatar name={f.displayName} image={f.image} tone={statusTone(f)} />
                      <span className="chip-text">
                        <span className="chip-name">{f.displayName}</span>
                        {f.statusDescription && <span className="chip-note">{f.statusDescription}</span>}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </section>

            <section>
              <h3>Everyone else</h3>
              {others.length > 0 ? (
                <ul className="chips">
                  {others.map((u) => (
                    <li className="chip chip-open" data-person={str(u.id)} data-person-name={str(u.displayName)} role="button" tabIndex={0} key={str(u.id)} onClick={() => openProfile(str(u.id))} onKeyDown={(e) => e.key === "Enter" && openProfile(str(u.id))}>
                      <Avatar name={str(u.displayName)} image={str(u.userIcon) || str(u.currentAvatarThumbnailImageUrl)} />
                      <span className="chip-text">
                        <span className="chip-name">{str(u.displayName)}</span>
                      </span>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="profile-fine">
                  VRChat names everyone only to people who are inside the instance. From outside, the head count and
                  your friends are all it shares.
                </p>
              )}
            </section>
          </div>
        </div>
      </div>
    </div>
  );
}
