import { useState, type CSSProperties, type ReactNode } from "react";
import { backend, errorText, type Friend, type Instance, type World } from "./backend";
import { Avatar, accessName, regionName, statusTone, statusWords, tidy } from "./bits";
import { openThing } from "./Detail";
import { launched } from "./History";
import { parsePlace } from "./place";
import { openProfile } from "./Profile";
import { savedLaunchMode } from "./settings";

interface Room {
  location: string;
  worldId: string;
  people: Friend[];
}

const FACE = 48;
/** The room one face takes on a ring: the picture, and its name under it. */
const SEAT = 78;
const askFirst = new Set(["invite", "invitePlus"]);

/** How wide a bubble is: enough for its faces and their names, a little more the fuller the instance is. */
function width(friends: number, everyone: number): number {
  const forFaces = friends <= 1 ? 150 : Math.max(210, 80 + (friends * SEAT) / Math.PI + (friends > 9 ? 40 : 0));
  return Math.round(Math.min(460, forFaces + Math.min(50, Math.sqrt(Math.max(0, everyone - friends)) * 8)));
}

/** Where each face sits in a bubble: one in the middle, more on a ring, many on two rings. */
function seats(count: number, size: number): Array<{ x: number; y: number }> {
  if (count === 1) return [{ x: 0, y: -6 }];
  const outer = size / 2 - FACE / 2 - 22;
  const fits = Math.max(2, Math.floor((2 * Math.PI * outer) / SEAT));
  const onOuter = Math.min(count, fits);
  const inside = count - onOuter;
  const ring = (n: number, radius: number, turn: number) =>
    Array.from({ length: n }, (_, i) => {
      const angle = turn + (i / n) * 2 * Math.PI;
      return { x: Math.cos(angle) * radius, y: Math.sin(angle) * radius - 6 };
    });
  return [...ring(onOuter, outer, -Math.PI / 2), ...(inside === 1 ? [{ x: 0, y: -6 }] : ring(inside, Math.max(0, outer - SEAT), -Math.PI / 2 + 0.5))];
}

function Face({ f, at, starred }: { f: Friend; at: { x: number; y: number }; starred: boolean }) {
  const status = f.statusDescription ? `${statusWords(f)}: ${f.statusDescription}` : statusWords(f);
  return (
    <button className={`map-face${starred ? " map-starred" : ""}`} style={{ translate: `${at.x}px ${at.y}px` }} title={`${f.displayName}. ${status}`} onClick={() => openProfile(f.id)}>
      <Avatar name={f.displayName} image={f.image} tone={statusTone(f)} size={FACE} />
      <span className="map-name hide-me">{f.displayName}</span>
      {starred && (
        <span className="map-star" aria-label="A favourite">
          ★
        </span>
      )}
    </button>
  );
}

function Bubble({
  people,
  size,
  picture,
  tone,
  starred,
  order,
  children,
}: {
  people: Friend[];
  size: number;
  picture?: string;
  tone: string;
  starred: Set<string>;
  order: number;
  children: ReactNode;
}) {
  // Faces past what two rings hold are counted, not drawn. Favourites are seated first.
  const most = 22;
  const sorted = [...people].sort((a, b) => Number(starred.has(b.id)) - Number(starred.has(a.id)));
  const shown = sorted.slice(0, most);
  const places = seats(shown.length, size);
  return (
    <li className={`map-room map-${tone}`} style={{ "--order": order } as CSSProperties}>
      <div className="map-bubble" style={{ width: size, height: size, "--pic": picture ? `url("${picture}")` : "none" } as CSSProperties}>
        {shown.map((f, i) => (
          <Face key={f.id} f={f} at={places[i]} starred={starred.has(f.id)} />
        ))}
        {people.length > most && <span className="map-more">and {people.length - most} more</span>}
      </div>
      {children}
    </li>
  );
}

const legend: Array<[string, string]> = [
  ["public", "Public"],
  ["friends", "Friends, Friends+"],
  ["invite", "Invite, Invite+"],
  ["group", "Group"],
  ["private", "Private or not in a world"],
];

/**
 * Who is with whom, as a picture: one bubble per instance with the friends in it, bigger the
 * more of them there are, then one bubble each for private worlds, people between worlds and
 * people who are signed in without being in the game.
 */
