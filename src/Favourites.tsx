import { GroupNames } from "./GroupNames";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { backend, errorText, type FavouriteCheck, type Favorites, type Friend, type LocalFavorite } from "./backend";
import { Avatar, statusTone, tidy } from "./bits";
import { openThing } from "./Detail";
import { openProfile } from "./Profile";
import { openPicture } from "./Viewer";
import { ALL_TAGS, MarkBadges, TagChips, TagFilter, passesTag, useTags, useWorldMarks } from "./Mine";

type Kind = "friend" | "world" | "avatar";
type Json = Record<string, unknown>;

const str = (v: unknown): string => (typeof v === "string" ? v : "");
const list = (v: unknown): Json[] => (Array.isArray(v) ? (v.filter((x) => x && typeof x === "object") as Json[]) : []);

interface Loaded {
  groups: Json[];
  entries: Json[];
  items: Json[];
  /** How many favourites one group holds. */
  capacity: number;
}

const lastSeen = new Map<Kind, Loaded>();

/** A group heading that can be renamed in place. */
function GroupTitle({
  kind,
  group,
  label,
  children,
  onDone,
  onRenamed,
}: {
  kind: Kind;
  group: string;
  label: string;
  children: ReactNode;
  onDone: (message: string) => void;
  onRenamed: () => void;
}) {
  const [text, setText] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  if (text === null) {
    return (
      <h2>
        {label} {children}
        <button className="fav-rename" onClick={() => setText(label)} title="Give this group your own name">
          Rename
        </button>
      </h2>
    );
  }
  const save = () => {
    setBusy(true);
    void backend
      .renameFavoriteGroup(kind, group, text)
      .then(() => {
        onDone(`Group renamed to ${text.trim()}.`);
        setText(null);
        onRenamed();
      })
      .catch((err) => onDone(`The name was not saved: ${errorText(err)}`))
      .finally(() => setBusy(false));
  };
  return (
    <form
      className="fav-rename-form"
      onSubmit={(e) => {
        e.preventDefault();
        save();
      }}
    >
      <input value={text} maxLength={24} autoFocus onChange={(e) => setText(e.target.value)} aria-label="Group name" />
      <button className="plain go" disabled={busy || !text.trim()}>
        Save
      </button>
      <button type="button" className="plain" onClick={() => setText(null)}>
        Cancel
      </button>
    </form>
  );
}

/** VRChat's own names like `avatars2` read better as "Avatars 2". */
export function prettyGroup(name: string): string {
  const m = /^(avatars|worlds|group_)(\d+)$/.exec(name);
  if (!m) return name;
  const word = m[1] === "avatars" ? "Avatars" : m[1] === "worlds" ? "Worlds" : "Group";
  return `${word} ${m[1] === "group_" ? Number(m[2]) + 1 : m[2]}`;
}

