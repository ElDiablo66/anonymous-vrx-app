import { useEscape, useVeil } from "./escape";
import { useEffect, useState } from "react";
import { backend, errorText, type Friend } from "./backend";
import { Avatar, accessName, ageGated, regionName, sinceWords, statusTone, tidy } from "./bits";
import { live } from "./settings";
import { openThing } from "./Detail";
import { WayBack } from "./History";
import { parsePlace } from "./place";
import { openProfile } from "./Profile";
import { openPicture } from "./Viewer";
import { QueueButton } from "./Queue";

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

  // Escape goes back one window when there is one to go back to; Close puts them all away.
  useEscape(() => (onBack ? onBack() : onClose()));
  const veil = useVeil(onClose);

  const name = tidy(str(world?.name)) || "World";
  const image = str(world?.imageUrl) || str(world?.thumbnailImageUrl);
  const here = [...friends.values()]
    .filter((f) => f.state === "online" && f.location === location)
    .sort((a, b) => a.displayName.localeCompare(b.displayName));
  // VRChat sends the head count under two names and one can read 0, so the larger counts,
  // the same as on the cards.
  const count = Math.max(num(info?.n_users), num(info?.userCount));
  const waiting = info?.queueEnabled === true ? Math.max(0, num(info?.queueSize)) : 0;
  const capacity = num(info?.capacity) || num(world?.capacity);
  // VRChat lists everyone only to people who are inside the instance themselves.
  const everyone = Array.isArray(info?.users) ? (info.users as Json[]).filter((u) => u && typeof u === "object") : [];
  const others = everyone.filter((u) => !friends.has(str(u.id)));
  const full = info?.full === true || (capacity > 0 && count >= capacity);
  const known = info !== null && count > 0;
  const free = capacity > 0 && known ? Math.max(0, capacity - count) : null;
  const share = capacity > 0 && known ? Math.min(1, count / capacity) : 0;
  // Past the size the world's maker recommends, a world gets crowded before it is full.
  const comfy = num(world?.recommendedCapacity);
  const crowded = comfy > 0 && comfy < capacity && count > comfy;
  const tone = full ? "full" : crowded || share >= 0.85 ? "busy" : "open";
  const about = tidy(str(world?.description));
  const author = tidy(str(world?.authorName));
  // VRChat lists only public instances for a world, so this one is often not among them.
  // Each entry is [instance id, people]; only the ones that are not this instance count.
  const elsewhere = Array.isArray(world?.instances)
    ? (world.instances as unknown[]).filter((x) => Array.isArray(x) && typeof x[0] === "string" && `${worldId}:${x[0]}` !== location).length
    : 0;

  return (
    <div className="veil" {...veil}>
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
            {place.instance && live().showInstanceId && <li>#{place.instance}</li>}
            {ageGated(location) && <li className="chip-adult" title="Only people VRChat has verified as 18 or older get in">18+</li>}
          </ul>
          <QueueButton location={location} worldName={name} full={full} inLine={waiting} onDone={onDone} />
          <WayBack location={location} onDone={onDone} />
        </aside>

        <div className="profile-main">
          <div className="profile-top">
            <h3 className="window-title">Who is here</h3>
            <span className="window-nav">
              {onBack && <button onClick={onBack}>Back</button>}
              <button onClick={onClose}>Close</button>
            </span>
          </div>
          <div className="profile-body inst-body">
            <section className={`inst-count inst-${tone}`}>
              <div className="inst-figures">
                <p className="inst-big">
                  {info === null ? <span className="inst-wait">Asking VRChat</span> : known ? <b>{count}</b> : <span className="inst-wait">No head count</span>}
                  {known && capacity > 0 && <span>of {capacity} people</span>}
                  {known && capacity === 0 && <span>{count === 1 ? "person" : "people"}</span>}
                </p>
                <p className="inst-state">
                  {info === null ? "" : full ? "Full" : free !== null ? `${free} ${free === 1 ? "place" : "places"} free` : ""}
                  {waiting > 0 && <small className="inst-waiting">{waiting} waiting</small>}
                </p>
              </div>
              {capacity > 0 && (
                <div className="inst-meter" role="img" aria-label={known ? `${count} of ${capacity} places taken` : "Head count not known"}>
                  <i style={{ width: `${Math.round(share * 100)}%` }} />
                  {comfy > 0 && comfy < capacity && <u style={{ left: `${Math.round((comfy / capacity) * 100)}%` }} title={`The world's maker recommends ${comfy} people at most`} />}
                </div>
              )}
              <p className="inst-notes">
                <span>
                  {here.length === 0 ? "None of your friends" : here.length === 1 ? "1 friend of yours" : `${here.length} friends of yours`}
                  {known ? `, ${Math.max(0, count - here.length)} ${count - here.length === 1 ? "other" : "others"}` : ""}
                </span>
                {comfy > 0 && comfy < capacity && <span>{crowded ? `Past the ${comfy} its maker recommends` : `Comfortable up to ${comfy}`}</span>}
                {waiting > 0 && <span>{waiting === 1 ? "1 person is" : `${waiting} people are`} in VRChat's queue and get in before anyone else</span>}
              </p>
              {problem && <p className="profile-fine">VRChat did not share the head count: {problem}</p>}
            </section>

            <section>
              <h3>Friends here</h3>
              {here.length === 0 ? (
                <p className="profile-fine">None of your friends are in this instance right now. They may have moved on since the card was drawn.</p>
              ) : (
                <ul className="inst-friends">
                  {here.map((f) => (
                    <li key={f.id}>
                      <button className="inst-friend" data-person={f.id} data-person-name={f.displayName} onClick={() => openProfile(f.id)} title={`Open ${f.displayName}'s profile`}>
                        <Avatar name={f.displayName} image={f.image} tone={statusTone(f)} />
                        <span className="inst-friend-text">
                          <b className="hide-name">{f.displayName}</b>
                          {f.statusDescription && <small className="hide-name">{f.statusDescription}</small>}
                        </span>
                        {f.since && (
                          <span className="inst-since" title="How long they have been in this instance">
                            {sinceWords(f.since)}
                            {f.sinceAtLeast ? "+" : ""}
                          </span>
                        )}
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </section>

            {(about || author) && (
              <section>
                <h3>The world</h3>
                {about && <p className="inst-about">{about}</p>}
                <p className="inst-world-line">
                  {author && (
                    <span>
                      by{" "}
                      <button className="name-link who" disabled={!str(world?.authorId)} onClick={() => openProfile(str(world?.authorId))}>
                        {author}
                      </button>
                    </span>
                  )}
                  {elsewhere > 0 && (
                    <span>
                      {elsewhere} other open {elsewhere === 1 ? "instance" : "instances"},{" "}
                      <button className="name-link" onClick={() => openThing("world", worldId)}>
                        see them
                      </button>
                    </span>
                  )}
                </p>
              </section>
            )}

            {others.length > 0 ? (
              <section>
                <h3>Everyone else</h3>
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
              </section>
            ) : (
              <p className="inst-foot">VRChat names everyone only to people inside the instance. From outside it shares the head count and your friends.</p>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
