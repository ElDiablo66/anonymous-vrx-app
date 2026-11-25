import { useEffect, useState } from "react";
import { backend, errorText } from "./backend";
import { prettyGroup } from "./Favourites";

type Json = Record<string, unknown>;
const str = (v: unknown): string => (typeof v === "string" ? v : "");
const list = (v: unknown): Json[] => (Array.isArray(v) ? (v.filter((x) => x && typeof x === "object") as Json[]) : []);

/** For someone who is not a friend yet: send a request, or take one back. */
export function AddFriend({ userId, name, onDone }: { userId: string; name: string; onDone: (message: string) => void }) {
  const [status, setStatus] = useState<{ outgoing: boolean; incoming: boolean } | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    setStatus(null);
    void backend
      .friendStatus(userId)
      .then((s) => alive && setStatus({ outgoing: s.outgoingRequest === true, incoming: s.incomingRequest === true }))
      .catch(() => alive && setStatus({ outgoing: false, incoming: false }));
    return () => {
      alive = false;
    };
  }, [userId]);

  async function act(action: "add" | "cancel", done: string) {
    setBusy(true);
    try {
      await backend.friendAction(userId, action);
      setStatus({ outgoing: action === "add", incoming: false });
      onDone(done);
    } catch (err) {
      onDone(`That did not work: ${errorText(err)}`);
    } finally {
      setBusy(false);
    }
  }

  if (!status) return null;
  return (
    <div className="wayback">
      {status.incoming ? (
        <button className="go" disabled={busy} title="They already asked you; sending yours accepts it" onClick={() => void act("add", `You and ${name} are now friends.`)}>
          Accept friend request
        </button>
      ) : status.outgoing ? (
        <>
          <button disabled>Friend request sent</button>
          <button disabled={busy} onClick={() => void act("cancel", "Friend request taken back.")}>
            Take it back
          </button>
        </>
      ) : (
        <button className="go" disabled={busy} onClick={() => void act("add", `Friend request sent to ${name}.`)}>
          Add friend
        </button>
      )}
    </div>
  );
}

/** Unfriending asks twice. */
export function Unfriend({ userId, name, onDone }: { userId: string; name: string; onDone: (message: string) => void }) {
  const [asking, setAsking] = useState(false);
  const [busy, setBusy] = useState(false);
  if (!asking) return <button onClick={() => setAsking(true)}>Unfriend</button>;
  return (
    <>
      <button
        disabled={busy}
        onClick={() => {
          setBusy(true);
          void backend
            .friendAction(userId, "remove")
            .then(() => onDone(`${name} is no longer on your friend list.`))
            .catch((err) => onDone(`That did not work: ${errorText(err)}`))
            .finally(() => {
              setBusy(false);
              setAsking(false);
            });
        }}
      >
        Yes, unfriend
      </button>
      <button onClick={() => setAsking(false)}>Keep</button>
    </>
  );
}

export interface BlockState {
  block: boolean;
  mute: boolean;
}

/** Mute and block for one person. Blocking asks twice; taking either back does not. The
 *  profile holds the state, so every place that shows it changes together. `only` narrows it
 *  to the one button that takes a block back. */
export function BlockMute({
  userId,
  name,
  state,
  setState,
  only,
  onDone,
}: {
  userId: string;
  name: string;
  state: BlockState | null;
  setState: (next: BlockState) => void;
  only?: "unblock";
  onDone: (message: string) => void;
}) {
  const [asking, setAsking] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => setAsking(false), [userId]);

  async function set(kind: "block" | "mute", on: boolean, done: string) {
    if (!state) return;
    setBusy(true);
    try {
      await backend.setModeration(userId, kind, on);
      setState({ ...state, [kind]: on });
      onDone(done);
    } catch (err) {
      onDone(`That did not work: ${errorText(err)}`);
    } finally {
      setBusy(false);
      setAsking(false);
    }
  }

  if (!state) return null;
  if (only === "unblock") {
    return (
      <button className="go" disabled={busy} onClick={() => void set("block", false, `${name} is no longer blocked.`)}>
        Unblock
      </button>
    );
  }
  return (
    <>
      {state.mute ? (
        <button disabled={busy} onClick={() => void set("mute", false, `${name} is no longer muted.`)}>
          Unmute
        </button>
      ) : (
        <button disabled={busy} title="You stop hearing them in VRChat. They are not told" onClick={() => void set("mute", true, `${name} is muted.`)}>
          Mute
        </button>
      )}
      {state.block ? (
        <button disabled={busy} onClick={() => void set("block", false, `${name} is no longer blocked.`)}>
          Unblock
        </button>
      ) : asking ? (
        <>
          <button className="danger" disabled={busy} onClick={() => void set("block", true, `${name} is blocked.`)}>
            Yes, block
          </button>
          <button onClick={() => setAsking(false)}>Keep</button>
        </>
      ) : (
        <button title="You and they stop seeing and hearing each other in VRChat" onClick={() => setAsking(true)}>
          Block
        </button>
      )}
    </>
  );
}

