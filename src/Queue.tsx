import { useEffect, useState } from "react";
import { backend, errorText, type Waiting } from "./backend";
import { launched } from "./History";
import type { Settings } from "./settings";

/** The full instances I am waiting for. Kept in step with the app's own list. */
export function useWaiting(): Waiting[] {
  const [list, setList] = useState<Waiting[]>([]);
  useEffect(() => {
    let alive = true;
    const read = () =>
      void backend
        .waiting()
        .then((w) => alive && setList(w))
        .catch(() => {});
    read();
    const stop = backend.on("queue:changed", read);
    return () => {
      alive = false;
      void stop.then((s) => s());
    };
  }, []);
  // The minutes shown next to a wait move on.
  const some = list.length > 0;
  useEffect(() => {
    if (!some) return;
    const id = window.setInterval(() => void backend.waiting().then(setList).catch(() => {}), 60_000);
    return () => window.clearInterval(id);
  }, [some]);
  return list;
}

/** Starts or stops waiting for room in one instance, and says what happens next. */
export async function toggleWait(location: string, worldName: string, waiting: boolean, onDone: (message: string) => void) {
  try {
    if (waiting) {
      await backend.stopWaiting(location);
      onDone(`Stopped waiting for ${worldName}.`);
    } else {
      await backend.waitForRoom(location, worldName);
      onDone(`Waiting for room in ${worldName}. The app looks every 45 seconds and tells you. It gives up after an hour.`);
    }
  } catch (err) {
    onDone(`That did not work: ${errorText(err)}`);
  }
}

/** In the instance window: shown while the instance is full, or while I wait for it. */
export function QueueButton({ location, worldName, full, onDone }: { location: string; worldName: string; full: boolean; onDone: (message: string) => void }) {
  const waiting = useWaiting().some((w) => w.location === location);
  const [busy, setBusy] = useState(false);
  if (!full && !waiting) return null;
  return (
    <div className="queue-ask">
      <p>
        {waiting ? "You are waiting for room here." : "This instance is full."} <span className="mark">Beta</span>
      </p>
      <button
        className={waiting ? "plain" : "plain go"}
        disabled={busy}
        onClick={() => {
          setBusy(true);
          void toggleWait(location, worldName, waiting, onDone).finally(() => setBusy(false));
        }}
      >
        {waiting ? "Stop waiting" : "Tell me when there is room"}
      </button>
    </div>
  );
}

const minutes = (n: number) => (n < 1 ? "just now" : `${n} min`);

/** Above the page: the instances I wait for, and the ones that now have room. */
export function QueueBar({ settings, onDone }: { settings: Settings; onDone: (message: string) => void }) {
  const waiting = useWaiting();
  const [rooms, setRooms] = useState<Array<{ location: string; worldName: string; free: number }>>([]);
  const join = (location: string) =>
    void backend
      .launchInstance(location, settings.launchMode)
      .then((how) => onDone(launched[how]))
      .catch((err) => onDone(`That did not work: ${errorText(err)}`));
  const auto = settings.queueJoin;

  useEffect(() => {
    const stops = [
      backend.on("queue:room", (room) => {
        if (auto) {
          onDone(`Room in ${room.worldName}. Joining now.`);
          join(room.location);
        } else {
          setRooms((old) => [room, ...old.filter((r) => r.location !== room.location)]);
        }
      }),
      backend.on("queue:gone", (gone) => onDone(gone.why === "hour" ? `Still full after an hour, so the wait for ${gone.worldName} has ended.` : `The instance of ${gone.worldName} you waited for has closed.`)),
    ];
    return () => stops.forEach((stop) => void stop.then((s) => s()));
  }, [auto, settings.launchMode]); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <>
      {rooms.map((room) => (
        <div className="banner crash-note queue-room" role="alert" key={room.location}>
          <span>
            There is room in {room.worldName}: {room.free} {room.free === 1 ? "place" : "places"} free a moment ago.
          </span>
          <button
            className="go"
            onClick={() => {
              setRooms((old) => old.filter((r) => r.location !== room.location));
              join(room.location);
            }}
          >
            Join
          </button>
          <button onClick={() => setRooms((old) => old.filter((r) => r.location !== room.location))}>Not now</button>
        </div>
      ))}
      {waiting.length > 0 && (
        <div className="queue-strip" role="status">
          <span className="queue-lead">
            Waiting for room <span className="mark">Beta</span>
          </span>
          <ul>
            {waiting.map((w) => (
              <li key={w.location}>
                <b className="hide-me">{w.worldName}</b>
                <small>{minutes(w.minutes)}</small>
                <button className="plain" onClick={() => void toggleWait(w.location, w.worldName, true, onDone)}>
                  Stop
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </>
  );
}