export function Favourites({
  friends,
  favorites,
  onDone,
}: {
  friends: Map<string, Friend>;
  favorites: Favorites;
  meId: string;
  onDone: (message: string) => void;
}) {
  const [kind, setKind] = useState<Kind>("avatar");
  const [naming, setNaming] = useState(false);
  const marks = useWorldMarks();
  const [mine, setMine] = useState<"all" | "top" | "unrated" | "been" | "new">("all");
  const tags = useTags();
  const [tag, setTag] = useState(ALL_TAGS);
  const passes = (id: string) =>
    kind === "avatar" ? passesTag(id, tags, tag) :
    kind !== "world" ||
    mine === "all" ||
    (mine === "top" && (marks.ratings[id] ?? 0) >= 4) ||
    (mine === "unrated" && !marks.ratings[id]) ||
    (mine === "been" && !!marks.visits[id]) ||
    (mine === "new" && !marks.visits[id]);
  const [data, setData] = useState<Loaded | null>(null);
  const [problem, setProblem] = useState("");
  const [busy, setBusy] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const [tick, setTick] = useState(0);
  const [check, setCheck] = useState<FavouriteCheck | "busy" | null>(null);
  const [local, setLocal] = useState<LocalFavorite[]>([]);

  useEffect(() => {
    let alive = true;
    setLocal([]);
    if (kind === "friend") return;
    void backend
      .localFavorites(kind)
      .then((list) => alive && setLocal(list))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [kind, tick]);

  useEffect(() => {
    let alive = true;
    // What was shown last time comes up at once; the fresh answer replaces it when it arrives.
    setData(lastSeen.get(kind) ?? null);
    setProblem("");
    setConfirm(false);
    setCheck(null);
    if (kind === "friend") return;
    const read = (d: unknown): Loaded => {
      const all = (d ?? {}) as Json;
      return { groups: list(all.groups), entries: list(all.entries), items: list(all.items), capacity: typeof all.capacity === "number" ? all.capacity : 0 };
    };
    void backend
      .favoritesCached(kind)
      .then((d) => alive && d && setData((old) => old ?? read(d)))
      .catch(() => {});
    void backend
      .favoritesOf(kind)
      .then((d) => {
        const fresh = read(d);
        lastSeen.set(kind, fresh);
        if (alive) setData(fresh);
      })
      .catch((err) => alive && setProblem(errorText(err)));
    return () => {
      alive = false;
    };
  }, [kind, tick]);

  const byId = useMemo(() => new Map((data?.items ?? []).map((i) => [str(i.id), i])), [data]);

  /** Favourites VRChat confirmed are gone: deleted, or made private by their author. */
  const deadIds = useMemo(
    () => new Set(check && check !== "busy" ? check.dead.map((d) => d.entryId) : []),
    [check],
  );
  const dead = data ? data.entries.filter((e) => deadIds.has(str(e.id))) : [];

  async function runCheck() {
    setCheck("busy");
    setConfirm(false);
    try {
      setCheck(await backend.checkFavorites(kind));
    } catch (err) {
      setCheck(null);
      onDone(`The check did not finish: ${errorText(err)}`);
    }
  }

  async function remove(entryIds: string[], done: string) {
    setBusy(true);
    try {
      const count = await backend.removeFavorites(entryIds);
      onDone(count === entryIds.length ? done : `Removed ${count} of ${entryIds.length}. VRChat asked to slow down; try again in a minute.`);
      setCheck(null);
      setTick((n) => n + 1);
    } catch (err) {
      onDone(`That did not work: ${errorText(err)}`);
    } finally {
      setBusy(false);
      setConfirm(false);
    }
  }

  const groupName = (key: string) => {
    const g = data?.groups.find((x) => str(x.name) === key);
    return prettyGroup((g && str(g.displayName)) || key);
  };
  // Every group, the empty ones too, so it is clear where there is room.
  const groupKeys = data
    ? [...new Set([...data.groups.map((g) => str(g.name)), ...data.entries.map((e) => str((e.tags as unknown[])?.[0]))])].filter(Boolean).sort()
    : [];

  return (
    <div className="table-wrap">
      <div className="tabs" role="tablist" aria-label="Favourites">
        {(
          [
            ["avatar", "Avatars"],
            ["world", "Worlds"],
            ["friend", "Friends"],
          ] as const
        ).map(([key, label]) => (
          <button
            key={key}
            role="tab"
            aria-selected={!naming && kind === key}
            className={!naming && kind === key ? "on" : ""}
            onClick={() => {
              setNaming(false);
              setKind(key);
            }}
          >
            {label}
          </button>
        ))}
        <button role="tab" aria-selected={naming} className={naming ? "on" : ""} onClick={() => setNaming(true)}>
          Group names
        </button>
      </div>
      {naming && <GroupNames onDone={onDone} />}
      <div className="fav-rest" hidden={naming}>

      {problem && (
        <p className="banner" role="alert">
          {problem}
        </p>
      )}

      {kind === "friend" &&
        (favorites.groups.length === 0 ? (
          <p className="empty">No favourite friends yet. Add them in VRChat and they are pinned on the Around screen.</p>
        ) : (
          favorites.groups.map((g) => {
            const members = Object.entries(favorites.friends)
              .filter(([, key]) => key === g.key)
              .map(([id]) => friends.get(id))
              .filter((f): f is Friend => !!f)
              .sort((a, b) => a.displayName.localeCompare(b.displayName));
            return (
              <section className="fav-group" key={g.key}>
                <GroupTitle kind="friend" group={g.key} label={g.name} onDone={onDone} onRenamed={() => {}}>
                  <span className="fav-count">{members.length}</span>
                </GroupTitle>
                <ul className="chips">
                  {members.map((f) => (
                    <li className="chip" data-person={f.id} data-person-name={f.displayName} role="button" tabIndex={0} key={f.id} onClick={() => openProfile(f.id)} onKeyDown={(e) => e.key === "Enter" && openProfile(f.id)}>
                      <Avatar name={f.displayName} image={f.image} tone={statusTone(f)} />
                      <span className="chip-text">
                        <span className="chip-name">{f.displayName}</span>
                      </span>
                    </li>
                  ))}
                </ul>
              </section>
            );
          })
        ))}

      {kind !== "friend" && !data && !problem && <p className="empty">Asking VRChat for your favourites</p>}

      {kind !== "friend" && data && data.entries.length > 0 && (
        <section className="cleanup">
          <div>
            {check === null && (
              <>
                <b>Clean up</b>
                <p>
                  Checks each favourite {kind} VRChat did not list with VRChat itself. Only ones it says were deleted,
                  or made private by their author, are offered for removal.
                </p>
              </>
            )}
            {check === "busy" && <b>Asking VRChat about each one</b>}
            {check && check !== "busy" && (
              <>
                <b>
                  {dead.length === 0
                    ? `Nothing gone. All your favourite ${kind}s still work.`
                    : `${dead.length} favourite ${kind}${dead.length === 1 ? " is" : "s are"} gone`}
                </b>
                <p>
                  {dead.length > 0 && "Deleted, or made private by their author. Removing them frees the slots; it cannot be undone. "}
                  {check.left > 0 && `${check.left} more could not be checked yet${check.stopped ? " because VRChat asked to slow down" : ""}. Run the check again in a minute.`}
                </p>
              </>
            )}
          </div>
          {check === null || check === "busy" ? (
            <button className="plain" disabled={check === "busy"} onClick={() => void runCheck()}>
              {check === "busy" ? "Checking" : "Check for gone ones"}
            </button>
          ) : dead.length === 0 ? null : confirm ? (
            <span className="cleanup-ask">
              Remove all {dead.length}?
              <button className="plain go" disabled={busy} onClick={() => void remove(dead.map((e) => str(e.id)), `Removed ${dead.length} dead favourites.`)}>
                {busy ? "Removing" : "Yes, remove them"}
              </button>
              <button className="plain" disabled={busy} onClick={() => setConfirm(false)}>
                No
              </button>
            </span>
          ) : (
            <button className="plain" onClick={() => setConfirm(true)}>
              Remove {dead.length}
            </button>
          )}
        </section>
      )}

      {kind !== "friend" && local.length > 0 && (
        <section className="fav-group">
          <h2>
            On this PC <span className="fav-count">{local.length}</span>
          </h2>
          <ul className="fav-list">
            {local.map((f) => (
              <li key={f.itemId} className="fav-item">
                <button className="pic-button" disabled={!f.image} onClick={() => openPicture(f.image, tidy(f.name))} title="Show the picture full size" aria-label="Show the picture full size">
                  <span className="thumb" style={f.image ? { backgroundImage: `url("${f.image}")` } : undefined} />
                </button>
                <button className="row-link" onClick={() => openThing(kind, f.itemId)} title="Open">
                  <span>
                    <span className="tile-name">{tidy(f.name) || f.itemId}</span>
                    <span className="tile-note">{f.authorName}</span>
                  </span>
                </button>
                <button
                  className="fav-remove"
                  title="Remove from this PC"
                  aria-label="Remove from this PC"
                  onClick={() =>
                    void backend
                      .removeLocalFavorite(kind, f.itemId)
                      .then(() => {
                        onDone("Removed from this PC.");
                        setTick((n) => n + 1);
                      })
                      .catch((err) => onDone(errorText(err)))
                  }
                >
                  Remove
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}

      {kind !== "friend" && data && data.entries.length === 0 && (
        <p className="empty">
          No favourite {kind === "world" ? "worlds" : "avatars"} yet. Open one and press "Add to favourites".
        </p>
      )}

      {kind === "world" && data && data.entries.length > 0 && (
        <div className="list-tools">
          <label className="tool">
            Show
            <select value={mine} onChange={(e) => setMine(e.target.value as typeof mine)}>
              <option value="all">Every favourite world</option>
              <option value="top">Rated 4 or 5 by me</option>
              <option value="unrated">Not rated yet</option>
              <option value="been">Been there</option>
              <option value="new">Not been there</option>
            </select>
          </label>
        </div>
      )}

      {kind === "avatar" && data && <TagFilter ids={data.entries.map((e) => str(e.favoriteId))} tags={tags} value={tag} set={setTag} />}

      {kind !== "friend" &&
        data &&
        groupKeys.map((key) => {
          const inGroup = data.entries.filter((e) => str((e.tags as unknown[])?.[0]) === key);
          const entries = inGroup.filter((e) => passes(str(e.favoriteId)));
          return (
            <section className="fav-group" key={key}>
              <GroupTitle kind={kind} group={key} label={groupName(key)} onDone={onDone} onRenamed={() => setTick((n) => n + 1)}>
                <span className={`fav-count${data.capacity && inGroup.length >= data.capacity ? " fav-full" : ""}`}>
                  {inGroup.length}
                  {data.capacity ? `/${data.capacity}` : ""}
                  {data.capacity && inGroup.length >= data.capacity ? ", full" : ""}
                </span>
              </GroupTitle>
              {inGroup.length === 0 && <p className="tile-note">Empty. Room for {data.capacity || "more"}.</p>}
              {inGroup.length > 0 && entries.length === 0 && <p className="tile-note">None in this group match what is shown.</p>}
              <ul className="fav-list">
                {entries.map((e) => {
                  const item = byId.get(str(e.favoriteId));
                  const gone = deadIds.has(str(e.id));
                  const pic = item ? str(item.thumbnailImageUrl) || str(item.imageUrl) : "";
                  return (
                    <li key={str(e.id)} className={gone ? "fav-item fav-gone" : "fav-item"}>
                      <button className="pic-button" disabled={!pic} onClick={() => openPicture(pic, tidy(str(item?.name)))} title="Show the picture full size" aria-label="Show the picture full size">
                        <span className="thumb" style={pic ? { backgroundImage: `url("${pic}")` } : undefined} />
                      </button>
                      <button className="row-link" onClick={() => openThing(kind, str(e.favoriteId))} title="Open">
                        <span>
                          <span className="tile-name">{item ? tidy(str(item.name)) : gone ? "Gone" : "Not listed by VRChat"}</span>
                          {kind === "world" && <MarkBadges id={str(e.favoriteId)} marks={marks} />}
                          {kind === "avatar" && <TagChips id={str(e.favoriteId)} tags={tags} />}
                          <span className="tile-note">
                            {item ? `${str(item.authorName)}${gone ? ", now private" : ""}` : gone ? "Deleted or made private" : "Open it to see if it still works"}
                          </span>
                        </span>
                      </button>
                      <button className="fav-remove" disabled={busy} title="Remove from favourites" aria-label="Remove from favourites" onClick={() => void remove([str(e.id)], "Removed from favourites.")}>
                        Remove
                      </button>
                    </li>
                  );
                })}
              </ul>
            </section>
          );
        })}
      </div>
    </div>
  );
}
