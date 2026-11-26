import { Photos } from "./Photos";
import { useEffect, useState } from "react";
import { backend, errorText, type Friend, type Sighting, type Visit, type World } from "./backend";
import { Avatar, accessName, clock, dayLabel, regionName, statusTone, tidy } from "./bits";
import { joinLink } from "./place";
import { savedLaunchMode } from "./settings";
import { AvatarHistory } from "./AvatarHistory";
import { FeedBrowser } from "./FeedBrowser";
import { openProfile } from "./Profile";

export function stayLength(from: string, to: string | null): string {
  const minutes = Math.max(0, Math.round(((to ? Date.parse(to) : Date.now()) - Date.parse(from)) / 60_000));
  if (minutes < 1) return "under a minute";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest ? `${hours} h ${rest} min` : `${hours} h`;
}

export const launched = {
  running: "VRChat is open, so it is moving you there.",
  desktop: "Starting VRChat in desktop mode, straight into that instance.",
  vr: "Starting VRChat in VR, straight into that instance.",
};

/** Asks VRChat once whether an instance from the past is still there, and how many are in it. */
function StillOpen({ location }: { location: string }) {
  const [state, setState] = useState<{ open: boolean; users: number; capacity: number } | null>(null);
  useEffect(() => {
    let alive = true;
    setState(null);
    void backend
      .thing("instance", location)
      .then((raw) => {
        if (!alive) return;
        const i = (raw ?? {}) as Record<string, unknown>;
        const n = (v: unknown) => (typeof v === "number" ? v : 0);
        const closed = typeof i.closedAt === "string" && i.closedAt !== "";
        setState({ open: !closed && typeof i.id === "string", users: n(i.n_users) || n(i.userCount), capacity: n(i.capacity) });
      })
      .catch(() => alive && setState({ open: false, users: 0, capacity: 0 }));
    return () => {
      alive = false;
    };
  }, [location]);
  if (!state) return <p className="tile-note still-open">Asking VRChat if this instance is still open</p>;
  return (
    <p className={`tile-note still-open ${state.open ? "still-yes" : "still-no"}`}>
      {state.open
        ? `Still open, ${state.users} ${state.users === 1 ? "person" : "people"} in it now${state.capacity ? ` of ${state.capacity}` : ""}.`
        : "This instance has closed, or you can no longer see it."}
    </p>
  );
}

/**
 * Ways into an instance. "Join in VRChat" starts the game there (or moves the
 * running game). "Invite me" is for when the game is already open.
 * `shareOnly` leaves just the two that make sense for the world you are already in.
 */
