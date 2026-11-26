import { useEffect, useMemo, useState } from "react";
import { backend, errorText } from "./backend";
import { tidy } from "./bits";
import { openThing } from "./Detail";
import { usePic } from "./Pic";
import { ALL_TAGS, TagChips, TagEditor, TagFilter, liveTag, passesTag, useTags } from "./Mine";

type Kind = "avatars" | "worlds";
type Json = Record<string, unknown>;

const str = (v: unknown): string => (typeof v === "string" ? v : "");
const num = (v: unknown): number => (typeof v === "number" ? v : 0);
const list = (v: unknown): Json[] => (Array.isArray(v) ? (v.filter((x) => x && typeof x === "object") as Json[]) : []);

function when(iso: string): string {
  return iso ? new Date(iso).toLocaleDateString([], { day: "numeric", month: "short", year: "numeric" }) : "";
}

function Picture({ url }: { url: string }) {
  const shown = usePic(url);
  return <span className="thumb" style={url ? { backgroundImage: `url("${shown}")` } : undefined} />;
}

/** Everything I uploaded to VRChat: my avatars and my worlds, private ones included. */
export function Uploads({ meId, onDone }: { meId: string; onDone: (message: string) => void }) {
  const [kind, setKind] = useState<Kind>("avatars");
  const [recovery, setRecovery] = useState(false);
  const [rows, setRows] = useState<Record<string, Json[] | null>>({});
  const [problem, setProblem] = useState("");
  const [query, setQuery] = useState("");
  const [only, setOnly] = useState<"all" | "public" | "private">("all");
  const [busy, setBusy] = useState("");
  const tags = useTags();
  const [pickedTag, setTag] = useState(ALL_TAGS);
  /** The avatar whose tags are being edited. */
  const [tagging, setTagging] = useState("");
  /** The upload whose Delete was pressed once and waits for the second click. */
  const [deleting, setDeleting] = useState("");

  async function change(id: string, name: string, action: "public" | "private" | "delete") {
    setBusy(id);
    try {
      await backend.uploadAction(kind === "avatars" ? "avatar" : "world", id, action);
      // The list on screen follows at once; VRChat's own list catches up a little later.
      setRows((old) => ({
        ...old,
        [kind]: (old[kind] ?? []).flatMap((r) => (str(r.id) !== id ? [r] : action === "delete" ? [] : [{ ...r, releaseStatus: action }])),
      }));
      onDone(action === "delete" ? `${name} is deleted.` : `${name} is now ${action}.`);
    } catch (err) {
      onDone(`That did not work: ${errorText(err)}`);
    } finally {
      setBusy("");
      setDeleting("");
    }
  }

  useEffect(() => {
    if (kind in rows) return;
    let alive = true;
    setProblem("");
    void backend
      .profilePart(meId, kind, "", "")
      .then((d) => alive && setRows((old) => ({ ...old, [kind]: list(d) })))
      .catch((err) => {
        if (!alive) return;
        setRows((old) => ({ ...old, [kind]: null }));
        setProblem(errorText(err));
      });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kind, meId]);

  const all = rows[kind];
  const tag = liveTag((rows.avatars ?? []).map((r) => str(r.id)), tags, pickedTag);
  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (all ?? []).filter(
      (r) =>
        (only === "all" || str(r.releaseStatus) === only) &&
        // A search also finds an avatar by one of my tags.
        (!q || str(r.name).toLowerCase().includes(q) || (tags[str(r.id)] ?? []).some((t) => t.toLowerCase().includes(q))) &&
        (kind !== "avatars" || passesTag(str(r.id), tags, tag)),
    );
  }, [all, query, only, tags, tag, kind]);

  const copy = (text: string, done: string) =>
    void navigator.clipboard.writeText(text).then(
      () => onDone(done),
      () => onDone("Could not copy."),
    );

  async function wear(id: string, name: string) {
    setBusy(id);
    try {
      await backend.wearAvatar(id);
      onDone(`You are now wearing ${name}.`);
    } catch (err) {
      onDone(`That did not work: ${errorText(err)}`);
    } finally {
      setBusy("");
    }
  }

  return (
    <div className="table-wrap">
      <div className="tabs" role="tablist" aria-label="My uploads">
        {(
          [
            ["avatars", "Avatars"],
            ["worlds", "Worlds"],
          ] as const
        ).map(([id, label]) => (
          <button
            key={id}
            role="tab"
            aria-selected={!recovery && kind === id}
            className={!recovery && kind === id ? "on" : ""}
            onClick={() => {
              setRecovery(false);
              setKind(id);
            }}
          >
            {label}
            {rows[id] ? `, ${rows[id]!.length}` : ""}
          </button>
        ))}
      </div>

      <div style={recovery ? { display: "none" } : undefined}>

      <div className="list-tools">
        <input type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder={`Find one of your ${kind}`} aria-label={`Find one of your ${kind}`} />
        <label className="tool">
          Show
          <select value={only} onChange={(e) => setOnly(e.target.value as typeof only)}>
            <option value="all">Everything</option>
            <option value="public">Public</option>
            <option value="private">Private</option>
          </select>
        </label>
      </div>

      {kind === "avatars" && all && <TagFilter ids={all.map((r) => str(r.id))} tags={tags} value={tag} set={setTag} />}

      {problem && (
        <p className="banner" role="alert">
          VRChat did not send your {kind}: {problem}
        </p>
      )}
      {all === undefined && <p className="empty">Asking VRChat for your {kind}</p>}
      {all && all.length === 0 && <p className="empty">You have not uploaded any {kind}.</p>}
      {all && all.length > 0 && shown.length === 0 && <p className="empty">None match.</p>}

      <ul className="tiles tiles-wide">
        {shown.map((r) => {
          const id = str(r.id);
          const name = tidy(str(r.name)) || (kind === "avatars" ? "Unnamed avatar" : "Unnamed world");
          const perf = (r.performance ?? {}) as Json;
          const notes =
            kind === "avatars"
              ? [str(r.releaseStatus), str(perf.standalonewindows) && `PC ${str(perf.standalonewindows)}`, str(perf.android) && `Quest ${str(perf.android)}`]
              : [str(r.releaseStatus), `${num(r.visits).toLocaleString()} visits`, num(r.favorites) ? `${num(r.favorites).toLocaleString()} favourites` : ""];
          return (
            <li key={id} className="tile">
              <button className="tile-open" onClick={() => openThing(kind === "avatars" ? "avatar" : "world", id)} title="Open">
                <Picture url={str(r.thumbnailImageUrl) || str(r.imageUrl)} />
                <span className="tile-name">{name}</span>
              </button>
              <span className="tile-note">{notes.filter(Boolean).join(", ")}</span>
              {str(r.updated_at) && <span className="tile-note">Updated {when(str(r.updated_at))}</span>}
              {kind === "avatars" && tagging !== id && <TagChips id={id} tags={tags} />}
              {kind === "avatars" && tagging === id && <TagEditor id={id} tags={tags} onDone={onDone} />}
              <div className="wayback">
                {kind === "avatars" && (
                  <>
                    <button className="go" disabled={busy === id} onClick={() => void wear(id, name)}>
                      Wear
                    </button>
                  </>
                )}
                <button onClick={() => copy(id, "ID copied.")}>Copy ID</button>
                {kind === "avatars" && (
                  <button aria-expanded={tagging === id} title="Your own tags, kept on this PC" onClick={() => setTagging(tagging === id ? "" : id)}>
                    {tagging === id ? "Done" : "Tags"}
                  </button>
                )}
                {deleting === id ? (
                  <>
                    <button className="danger" disabled={busy === id} onClick={() => void change(id, name, "delete")}>
                      Yes, delete for good
                    </button>
                    <button onClick={() => setDeleting("")}>Keep</button>
                  </>
                ) : (
                  <>
                    <button
                      disabled={busy === id}
                      title={str(r.releaseStatus) === "public" ? "Only you can use it after this" : kind === "avatars" ? "Anyone can use it after this" : "Publishes the world"}
                      onClick={() => void change(id, name, str(r.releaseStatus) === "public" ? "private" : "public")}
                    >
                      {str(r.releaseStatus) === "public" ? "Make private" : "Make public"}
                    </button>
                    <button title="Removes it from VRChat. This cannot be undone." onClick={() => setDeleting(id)}>
                      Delete
                    </button>
                  </>
                )}
              </div>
              {deleting === id && (
                <span className="tile-note delete-warning">
                  This removes {name} from VRChat for everyone and cannot be undone.
                </span>
              )}
            </li>
          );
        })}
      </ul>
      </div>
    </div>
  );
}
