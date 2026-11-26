import { NewInstance } from "./NewInstance";
import { useEffect, useState } from "react";
import { backend, errorText } from "./backend";
import { tidy } from "./bits";
import { openProfile } from "./Profile";
import { openPicture } from "./Viewer";
import { Stars, TagEditor, beenWords, useTags, useWorldMarks } from "./Mine";
import { prettyGroup } from "./Favourites";
import { SafeImg, usePic } from "./Pic";
import { accessName, regionName } from "./bits";
import { parsePlace } from "./place";

export type ThingKind = "world" | "avatar" | "group" | "instance";
type Json = Record<string, unknown>;

/** Any part of the window can open a world, an avatar or a group this way. */
export function openThing(kind: ThingKind, id: string) {
  if (id) window.dispatchEvent(new CustomEvent("open-thing", { detail: { kind, id } }));
}

const str = (v: unknown): string => (typeof v === "string" ? v : "");
const num = (v: unknown): number => (typeof v === "number" ? v : 0);
const list = (v: unknown): Json[] => (Array.isArray(v) ? (v.filter((x) => x && typeof x === "object") as Json[]) : []);

function age(iso: string): string {
  if (!iso) return "";
  const days = Math.floor((Date.now() - Date.parse(iso)) / 86_400_000);
  if (days < 1) return "today";
  if (days < 60) return `${days} days ago`;
  if (days < 730) return `${Math.floor(days / 30)} months ago`;
  return `${Math.floor(days / 365)} years ago`;
}

const fallbackGroups: Record<string, string[]> = {
  world: ["worlds1", "worlds2", "worlds3", "worlds4"],
  avatar: ["avatars1"],
};