export function FriendMap({
  rooms,
  hidden,
  moving,
  away,
  worlds,
  instances,
  starred,
  onDone,
}: {
  rooms: Room[];
  hidden: Friend[];
  moving: Friend[];
  /** Signed in on the website or a phone, not in the game. */
  away: Friend[];
  worlds: Map<string, World>;
  instances: Map<string, Instance>;
  /** Friends in one of my favourite groups. */
  starred: Set<string>;
  onDone: (message: string) => void;
}) {
  const [busy, setBusy] = useState("");
  const [showAway, setShowAway] = useState(false);

  async function go(room: Room, ask: boolean) {
    setBusy(room.location);
    try {
      if (ask) {
        await backend.requestInvite(room.people[0].id);
        onDone(`Asked ${room.people[0].displayName} for an invite. Their answer arrives in your Inbox.`);
      } else {
        onDone(launched[await backend.launchInstance(room.location, savedLaunchMode())]);
      }
    } catch (err) {
      onDone(`That did not work: ${errorText(err)}`);
    } finally {
      setBusy("");
    }
  }

  if (rooms.length + hidden.length + moving.length + away.length === 0) return <p className="empty">Nobody is around right now. Friends show up here as bubbles once they are in VRChat.</p>;
  const inGame = rooms.reduce((n, r) => n + r.people.length, 0) + hidden.length + moving.length;
  let order = 0;
  return (
    <div className="map-wrap">
      <div className="map-top">
        <p className="map-count">
          <b>{inGame}</b> {inGame === 1 ? "friend" : "friends"} in VRChat
          {rooms.length > 0 && (
            <>
              , in <b>{rooms.length}</b> {rooms.length === 1 ? "instance" : "instances"} you can see
            </>
          )}
        </p>
        <ul className="map-legend" aria-label="What the ring colours mean">
          {legend.map(([tone, words]) => (
            <li key={tone} className={`map-${tone}`}>
              <i aria-hidden="true" />
              {words}
            </li>
          ))}
          <li>
            <span className="map-star" aria-hidden="true">
              ★
            </span>
            A favourite
          </li>
        </ul>
      </div>

      <ul className="friend-map" aria-label="Friends by the instance they are in">
        {rooms.map((room) => {
          const world = worlds.get(room.worldId);
          const instance = instances.get(room.location);
          const everyone = instance?.users ?? 0;
          const place = parsePlace(room.location);
          const name = tidy(world?.name ?? "") || "A world";
          const ask = askFirst.has(place.access ?? "");
          const facts = [accessName(place), regionName(place), everyone > 0 ? `${room.people.length} of ${everyone} here are friends` : ""].filter(Boolean);
          return (
            <Bubble key={room.location} people={room.people} size={width(room.people.length, everyone)} picture={world?.image} tone={place.access || "public"} starred={starred} order={order++}>
              <button className="map-label" onClick={() => openThing("instance", room.location)} title="Open this instance">
                <b>{name}</b>
                <span>{facts.join(", ")}</span>
              </button>
              <button
                className={ask ? "plain map-go" : "plain go map-go"}
                disabled={busy === room.location || (!ask && instance?.full === true)}
                title={ask ? `Asks ${room.people[0].displayName} for an invite` : instance?.full ? "This instance is full" : "Starts VRChat in this instance, or moves you there"}
                onClick={() => void go(room, ask)}
              >
                {ask ? "Ask to join" : instance?.full ? "Full" : "Join"}
              </button>
            </Bubble>
          );
        })}
        {hidden.length > 0 && (
          <Bubble people={hidden} size={width(hidden.length, hidden.length)} tone="private" starred={starred} order={order++}>
            <span className="map-label">
              <b>Private worlds</b>
              <span>
                {hidden.length} {hidden.length === 1 ? "friend" : "friends"}. VRChat does not say who is with whom.
              </span>
            </span>
          </Bubble>
        )}
        {moving.length > 0 && (
          <Bubble people={moving} size={width(moving.length, moving.length)} tone="moving" starred={starred} order={order++}>
            <span className="map-label">
              <b>On the way</b>
              <span>Between worlds right now</span>
            </span>
          </Bubble>
        )}
        {showAway && away.length > 0 && (
          <Bubble people={away} size={width(away.length, away.length)} tone="away" starred={starred} order={order++}>
            <span className="map-label">
              <b>Not in the game</b>
              <span>Signed in on the website or a phone</span>
            </span>
          </Bubble>
        )}
      </ul>
      {away.length > 0 && (
        <button className="fold map-away" aria-expanded={showAway} onClick={() => setShowAway((v) => !v)}>
          {showAway ? "Hide" : "Show"} the {away.length} on the website or phone
        </button>
      )}
    </div>
  );
}