const standardGroups = ["group_0", "group_1", "group_2", "group_3"];

/** Puts a friend into one of my favourite friend groups, or takes them out. */
export function FriendFavourite({ userId, name, onDone }: { userId: string; name: string; onDone: (message: string) => void }) {
  const [all, setAll] = useState<Json | null>(null);
  const [group, setGroup] = useState("");
  const [busy, setBusy] = useState(false);

  async function load(fresh: boolean) {
    try {
      const data = ((!fresh && (await backend.favoritesCached("friend"))) || (await backend.favoritesOf("friend"))) as Json;
      setAll(data);
    } catch {
      setAll({});
    }
  }

  useEffect(() => {
    setAll(null);
    void load(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [userId]);

  if (!all) return null;
  const capacity = typeof all.capacity === "number" ? all.capacity : 0;
  const entries = list(all.entries);
  const mine = entries.find((e) => str(e.favoriteId) === userId);
  const named = new Map(list(all.groups).map((g) => [str(g.name), str(g.displayName) || str(g.name)]));
  const keys = [...new Set([...standardGroups, ...named.keys()])].sort();
  const count = (key: string) => entries.filter((e) => str((e.tags as unknown[])?.[0]) === key).length;
  const label = (key: string) => prettyGroup(named.get(key) ?? key);
  const picked = group || keys.find((k) => !capacity || count(k) < capacity) || keys[0];

  const current = mine ? str((mine.tags as unknown[])?.[0]) : "";
  const others = keys.filter((k) => k !== current);
  const moveTo = group && group !== current ? group : (others.find((k) => !capacity || count(k) < capacity) ?? others[0] ?? "");

  async function act(job: () => Promise<unknown>, done: string) {
    setBusy(true);
    try {
      await job();
      onDone(done);
      await load(true);
    } catch (err) {
      onDone(`That did not work: ${errorText(err)}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section>
      <h3>Favourite</h3>
      {mine ? (
        <div className="fav-card">
          <div className="fav-now">
            <span className="fav-in">In {label(current)}</span>
            <button className="plain" disabled={busy} onClick={() => void act(() => backend.removeFavorites([str(mine.id)]), `${name} is no longer a favourite.`)}>
              Remove
            </button>
          </div>
          {others.length > 0 && (
            <>
              <span className="fav-label">Move to another group</span>
              <div className="fav-pick">
                <select value={moveTo} onChange={(e) => setGroup(e.target.value)} aria-label="Move to favourite group" disabled={busy}>
                  {others.map((k) => (
                    <option key={k} value={k} disabled={capacity > 0 && count(k) >= capacity}>
                      {label(k)}
                      {capacity ? ` (${count(k)}/${capacity})` : ""}
                    </option>
                  ))}
                </select>
                <button
                  className="plain go"
                  disabled={busy || !moveTo || (capacity > 0 && count(moveTo) >= capacity)}
                  title="VRChat keeps a friend in one favourite group, so this takes them out of the old one and puts them in the new one"
                  onClick={() =>
                    void act(async () => {
                      await backend.removeFavorites([str(mine.id)]);
                      try {
                        await backend.addFavorite("friend", userId, moveTo);
                      } catch (err) {
                        // Put back where they were, so a failed move does not drop the favourite.
                        await backend.addFavorite("friend", userId, current).catch(() => {});
                        throw err;
                      }
                    }, `${name} moved to ${label(moveTo)}.`)
                  }
                >
                  Move
                </button>
              </div>
            </>
          )}
        </div>
      ) : (
        <div className="fav-pick">
          <select value={picked} onChange={(e) => setGroup(e.target.value)} aria-label="Favourite group">
            {keys.map((k) => (
              <option key={k} value={k} disabled={capacity > 0 && count(k) >= capacity}>
                {label(k)}
                {capacity ? ` (${count(k)}/${capacity})` : ""}
              </option>
            ))}
          </select>
          <button className="plain go" disabled={busy} onClick={() => void act(() => backend.addFavorite("friend", userId, picked), `${name} is now a favourite. They are pinned on Around.`)}>
            Add
          </button>
        </div>
      )}
    </section>
  );
}
