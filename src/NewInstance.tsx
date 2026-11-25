import { useState } from "react";
import { backend, errorText } from "./backend";
import { WayBack } from "./History";

const kinds: Array<[string, string, string]> = [
  ["invitePlus", "Invite+", "Only people you invite can join, and anyone inside can be asked for an invite"],
  ["invite", "Invite", "Only people you invite can join"],
  ["friends", "Friends", "Your friends can join"],
  ["friendsPlus", "Friends+", "Your friends can join, and so can friends of anyone inside"],
  ["public", "Public", "Anyone can join"],
];

const regions: Array<[string, string]> = [
  ["us", "US West"],
  ["use", "US East"],
  ["eu", "Europe"],
  ["jp", "Japan"],
];

const REGION_KEY = "new-instance-region";
const KIND_KEY = "new-instance-kind";

function saved(key: string, fallback: string): string {
  try {
    return localStorage.getItem(key) || fallback;
  } catch {
    return fallback;
  }
}

function keep(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Remembering the choice is a nicety; the instance is made either way.
  }
}

/** Makes a fresh instance of a world and offers the ways into it. */
export function NewInstance({ worldId, onDone }: { worldId: string; onDone: (message: string) => void }) {
  const [kind, setKind] = useState(() => saved(KIND_KEY, "invitePlus"));
  const [region, setRegion] = useState(() => saved(REGION_KEY, "us"));
  const [busy, setBusy] = useState(false);
  const [location, setLocation] = useState("");
  const [problem, setProblem] = useState("");

  async function create() {
    setBusy(true);
    setProblem("");
    try {
      setLocation(await backend.createInstance(worldId, kind, region));
      keep(KIND_KEY, kind);
      keep(REGION_KEY, region);
    } catch (err) {
      setProblem(errorText(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section>
      <h3>New instance</h3>
      <div className="new-instance">
        <label className="tool">
          Who can join
          <select value={kind} onChange={(e) => setKind(e.target.value)} disabled={busy}>
            {kinds.map(([id, label, hint]) => (
              <option key={id} value={id} title={hint}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <label className="tool">
          Region
          <select value={region} onChange={(e) => setRegion(e.target.value)} disabled={busy}>
            {regions.map(([id, label]) => (
              <option key={id} value={id}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <button className="plain go" disabled={busy} onClick={() => void create()}>
          {busy ? "Making it" : location ? "Make another" : "Make instance"}
        </button>
      </div>
      <p className="profile-fine">{kinds.find(([id]) => id === kind)?.[2]}.</p>
      {problem && (
        <p className="banner" role="alert">
          {problem}
        </p>
      )}
      {location && (
        <>
          <p className="profile-fine">Your instance is ready. It stays open while someone is in it, so join soon.</p>
          <WayBack location={location} onDone={onDone} />
        </>
      )}
    </section>
  );
}