export function WayBack({ location, onDone, shareOnly }: { location: string; onDone: (message: string) => void; shareOnly?: boolean }) {
  const [busy, setBusy] = useState(false);
  const link = joinLink(location);
  if (!link) return null;

  async function run(job: () => Promise<string>) {
    setBusy(true);
    try {
      onDone(await job());
    } catch (err) {
      onDone(`That did not work: ${errorText(err)}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="wayback">
      {!shareOnly && (
        <>
          <button
            className="go"
            disabled={busy}
            title="Starts VRChat in this instance. If the game is already open, it moves you there."
            onClick={() => void run(async () => launched[await backend.launchInstance(location, savedLaunchMode())])}
          >
            Join in VRChat
          </button>
          <button
            disabled={busy}
            title="For when VRChat is already open: sends you an invite that shows up in your notifications inside the game"
            onClick={() =>
              void run(async () => {
                await backend.inviteMyself(location);
                return "Invite sent. Look in your notifications inside VRChat; nothing opens on the desktop.";
              })
            }
          >
            Invite me
          </button>
        </>
      )}
      <button
        title="Copies a link that opens this exact instance on VRChat's website"
        onClick={() => {
          void navigator.clipboard.writeText(link).then(
            () => onDone("Join link copied."),
            () => onDone("Could not copy the link."),
          );
        }}
      >
        Copy link
      </button>
      <button
        title="Opens this instance on VRChat's website"
        onClick={() =>
          void run(async () => {
            await backend.openJoinPage(location);
            return "Opened in your browser.";
          })
        }
      >
        Website
      </button>
    </div>
  );
}

/** Friends first, then by when they arrived. The signed-in user is left out. */
export function PeopleList({
  people,
  friends,
  meId,
  nobody = "Nobody else was here.",
}: {
  people: Sighting[];
  friends: Map<string, Friend>;
  meId: string;
  nobody?: string;
}) {
  const seen = new Map<string, Sighting>();
  for (const p of people) {
    if (p.userId === meId) continue;
    // Someone who left and came back shows once, with their latest stay.
    seen.set(p.userId || p.displayName, p);
  }
  const rows = [...seen.values()].sort(
    (a, b) => Number(friends.has(b.userId)) - Number(friends.has(a.userId)) || a.joinedAt.localeCompare(b.joinedAt),
  );
  if (rows.length === 0) return <p className="empty people-none">{nobody}</p>;
  return (
    <ul className="chips">
      {rows.map((p) => {
        const friend = friends.get(p.userId);
        return (
          <li
            className={`chip${p.leftAt ? " chip-gone" : ""}${friend ? " chip-friend" : ""}${p.userId ? " chip-open" : ""}`}
            key={p.userId || p.displayName}
            data-person={p.userId || undefined}
            data-person-name={p.displayName}
            role={p.userId ? "button" : undefined}
            tabIndex={p.userId ? 0 : undefined}
            title={p.userId ? "Open profile" : undefined}
            onClick={() => p.userId && openProfile(p.userId)}
            onKeyDown={(e) => e.key === "Enter" && p.userId && openProfile(p.userId)}
          >
            <Avatar name={p.displayName} image={friend?.image ?? ""} tone={friend ? statusTone(friend) : undefined} />
            <span className="chip-text">
              <span className="chip-name">{p.displayName}</span>
              <span className="chip-note">
                {friend ? "Friend, " : ""}
                {p.leftAt ? `left ${clock(p.leftAt)}` : `since ${clock(p.joinedAt)}`}
              </span>
            </span>
          </li>
        );
      })}
    </ul>
  );
}

interface HistoryProps {
  friends: Map<string, Friend>;
  worlds: Map<string, World>;
  meId: string;
  tick: number;
  onDone: (message: string) => void;
}

const historyTabs: Array<["worlds" | "avatars" | "feed" | "photos", string]> = [
  ["worlds", "Worlds"],
  ["photos", "Photos"],
  ["avatars", "Avatars"],
  ["feed", "Friend activity"],
];

export function History(props: HistoryProps) {
  const [tab, setTab] = useState<"worlds" | "avatars" | "feed" | "photos">("worlds");
  return (
    <div className="table-wrap">
      <div className="tabs" role="tablist" aria-label="History">
        {historyTabs.map(([id, label]) => (
          <button key={id} role="tab" aria-selected={tab === id} className={tab === id ? "on" : ""} onClick={() => setTab(id)}>
            {label}
          </button>
        ))}
      </div>
      {tab === "worlds" && <WorldVisits {...props} />}
      {tab === "photos" && <Photos onDone={props.onDone} />}
      {tab === "avatars" && <AvatarHistory onDone={props.onDone} />}
      {tab === "feed" && <FeedBrowser friends={props.friends} worlds={props.worlds} />}
    </div>
  );
}

function WorldVisits({ friends, worlds, meId, tick, onDone }: HistoryProps) {
  const [visits, setVisits] = useState<Visit[] | null>(null);
  const [open, setOpen] = useState<number | null>(null);
  const [people, setPeople] = useState<Sighting[]>([]);
  const [more, setMore] = useState(true);

  useEffect(() => {
    let alive = true;
    void backend
      .visits(100)
      .then((list) => {
        if (!alive) return;
        setVisits(list);
        setMore(list.length === 100);
      })
      .catch(() => alive && setVisits([]));
    return () => {
      alive = false;
    };
  }, [tick]);

  useEffect(() => {
    if (open === null) return;
    let alive = true;
    void backend.visitPeople(open).then((list) => alive && setPeople(list));
    return () => {
      alive = false;
    };
  }, [open, tick]);

  async function loadOlder() {
    if (!visits || visits.length === 0) return;
    const older = await backend.visits(100, visits[visits.length - 1].id);
    setVisits([...visits, ...older]);
    setMore(older.length === 100);
  }

  if (visits === null) return <p className="empty">Reading your world history</p>;
  if (visits.length === 0) {
    return (
      <p className="empty">
        No worlds yet. This fills in from VRChat's own log files on this PC, so play a session and the worlds you
        visit will be listed here with who was in them.
      </p>
    );
  }

  let lastDay = "";
  return (
    <div>
      <ol className="visits">
        {visits.map((v) => {
          const day = dayLabel(v.joinedAt);
          const header = day !== lastDay ? <li className="day" key={`day-${v.id}`}>{day}</li> : null;
          lastDay = day;
          const isOpen = open === v.id;
          const name = tidy(v.worldName || worlds.get(v.worldId)?.name || "Unnamed world");
          return [
            header,
            <li className="visit" key={v.id}>
              <button className="visit-row" aria-expanded={isOpen} onClick={() => { setPeople([]); setOpen(isOpen ? null : v.id); }}>
                <time dateTime={v.joinedAt}>{clock(v.joinedAt)}</time>
                <span className="visit-name">{name}</span>
                <span className={`access access-${v.place.access}`}>{accessName(v.place)}</span>
                <span>{regionName(v.place)}</span>
                <span title="Instance number">{v.place.instance ? `#${v.place.instance}` : ""}</span>
                <span>{v.leftAt ? stayLength(v.joinedAt, v.leftAt) : "there now"}</span>
                <span>{Math.max(0, v.people - 1)} others</span>
              </button>
              {isOpen && (
                <>
                  {v.leftAt && v.place.kind === "world" && <StillOpen location={v.location} />}
                  <WayBack location={v.location} onDone={onDone} />
                  <PeopleList people={people} friends={friends} meId={meId} />
                </>
              )}
            </li>,
          ];
        })}
      </ol>
      {more && (
        <button className="older" onClick={() => void loadOlder()}>
          Show older visits
        </button>
      )}
    </div>
  );
}
