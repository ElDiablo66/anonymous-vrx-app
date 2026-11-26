import { useEffect, useState } from "react";
import { backend, errorText } from "./backend";

type Kind = "friend" | "avatar" | "world";

const str = (v: unknown) => (typeof v === "string" ? v : "");

const kinds: Array<[Kind, string, string[]]> = [
  ["friend", "Friend groups", ["group_0", "group_1", "group_2", "group_3"]],
  ["avatar", "Avatar groups", ["avatars1", "avatars2", "avatars3", "avatars4"]],
  ["world", "World groups", ["worlds1", "worlds2", "worlds3", "worlds4"]],
];

interface Row {
  key: string;
  /** The name VRChat has for it now, empty when it was never named. */
  saved: string;
  /** Nothing while VRChat has not said yet. */
  count: number | null;
}

/** What was shown last time, so opening the tab again is instant. */
let lastRows: Record<Kind, Row[]> | null = null;
// Another account signed in: these were the groups of the one before.
window.addEventListener("account:changed", () => {
  lastRows = null;
});

/** VRChat's own names like `avatars2` read better as "Avatars 2". */
function plain(key: string): string {
  const m = /^([a-z]+?)_?(\d+)$/.exec(key);
  if (!m) return key;
  const word = m[1] === "group" ? "Group" : m[1][0].toUpperCase() + m[1].slice(1);
  return `${word} ${m[1] === "group" ? Number(m[2]) + 1 : m[2]}`;
}

/** Every favourite group of mine in one place, to give each its own name. */
export function GroupNames({ onDone }: { onDone: (message: string) => void }) {
  const [rows, setRows] = useState<Record<Kind, Row[]> | null>(lastRows);
  const [typed, setTyped] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState("");
  const [problem, setProblem] = useState("");

  /** Turns VRChat's answer into the three lists. Without counts, the last known ones are kept. */
  function build(answer: { groups: Array<Record<string, unknown>>; counts: Record<string, number> }, counted: boolean): Record<Kind, Row[]> {
    const out = {} as Record<Kind, Row[]>;
    for (const [kind, , standard] of kinds) {
      const mine = answer.groups.filter((g) => str(g.type) === kind);
      const used = Object.keys(answer.counts)
        .filter((k) => k.startsWith(`${kind}:`))
        .map((k) => k.slice(kind.length + 1));
      const keys = [...new Set([...standard, ...mine.map((g) => str(g.name)), ...used])].filter(Boolean).sort();
      out[kind] = keys.map((key) => {
        const shown = str(mine.find((g) => str(g.name) === key)?.displayName);
        const before = lastRows?.[kind].find((r) => r.key === key)?.count ?? null;
        return { key, saved: shown && shown !== key ? shown : "", count: counted ? (answer.counts[`${kind}:${key}`] ?? 0) : before };
      });
    }
    return out;
  }

  // The names come first, in one quick question; the counts follow when VRChat has answered.
  useEffect(() => {
    let alive = true;
    const show = (next: Record<Kind, Row[]>) => {
      if (!alive) return;
      lastRows = next;
      setRows(next);
    };
    backend
      .favoriteGroups(false)
      .then((names) => show(build(names, false)))
      .catch((err) => alive && !lastRows && setProblem(errorText(err)));
    backend
      .favoriteGroups(true)
      .then((full) => show(build(full, true)))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  async function save(kind: Kind, row: Row) {
    const id = `${kind}:${row.key}`;
    const name = (typed[id] ?? "").trim();
    if (!name) return;
    setBusy(id);
    try {
      await backend.renameFavoriteGroup(kind, row.key, name);
      onDone(`Group renamed to ${name}.`);
      // The new name is shown at once; nothing has to be asked again.
      setRows((old) => {
        const next = old ? { ...old, [kind]: old[kind].map((r) => (r.key === row.key ? { ...r, saved: name } : r)) } : old;
        lastRows = next;
        return next;
      });
      setTyped((old) => {
        const next = { ...old };
        delete next[id];
        return next;
      });
    } catch (err) {
      onDone(`The name was not saved: ${errorText(err)}`);
    } finally {
      setBusy("");
    }
  }

  if (problem) {
    return (
      <p className="banner" role="alert">
        {problem}
      </p>
    );
  }
  if (!rows) return <p className="empty">Asking VRChat for your favourite groups</p>;

  return (
    <div className="group-names">
      <p className="tile-note">
        Names are kept by VRChat, so they show in the game and on the website too. Up to 24 characters. Type a new name and press Save.
      </p>
      <div className="group-names-cols">
        {kinds.map(([kind, title]) => (
          <section key={kind} className="group-names-card">
            <h2>{title}</h2>
            <ul>
              {rows[kind].map((row) => {
                const id = `${kind}:${row.key}`;
                const now = row.saved || plain(row.key);
                const text = typed[id] ?? "";
                return (
                  <li key={row.key}>
                    <form
                      onSubmit={(e) => {
                        e.preventDefault();
                        void save(kind, row);
                      }}
                    >
                      <span className="group-now">
                        <b>{now}</b>
                        <small>
                          {row.count === null ? "counting" : `${row.count} ${row.count === 1 ? "favourite" : "favourites"}`}
                        </small>
                      </span>
                      <input value={text} maxLength={24} placeholder="New name" aria-label={`New name for ${now}`} onChange={(e) => setTyped((old) => ({ ...old, [id]: e.target.value }))} />
                      <button className="plain go" disabled={busy === id || !text.trim() || text.trim() === now}>
                        Save
                      </button>
                    </form>
                  </li>
                );
              })}
            </ul>
          </section>
        ))}
      </div>
    </div>
  );
}
