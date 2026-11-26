import type { CSSProperties, ReactNode } from "react";
import type { Friend, Instance, World } from "./backend";
import { Avatar, accessName, statusTone, tidy } from "./bits";
import { openThing } from "./Detail";
import { parsePlace } from "./place";
import { openProfile } from "./Profile";

interface Room {
  location: string;
  worldId: string;
  people: Friend[];
}

const FACE = 44;

/** How wide a bubble is: enough for its faces, and a little more the fuller the instance is. */
function width(friends: number, everyone: number): number {
  const forFaces = friends <= 1 ? 120 : 110 + Math.sqrt(friends) * 62;
  return Math.round(Math.min(380, forFaces + Math.min(60, Math.sqrt(Math.max(0, everyone - friends)) * 9)));
}

/** Where each face sits in a bubble: one in the middle, more on a ring, many on two rings. */
function seats(count: number, size: number): Array<{ x: number; y: number }> {
  if (count === 1) return [{ x: 0, y: 0 }];
  const outer = size / 2 - FACE / 2 - 12;
  const fits = Math.max(2, Math.floor((2 * Math.PI * outer) / (FACE + 6)));
  const onOuter = Math.min(count, fits);
  const inside = count - onOuter;
  const ring = (n: number, radius: number, turn: number) =>
    Array.from({ length: n }, (_, i) => {
      const angle = turn + (i / n) * 2 * Math.PI;
      return { x: Math.cos(angle) * radius, y: Math.sin(angle) * radius };
    });
  return [...ring(onOuter, outer, -Math.PI / 2), ...(inside === 1 ? [{ x: 0, y: 0 }] : ring(inside, Math.max(0, outer - FACE - 6), -Math.PI / 2 + 0.4))];
}

function Face({ f, at }: { f: Friend; at: { x: number; y: number } }) {
  return (
    <button
      className="map-face"
      style={{ translate: `${at.x}px ${at.y}px` }}
      title={f.statusDescription ? `${f.displayName}: ${f.statusDescription}` : f.displayName}
      aria-label={`${f.displayName}, open the profile`}
      onClick={() => openProfile(f.id)}
    >
      <Avatar name={f.displayName} image={f.image} tone={statusTone(f)} size={FACE} />
    </button>
  );
}

function Bubble({ people, size, picture, tone, children }: { people: Friend[]; size: number; picture?: string; tone: string; children: ReactNode }) {
  // Faces past what two rings hold are counted, not drawn.
  const most = 28;
  const shown = people.slice(0, most);
  const places = seats(shown.length, size);
  return (
    <li className={`map-room map-${tone}`}>
      <div className="map-bubble" style={{ width: size, height: size, "--pic": picture ? `url("${picture}")` : "none" } as CSSProperties}>
        {shown.map((f, i) => (
          <Face key={f.id} f={f} at={places[i]} />
        ))}
        {people.length > most && <span className="map-more">+{people.length - most}</span>}
      </div>
      {children}
    </li>
  );
}

/**
 * Who is with whom, as a picture: one bubble per instance with the friends in it, bigger the
 * more of them there are, then one bubble for everyone in private worlds.
 */
export function FriendMap({
  rooms,
  hidden,
  moving,
  worlds,
  instances,
}: {
  rooms: Room[];
  hidden: Friend[];
  moving: Friend[];
  worlds: Map<string, World>;
  instances: Map<string, Instance>;
}) {
  if (rooms.length + hidden.length + moving.length === 0) return <p className="empty">Nobody is in VRChat right now.</p>;
  return (
    <ul className="friend-map" aria-label="Friends by the instance they are in">
      {rooms.map((room) => {
        const world = worlds.get(room.worldId);
        const everyone = instances.get(room.location)?.users ?? 0;
        const place = parsePlace(room.location);
        const name = tidy(world?.name ?? "") || "A world";
        return (
          <Bubble key={room.location} people={room.people} size={width(room.people.length, everyone)} picture={world?.image} tone={place.access || "public"}>
            <button className="map-label" onClick={() => openThing("instance", room.location)} title="Open this instance">
              <b>{name}</b>
              <span>
                {accessName(place)}
                {everyone > 0 ? `, ${room.people.length} of ${everyone} ${everyone === 1 ? "person" : "people"}` : `, ${room.people.length} ${room.people.length === 1 ? "friend" : "friends"}`}
              </span>
            </button>
          </Bubble>
        );
      })}
      {hidden.length > 0 && (
        <Bubble people={hidden} size={width(hidden.length, hidden.length)} tone="private">
          <span className="map-label">
            <b>Private worlds</b>
            <span>
              {hidden.length} {hidden.length === 1 ? "friend" : "friends"}, each somewhere you cannot see
            </span>
          </span>
        </Bubble>
      )}
      {moving.length > 0 && (
        <Bubble people={moving} size={width(moving.length, moving.length)} tone="moving">
          <span className="map-label">
            <b>On the way</b>
            <span>Between worlds right now</span>
          </span>
        </Bubble>
      )}
    </ul>
  );
}