export function DetailPanel({
  kind,
  id,
  meId,
  onBack,
  onClose,
  onDone,
}: {
  /** The signed-in user's id; their own uploads get more buttons. */
  meId: string;
  onBack?: () => void;
  kind: ThingKind;
  id: string;
  onClose: () => void;
  onDone: (message: string) => void;
}) {
  const marks = useWorldMarks();
  const myTags = useTags();
  const [data, setData] = useState<Json | null>(null);
  const [problem, setProblem] = useState("");
  const [tab, setTab] = useState("info");
  const [extra, setExtra] = useState<Record<string, Json[] | null>>({});
  const [groups, setGroups] = useState<Array<{ name: string; label: string; full: boolean }>>([]);
  const [reload, setReload] = useState(0);
  const [leaving, setLeaving] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [gallery, setGallery] = useState("");
  const [group, setGroup] = useState("");
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState("");
  const [savedNote, setSavedNote] = useState("");

  useEffect(() => {
    if (kind === "group") return;
    let alive = true;
    void backend
      .note(id)
      .then((n) => {
        if (!alive) return;
        setNote(n);
        setSavedNote(n);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [kind, id]);

  useEffect(() => {
    let alive = true;
    setData(null);
    setProblem("");
    setTab("info");
    setExtra({});
    void backend
      .thing(kind, id)
      .then((d) => alive && setData((d ?? {}) as Json))
      .catch((err) => alive && setProblem(errorText(err)));
    return () => {
      alive = false;
    };
  }, [kind, id, reload]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const copy = (text: string, done: string) =>
    void navigator.clipboard.writeText(text).then(
      () => onDone(done),
      () => onDone("Could not copy."),
    );

  async function act(job: () => Promise<unknown>, done: string) {
    setBusy(true);
    try {
      await job();
      onDone(done);
    } catch (err) {
      onDone(`That did not work: ${errorText(err)}`);
    } finally {
      setBusy(false);
    }
  }

  async function showFavouriteGroups() {
    if (groups.length) return;
    let found: Array<{ name: string; label: string; full: boolean }> = [];
    try {
      // The answer from last time is enough to list the groups, and it is instant.
      const all = ((await backend.favoritesCached(kind)) ?? (await backend.favoritesOf(kind))) as Json;
      const capacity = num(all.capacity);
      const used = new Map<string, number>();
      for (const e of list(all.entries)) {
        const tag = str((e.tags as unknown[])?.[0]);
        used.set(tag, (used.get(tag) ?? 0) + 1);
      }
      found = list(all.groups)
        .map((g) => {
          const count = used.get(str(g.name)) ?? 0;
          const label = prettyGroup(str(g.displayName) || str(g.name));
          return { name: str(g.name), label: capacity ? `${label} (${count}/${capacity})` : label, full: capacity > 0 && count >= capacity };
        })
        .sort((a, b) => a.name.localeCompare(b.name));
    } catch {
      // Fall through to the standard group names.
    }
    if (!found.length) found = (fallbackGroups[kind] ?? []).map((n) => ({ name: n, label: prettyGroup(n), full: false }));
    setGroups(found);
    setGroup((found.find((g) => !g.full) ?? found[0])?.name ?? "");
  }

  function open(next: string) {
    setTab(next);
    const part = next === "members" ? "groupMembers" : next === "posts" ? "groupPosts" : next === "instances" ? "groupInstances" : "";
    if (next === "photos") showGallery(gallery || str(list((data ?? {}).galleries)[0]?.id));
    if (part && !(part in extra)) {
      void backend
        .thing(part, id)
        .then((d) => setExtra((old) => ({ ...old, [part]: list(Array.isArray(d) ? d : (d as Json)?.posts) })))
        .catch(() => setExtra((old) => ({ ...old, [part]: null })));
    }
  }

  function showGallery(galleryId: string) {
    if (!galleryId) return;
    setGallery(galleryId);
    const key = `photos:${galleryId}`;
    if (key in extra) return;
    void backend
      .thing("groupPhotos", `${id}/${galleryId}`)
      .then((d) => setExtra((old) => ({ ...old, [key]: list(d) })))
      .catch(() => setExtra((old) => ({ ...old, [key]: null })));
  }

  const d = data ?? {};
  const galleries = kind === "group" ? list(d.galleries) : [];
  const membership = str(d.membershipStatus);
  const mine = kind !== "group" && data !== null && str(d.authorId) === meId;
  const isPublic = str(d.releaseStatus) === "public";
  const joinState = str(d.joinState);
  const name = tidy(str(d.name)) || (kind === "group" ? "Group" : kind === "world" ? "World" : "Avatar");
  const image = str(d.imageUrl) || str(d.bannerUrl) || str(d.thumbnailImageUrl) || str(d.iconUrl);
  const icon = kind === "group" ? str(d.iconUrl) : "";
  const imageShown = usePic(image);
  const iconShown = usePic(icon);
  const site = `https://vrchat.com/home/${kind}/${id}`;
  const perf = (d.performance ?? {}) as Json;
  const tags = Array.isArray(d.tags) ? (d.tags as unknown[]).map(str) : [];
  // Public instances VRChat lists for a world: [instance, people].
  const instances: Array<[string, number]> =
    kind === "world" && Array.isArray(d.instances)
      ? (d.instances as unknown[])
          .filter((x): x is unknown[] => Array.isArray(x) && typeof x[0] === "string")
          .map((x) => [String(x[0]), typeof x[1] === "number" ? x[1] : 0] as [string, number])
          .sort((a, b) => b[1] - a[1])
          .slice(0, 30)
      : [];
  const authorTags = tags.filter((t) => t.startsWith("author_tag_")).map((t) => t.slice(11));
  const tabs: Array<[string, string]> =
    kind === "group"
      ? [["info", "Info"], ["posts", "Posts"], ["members", "Members"], ...(galleries.length ? [["photos", "Photos"] as [string, string]] : []), ["instances", "Instances"], ["json", "JSON"]]
      : [["info", "Info"], ["json", "JSON"]];

  const facts: Array<[string, string]> =
    kind === "world"
      ? [
          ["People there now", `${num(d.occupants)} (${num(d.publicOccupants)} public, ${num(d.privateOccupants)} private)`],
          ["Capacity", `${num(d.recommendedCapacity) || num(d.capacity)} recommended, ${num(d.capacity)} at most`],
          ["Visits", num(d.visits).toLocaleString()],
          ["Favourites", num(d.favorites).toLocaleString()],
          ["Created", age(str(d.created_at))],
          ["Last updated", age(str(d.updated_at))],
          ["Version", String(num(d.version) || "")],
        ]
      : kind === "avatar"
        ? [
            ["Visibility", str(d.releaseStatus)],
            ["Performance on PC", str(perf.standalonewindows)],
            ["Performance on Quest", str(perf.android)],
            ["Created", age(str(d.created_at))],
            ["Last updated", age(str(d.updated_at))],
            ["Version", String(num(d.version) || "")],
          ]
        : [
            ["Short code", str(d.shortCode) ? `${str(d.shortCode)}.${str(d.discriminator)}` : ""],
            ["Members", num(d.memberCount).toLocaleString()],
            ["Online now", num(d.onlineMemberCount).toLocaleString()],
            ["Joining", str(d.joinState)],
            ["Your membership", str(d.membershipStatus)],
            ["Created", age(str(d.createdAt))],
          ];

  return (
    <div className="veil" onClick={onClose}>
      <div className={mine ? "profile own-window" : "profile"} role="dialog" aria-modal="true" aria-label={name} onClick={(e) => e.stopPropagation()}>
        <aside className="profile-side">
          <button
            className={kind === "group" ? "big-pic wide" : "big-pic"}
            style={image ? { backgroundImage: `url("${imageShown}")` } : undefined}
            disabled={!image}
            title="Show the picture full size"
            onClick={() => openPicture(image, name)}
            aria-label="Show the picture full size"
          />
          {icon && (
            <button className="group-icon" onClick={() => openPicture(icon, `${name} icon`)} title="Show the group icon full size" aria-label="Show the group icon full size">
              <span className="thumb" style={{ backgroundImage: `url("${iconShown}")` }} />
            </button>
          )}
          <h2 className="hide-me">{name}</h2>
          {(str(d.authorName) || kind === "group") && (
            <p className="profile-status">
              {kind === "group" ? (
                <button className="name-link who" onClick={() => openProfile(str(d.ownerId))}>
                  Open the owner's profile
                </button>
              ) : (
                <button className="name-link who" onClick={() => openProfile(str(d.authorId))}>
                  by {str(d.authorName)}
                </button>
              )}
            </p>
          )}
          <div className="wayback">
            {kind === "avatar" && (
              <button className="go" disabled={busy || !data} onClick={() => void act(() => backend.wearAvatar(id), `You are now wearing ${name}.`)}>
                Wear
              </button>
            )}
            {kind === "group" && data && membership === "member" && !leaving && <button onClick={() => setLeaving(true)}>Leave group</button>}
            {kind === "group" && leaving && (
              <>
                <button
                  disabled={busy}
                  onClick={() =>
                    void act(() => backend.groupAction(id, "leave"), `You left ${name}.`).then(() => {
                      setLeaving(false);
                      setReload((n) => n + 1);
                    })
                  }
                >
                  Yes, leave
                </button>
                <button onClick={() => setLeaving(false)}>Stay</button>
              </>
            )}
            {kind === "group" && data && membership !== "member" && membership !== "requested" && membership !== "banned" && (joinState === "open" || joinState === "request" || membership === "invited") && (
              <button
                className="go"
                disabled={busy}
                onClick={() =>
                  void act(
                    () => backend.groupAction(id, "join"),
                    joinState === "open" || membership === "invited" ? `You joined ${name}.` : `Asked to join ${name}. The group decides.`,
                  ).then(() => setReload((n) => n + 1))
                }
              >
                {joinState === "open" || membership === "invited" ? "Join group" : "Ask to join"}
              </button>
            )}
            {kind === "group" && membership === "requested" && <button disabled>Request sent</button>}
            <button onClick={() => copy(id, "ID copied.")}>Copy ID</button>
            {kind !== "group" && <button onClick={() => copy(site, "Link copied.")}>Copy link</button>}
            {mine && !deleting && (
              <>
                <button
                  disabled={busy}
                  onClick={() =>
                    void act(
                      () => backend.uploadAction(kind as "avatar" | "world", id, isPublic ? "private" : "public"),
                      `${name} is now ${isPublic ? "private" : "public"}.`,
                    ).then(() => setReload((n) => n + 1))
                  }
                >
                  {isPublic ? "Make private" : "Make public"}
                </button>
                <button title="Removes it from VRChat. This cannot be undone." onClick={() => setDeleting(true)}>
                  Delete
                </button>
              </>
            )}
            {mine && deleting && (
              <>
                <button
                  className="danger"
                  disabled={busy}
                  onClick={() => void act(() => backend.uploadAction(kind as "avatar" | "world", id, "delete"), `${name} is deleted.`).then(onClose)}
                >
                  Yes, delete for good
                </button>
                <button onClick={() => setDeleting(false)}>Keep</button>
              </>
            )}
          </div>
          {mine && deleting && (
            <p className="tile-note delete-warning">
              This removes {name} from VRChat for everyone and cannot be undone.
            </p>
          )}

          {kind !== "group" && (
            <section>
              <h3>Favourite</h3>
              {groups.length === 0 ? (
                <div className="wayback">
                  <button disabled={!data} onClick={() => void showFavouriteGroups()}>
                    Add to favourites
                  </button>
                </div>
              ) : (
                <div className="fav-pick">
                  <select value={group} onChange={(e) => setGroup(e.target.value)} aria-label="Favourite group">
                    {groups.map((g) => (
                      <option key={g.name} value={g.name} disabled={g.full}>
                        {g.label}
                        {g.full ? ", full" : ""}
                      </option>
                    ))}
                  </select>
                  <button className="plain go" disabled={busy || !group} onClick={() => void act(() => backend.addFavorite(kind, id, group), `Added ${name} to your favourites.`)}>
                    Add
                  </button>
                </div>
              )}
              <div className="wayback">
                <button
                  disabled={!data || busy}
                  title="Keeps it in Favourites, On this PC. No limit, and nobody else sees it."
                  onClick={() =>
                    void act(
                      () =>
                        backend.addLocalFavorite({
                          kind: kind as "avatar" | "world",
                          itemId: id,
                          name: str(d.name),
                          authorName: str(d.authorName),
                          image: str(d.thumbnailImageUrl) || str(d.imageUrl),
                        }),
                      `${name} is kept on this PC.`,
                    )
                  }
                >
                  Keep on this PC
                </button>
              </div>
            </section>
          )}
        </aside>

        <div className="profile-main">
          <div className="profile-top">
            <div className="tabs" role="tablist" aria-label={name}>
              {tabs.map(([key, label]) => (
                <button key={key} role="tab" aria-selected={tab === key} className={tab === key ? "on" : ""} onClick={() => open(key)}>
                  {label}
                </button>
              ))}
            </div>
            <span className="window-nav">
              {onBack && <button onClick={onBack}>Back</button>}
              <button onClick={onClose}>Close</button>
            </span>
          </div>
          <div className="profile-body">
            {problem && (
              <p className="banner" role="alert">
                VRChat did not send this: {problem}
              </p>
            )}
            {!data && !problem && <p className="profile-fine">Asking VRChat</p>}

            {tab === "info" && data && (
              <>
                <section>
                  <h3>{kind === "group" ? "About" : "Description"}</h3>
                  {str(d.description) ? <p className="profile-bio">{str(d.description)}</p> : <p className="profile-fine">Nothing written.</p>}
                  {authorTags.length > 0 && <p className="profile-fine">Tags: {authorTags.join(", ")}</p>}
                </section>
                {kind === "group" && str(d.rules) && (
                  <section>
                    <h3>Rules</h3>
                    <p className="profile-bio">{str(d.rules)}</p>
                  </section>
                )}
                <section>
                  <h3>Facts</h3>
                  <dl className="facts">
                    {facts
                      .filter(([, value]) => value && value !== "0")
                      .map(([label, value]) => (
                        <div key={label}>
                          <dt>{label}</dt>
                          <dd>{value}</dd>
                        </div>
                      ))}
                  </dl>
                </section>
                {instances.length > 0 && (
                  <section>
                    <h3>Open instances</h3>
                    <ul className="instance-list">
                      {instances.map(([instance, count]) => {
                        const location = `${id}:${instance}`;
                        const place = parsePlace(location);
                        return (
                          <li key={instance}>
                            <button className="visit-row" title="Open this instance" onClick={() => openThing("instance", location)}>
                              <span>#{place.instance}</span>
                              <span className={`access access-${place.access}`}>{accessName(place)}</span>
                              <span>{regionName(place)}</span>
                              <span>{count} {count === 1 ? "person" : "people"}</span>
                            </button>
                          </li>
                        );
                      })}
                    </ul>
                  </section>
                )}
                {kind === "world" && (
                  <section>
                    <h3>Mine</h3>
                    <p className="my-rating">
                      <Stars id={id} onDone={onDone} />
                      <span className="tile-note">My own rating. It stays on this PC.</span>
                    </p>
                    <p className="tile-note">{beenWords(marks.visits[id])}</p>
                  </section>
                )}
                {kind === "avatar" && (
                  <section>
                    <h3>My tags</h3>
                    <TagEditor id={id} tags={myTags} onDone={onDone} />
                    <p className="tile-note">Kept on this PC. They sort and filter My uploads and favourite avatars; VRChat does not see them.</p>
                  </section>
                )}
                {kind === "world" && <NewInstance worldId={id} onDone={onDone} />}
                {kind !== "group" && (
                  <section>
                    <h3>My note</h3>
                    <textarea
                      value={note}
                      onChange={(e) => setNote(e.target.value)}
                      placeholder={kind === "avatar" ? "What it is good for, what to remember about it. Only you see this." : "What it is good for, who you met there. Only you see this."}
                      rows={3}
                      maxLength={2000}
                    />
                    <div className="wayback">
                      <button
                        className="go"
                        disabled={note === savedNote}
                        onClick={() =>
                          void backend
                            .setNote(id, note)
                            .then(() => {
                              setSavedNote(note);
                              onDone(note.trim() ? "Note saved on this PC." : "Note removed.");
                            })
                            .catch((err) => onDone(`The note was not saved: ${errorText(err)}`))
                        }
                      >
                        Save note
                      </button>
                    </div>
                  </section>
                )}
              </>
            )}

            {tab === "posts" && (
              <section>
                <h3>Posts</h3>
                {!("groupPosts" in extra) ? (
                  <p className="profile-fine">Asking VRChat</p>
                ) : !extra.groupPosts || extra.groupPosts.length === 0 ? (
                  <p className="profile-fine">No posts you can see.</p>
                ) : (
                  extra.groupPosts.map((p) => (
                    <article className="post" key={str(p.id)}>
                      <b>{str(p.title)}</b>
                      <p className="profile-bio">{str(p.text)}</p>
                      <span className="tile-note">{age(str(p.createdAt))}</span>
                    </article>
                  ))
                )}
              </section>
            )}


            {tab === "photos" && (
              <section>
                <h3>Photos</h3>
                {galleries.length > 1 && (
                  <div className="filters filters-flat" role="group" aria-label="Galleries">
                    {galleries.map((g) => (
                      <button key={str(g.id)} className={gallery === str(g.id) ? "on" : ""} aria-pressed={gallery === str(g.id)} onClick={() => showGallery(str(g.id))}>
                        {str(g.name) || "Gallery"}
                      </button>
                    ))}
                  </div>
                )}
                {!(`photos:${gallery}` in extra) ? (
                  <p className="profile-fine">Asking VRChat</p>
                ) : !extra[`photos:${gallery}`] || extra[`photos:${gallery}`]!.length === 0 ? (
                  <p className="profile-fine">No photos here that you can see. Some galleries are for members only.</p>
                ) : (
                  <ul className="photo-grid">
                    {extra[`photos:${gallery}`]!.map((p) => (
                      <li key={str(p.id)}>
                        <button onClick={() => openPicture(str(p.imageUrl), `${name} photo`)} title="Show full size" aria-label="Show the photo full size">
                          <SafeImg src={str(p.imageUrl)} loading="lazy" />
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
              </section>
            )}

            {tab === "instances" && (
              <section>
                <h3>Open group instances</h3>
                {!("groupInstances" in extra) ? (
                  <p className="profile-fine">Asking VRChat</p>
                ) : !extra.groupInstances || extra.groupInstances.length === 0 ? (
                  <p className="profile-fine">No instances open right now that you can see.</p>
                ) : (
                  <ul className="card-grid">
                    {extra.groupInstances.map((i) => {
                      const world = (i.world ?? {}) as Json;
                      const pic = str(world.thumbnailImageUrl) || str(world.imageUrl);
                      return (
                        <li key={str(i.location)}>
                          <button className="card" onClick={() => openThing("instance", str(i.location))} title="Open this instance">
                            <span className="thumb" style={pic ? { backgroundImage: `url("${pic}")` } : undefined} />
                            <span className="tile-name">{tidy(str(world.name)) || "World"}</span>
                            <span className="tile-note">
                              {num(i.memberCount)} {num(i.memberCount) === 1 ? "person" : "people"}, {accessName(parsePlace(str(i.location)))}
                            </span>
                          </button>
                        </li>
                      );
                    })}
                  </ul>
                )}
              </section>
            )}

            {tab === "members" && (
              <section>
                <h3>Members</h3>
                {!("groupMembers" in extra) ? (
                  <p className="profile-fine">Asking VRChat</p>
                ) : !extra.groupMembers || extra.groupMembers.length === 0 ? (
                  <p className="profile-fine">
                    {extra.groupMembers === null
                      ? "VRChat did not share the member list. Some groups only show it to their own members."
                      : "No members to show."}
                  </p>
                ) : (
                  <ul className="rows rows-people">
                    {extra.groupMembers.map((m) => {
                      const user = (m.user ?? {}) as Json;
                      const pic = str(user.iconUrl) || str(user.userIcon) || str(user.profilePicOverrideThumbnail) || str(user.thumbnailUrl) || str(user.currentAvatarThumbnailImageUrl);
                      return (
                        <li key={str(m.userId)}>
                          <button className="row-link" onClick={() => openProfile(str(m.userId))}>
                            <span className="thumb round" style={pic ? { backgroundImage: `url("${pic}")` } : undefined} />
                            <span className="tile-name">{str(user.displayName) || "Member"}</span>
                          </button>
                        </li>
                      );
                    })}
                  </ul>
                )}
              </section>
            )}

            {tab === "json" && data && (
              <section>
                <h3>Raw data from VRChat</h3>
                <div className="wayback">
                  <button onClick={() => copy(JSON.stringify(d, null, 2), "Copied.")}>Copy all</button>
                </div>
                <pre className="raw">{JSON.stringify(d, null, 2)}</pre>
              </section>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
