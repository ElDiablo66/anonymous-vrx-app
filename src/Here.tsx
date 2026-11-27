import { useEffect, useState } from "react";
import { backend, type Friend, type HereNow, type Instance, type World } from "./backend";
import { accessName, clock, regionName, tidy } from "./bits";
import { PeopleList, WayBack, stayLength } from "./History";

export function Here({
  friends,
  worlds,
  instances,
  meId,
  tick,
  onDone,
}: {
  friends: Map<string, Friend>;
  worlds: Map<string, World>;
  instances: Map<string, Instance>;
  meId: string;
  tick: number;
  onDone: (message: string) => void;
}) {
  const [here, setHere] = useState<HereNow | null | undefined>(undefined);

  useEffect(() => {
    let alive = true;
    void backend
      .here()
      .then((h) => alive && setHere(h))
      .catch(() => alive && setHere(null));
    return () => {
      alive = false;
    };
  }, [tick]);

  if (here === undefined) return <p className="empty">Checking where you are</p>;
  if (here === null) {
    return (
      <div className="around">
        <p className="here-state here-out">
          <i /> You are not in a world
        </p>
        <p className="empty here-empty">
          VRChat is closed, or still loading. Once you are in a world on this PC, it shows up here within a couple
          of seconds, with everyone in the instance. To go back to a world you were in, open History.
        </p>
      </div>
    );
  }

  const { visit, people } = here;
  const world = worlds.get(visit.worldId);
  const count = instances.get(visit.location);
  const present = people.filter((p) => !p.leftAt);
  const gone = people.filter((p) => p.leftAt);
  const others = present.filter((p) => p.userId !== meId);
  const friendsHere = others.filter((p) => friends.has(p.userId)).length;

  return (
    <div className="around">
      <p className="here-state here-in">
        <i /> You are here now
      </p>
      <article className="room room-here">
        <div className="room-pic" style={world?.image ? { backgroundImage: `url("${world.image}")` } : undefined} />
        <div className="room-body">
          <h2 className="hide-me">{tidy(visit.worldName || world?.name || "Unnamed world")}</h2>
          <p className="room-meta">
            <span className={`access access-${visit.place.access}`}>{accessName(visit.place)}</span>
            {regionName(visit.place) && <span>{regionName(visit.place)}</span>}
            {visit.place.instance && <span title="Instance number">#{visit.place.instance}</span>}
          </p>
          <dl className="facts">
            <div>
              <dt>Since</dt>
              <dd>
                {clock(visit.joinedAt)}, {stayLength(visit.joinedAt, null)}
              </dd>
            </div>
            <div>
              <dt>With you</dt>
              <dd>{others.length === 0 ? "Nobody" : others.length === 1 ? "1 other person" : `${others.length} other people`}</dd>
            </div>
            <div>
              <dt>Friends</dt>
              <dd>{friendsHere === 0 ? "None here" : friendsHere}</dd>
            </div>
            {count && (
              <div>
                <dt>VRChat says</dt>
                <dd>
                  {count.capacity > 0 ? `${count.users} of ${count.capacity}` : count.users}
                  {count.queueEnabled && count.queueSize > 0 ? `, ${count.queueSize} waiting` : ""}
                </dd>
              </div>
            )}
          </dl>
        </div>
        <div className="room-foot">
          <WayBack location={visit.location} onDone={onDone} shareOnly />
        </div>
      </article>
      <section className="band">
        <h2>In the instance with you</h2>
        <PeopleList people={present} friends={friends} meId={meId} nobody="You are alone here." />
      </section>
      {gone.length > 0 && (
        <section className="band band-dim">
          <h2>Left while you were here</h2>
          <PeopleList people={gone} friends={friends} meId={meId} />
        </section>
      )}
    </div>
  );
}
