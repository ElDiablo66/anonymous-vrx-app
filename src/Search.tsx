import { useState, type FormEvent } from "react";
import { backend, errorText, statusDot, type AvatarCard, type Person, type WorldCard } from "./backend";
import { MarkBadges, useWorldMarks } from "./Mine";
import { openThing } from "./Detail";
import { savedAvatarProviders } from "./settings";
import { Avatar, tidy } from "./bits";
import { openProfile } from "./Profile";

type Kind = "people" | "worlds" | "avatars";

const hints: Record<Kind, string> = {
  people: "A display name",
  worlds: "A world name",
  avatars: "An avatar or author name",
};

export function Search({ onDone }: { onDone: (message: string) => void }) {
  const [kind, setKind] = useState<Kind>("people");
  const [query, setQuery] = useState("");
  const [asked, setAsked] = useState("");
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState("");
  const [people, setPeople] = useState<Person[]>([]);
  const [worlds, setWorlds] = useState<WorldCard[]>([]);
  const marks = useWorldMarks();
  const [newOnly, setNewOnly] = useState(false);
  const [avatars, setAvatars] = useState<AvatarCard[]>([]);
  const [wearing, setWearing] = useState("");

  // One request per press of Search, never one per keystroke.
  async function run(e: FormEvent) {
    e.preventDefault();
    const text = query.trim();
    if (text.length < 2 || busy) return;
    setBusy(true);
    setProblem("");
    try {
      if (kind === "people") setPeople(await backend.searchUsers(text));
      else if (kind === "worlds") setWorlds(await backend.searchWorlds(text));
      else {
        // Every database is asked; one that is down does not spoil the others.
        const answers = await Promise.allSettled(savedAvatarProviders().map((p) => backend.searchAvatars(p, text)));
        const good = answers.filter((a) => a.status === "fulfilled");
        if (good.length === 0) throw (answers[0] as PromiseRejectedResult).reason;
        const seen = new Set<string>();
        setAvatars(good.flatMap((a) => a.value).filter((a) => !seen.has(a.id) && !!seen.add(a.id)));
      }
      setAsked(`${kind}:${text}`);
    } catch (err) {
      setProblem(errorText(err));
    } finally {
      setBusy(false);
    }
  }

  const answered = asked.startsWith(`${kind}:`);
  const count = kind === "people" ? people.length : kind === "worlds" ? worlds.length : avatars.length;

  async function wear(a: AvatarCard) {
    setWearing(a.id);
    try {
      await backend.wearAvatar(a.id);
      onDone(`You are now wearing ${a.name}.`);
    } catch (err) {
      onDone(`VRChat did not switch your avatar: ${errorText(err)}`);
    } finally {
      setWearing("");
    }
  }

  return (
    <div className="table-wrap">
      <form className="search-bar" onSubmit={run}>
        <div className="tabs" role="tablist" aria-label="Search for">
          <button type="button" role="tab" aria-selected={kind === "people"} className={kind === "people" ? "on" : ""} onClick={() => setKind("people")}>
            People
          </button>
          <button type="button" role="tab" aria-selected={kind === "worlds"} className={kind === "worlds" ? "on" : ""} onClick={() => setKind("worlds")}>
            Worlds
          </button>
          <button type="button" role="tab" aria-selected={kind === "avatars"} className={kind === "avatars" ? "on" : ""} onClick={() => setKind("avatars")}>
            Avatars
          </button>
        </div>
        <input
          className="search"
          type="search"
          autoFocus
          placeholder={hints[kind]}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          aria-label="Search for"
        />
        <button className="plain go" disabled={busy || query.trim().length < 2}>
          {busy ? "Searching" : "Search"}
        </button>
      </form>

      {problem && (
        <p className="banner" role="alert">
          {problem}
        </p>
      )}
      {!answered && !problem && (
        <p className="empty">
          {kind === "people"
            ? "Search VRChat for anyone by display name. Click a result to open their profile."
            : kind === "worlds"
              ? "Search VRChat's worlds by name."
              : "Search public avatars by name or author. This asks a community-run avatar database, not VRChat; the address is in Settings."}
        </p>
      )}
      {answered && count === 0 && <p className="empty">Nothing found for "{asked.slice(kind.length + 1)}".</p>}

      {kind === "people" && answered && people.length > 0 && (
        <ul className="results">
          {people.map((p) => (
            <li key={p.id}>
              <button className="result" onClick={() => openProfile(p.id)}>
                <Avatar name={p.displayName} image={p.image} tone={p.status ? statusDot(p.status) : undefined} size={44} />
                <span className="result-text">
                  <span className="result-name">{p.displayName}</span>
                  <span className="result-note">
                    <span className={`rank rank-${p.rank.replace(/ /g, "")}`}>{p.rank}</span>
                    {p.isFriend && <span className="rank rank-friend">Friend</span>}
                  </span>
                  {(p.status || p.statusDescription) && (
                    <span className="result-note">{[p.status && p.status !== "offline" ? p.status : "", p.statusDescription].filter(Boolean).join(", ") || "offline"}</span>
                  )}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}

      {kind === "avatars" && answered && avatars.length > 0 && (
        <ul className="results results-avatars">
          {avatars.map((a) => (
            <li key={a.id} className="avatar-result">
              <button className="tile-open" onClick={() => openThing("avatar", a.id)} title="Open this avatar">
                <div className="room-pic" style={a.image ? { backgroundImage: `url("${a.image}")` } : undefined} />
                <span className="result-name">{tidy(a.name) || "Unnamed avatar"}</span>
              </button>
              <button className="name-link result-note who" onClick={() => a.authorId && openProfile(a.authorId)} title="Open the author's profile">
                by {a.author || "unknown"}
              </button>
              <span className="result-note">
                {[a.pc && `PC ${a.pc}`, a.mobile && `Quest ${a.mobile}`].filter(Boolean).join(", ") || "Performance unknown"}
              </span>
              <div className="wayback">
                <button className="go" disabled={wearing === a.id} onClick={() => void wear(a)} title="Switches your VRChat avatar to this one">
                  Wear
                </button>
                <button onClick={() => void backend.openLink(`https://vrchat.com/home/avatar/${a.id}`).catch((err) => onDone(errorText(err)))}>Website</button>
                <button
                  onClick={() => {
                    void navigator.clipboard.writeText(a.id).then(
                      () => onDone("Avatar ID copied."),
                      () => onDone("Could not copy."),
                    );
                  }}
                >
                  Copy ID
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}

      {kind === "worlds" && answered && worlds.length > 0 && (
        <ul className="results results-worlds">
          <li className="results-tool">
            <label className="tool">
              <input type="checkbox" checked={newOnly} onChange={(e) => setNewOnly(e.target.checked)} />
              Only worlds I have not been to
            </label>
          </li>
          {worlds.filter((w) => !newOnly || !marks.visits[w.id]).map((w) => (
            <li key={w.id} className="world-result">
              <button className="tile-open" onClick={() => openThing("world", w.id)} title="Open this world">
                <div className="room-pic" style={w.image ? { backgroundImage: `url("${w.image}")` } : undefined} />
              </button>
              <div className="result-text">
                <button className="name-link result-name" onClick={() => openThing("world", w.id)}>
                  {tidy(w.name)}
                </button>
                <MarkBadges id={w.id} marks={marks} />
                <span className="result-note">
                  by {w.author || "unknown"}, up to {w.capacity} people, {w.visits.toLocaleString()} visits
                  {w.occupants > 0 ? `, ${w.occupants} in public instances now` : ""}
                </span>
                {w.tags.length > 0 && <span className="result-note">{w.tags.slice(0, 6).join(", ")}</span>}
                <div className="wayback">
                  <button onClick={() => void backend.openLink(`https://vrchat.com/home/world/${w.id}`).catch((err) => onDone(errorText(err)))} title="Opens the world's page on VRChat's website, where you can start an instance">
                    Open on the website
                  </button>
                  <button
                    onClick={() => {
                      void navigator.clipboard.writeText(`https://vrchat.com/home/world/${w.id}`).then(
                        () => onDone("World link copied."),
                        () => onDone("Could not copy the link."),
                      );
                    }}
                  >
                    Copy link
                  </button>
                </div>
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
