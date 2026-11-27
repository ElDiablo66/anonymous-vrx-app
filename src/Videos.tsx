import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { backend, errorText, type VideoPlay, type VideoTitle, type World } from "./backend";
import { clock, dayLabel, tidy } from "./bits";
import { openThing } from "./Detail";

/** Titles YouTube already gave in this run, shared by every list, and the ids being asked about now. */
const knownTitles = new Map<string, VideoTitle>();
const askingNow = new Set<string>();

/**
 * Titles for the YouTube videos in a list. Titles the history already holds come with the rows;
 * the rest are asked for a few at a time through the app, which keeps them. An id that got no
 * answer (no connection) is tried once per visit to the list.
 */
function useTitles(rows: VideoPlay[]): Map<string, VideoTitle> {
  const [, redraw] = useState(0);
  const [round, setRound] = useState(0);
  const tried = useRef(new Set<string>());
  const missing = useMemo(
    () =>
      [...new Set(rows.filter((r) => r.youtubeId && !r.title && !r.channel).map((r) => r.youtubeId))].filter(
        (id) => !knownTitles.has(id) && !askingNow.has(id) && !tried.current.has(id),
      ),
    [rows, round], // eslint-disable-line react-hooks/exhaustive-deps
  );
  useEffect(() => {
    if (missing.length === 0) return;
    let alive = true;
    const batch = missing.slice(0, 8);
    batch.forEach((id) => {
      askingNow.add(id);
      tried.current.add(id);
    });
    void backend
      .videoTitles(batch)
      .then((found) => {
        // Kept even when this list has gone meanwhile, so the next list does not ask again.
        for (const t of found) knownTitles.set(t.id, t);
      })
      .catch(() => {})
      .finally(() => {
        batch.forEach((id) => askingNow.delete(id));
        if (!alive) return;
        redraw((n) => n + 1);
        if (missing.length > batch.length) window.setTimeout(() => alive && setRound((n) => n + 1), 600);
      });
    return () => {
      alive = false;
    };
  }, [missing.join()]); // eslint-disable-line react-hooks/exhaustive-deps
  return knownTitles;
}

function host(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return "a link";
  }
}

function nameOf(v: VideoPlay, titles: Map<string, VideoTitle>): { title: string; channel: string } {
  const t = titles.get(v.youtubeId);
  const title = tidy(v.title || t?.title || "");
  const channel = tidy(v.channel || t?.channel || "");
  if (title) return { title, channel };
  return { title: v.youtubeId ? "A YouTube video" : `A video from ${host(v.url)}`, channel };
}

/** The video's own picture from YouTube, or a dusk tile with a play mark for any other link. */
function VideoThumb({ v }: { v: VideoPlay }) {
  const [broken, setBroken] = useState(false);
  if (!v.youtubeId || broken) {
    return (
      <span className="video-thumb video-thumb-plain" aria-hidden="true">
        <i />
      </span>
    );
  }
  return (
    <span className="video-thumb" aria-hidden="true">
      <img
        src={`https://i.ytimg.com/vi/${v.youtubeId}/mqdefault.jpg`}
        alt=""
        loading="lazy"
        decoding="async"
        referrerPolicy="no-referrer"
        onError={() => setBroken(true)}
        // A removed video still answers, with a small grey stand-in instead of its picture.
        onLoad={(e) => e.currentTarget.naturalWidth <= 120 && setBroken(true)}
      />
      <i />
    </span>
  );
}

/** Played within half a minute of my arrival: the world's own player started it, or it was already on when I came in. */
function onArrival(v: VideoPlay): boolean {
  return !!v.joinedAt && Date.parse(v.at) - Date.parse(v.joinedAt) <= 30_000;
}

/** Plays of the same video in the same stay, next to each other in the list, shown as one row. */
interface Group {
  first: VideoPlay;
  plays: VideoPlay[];
}

function grouped(rows: VideoPlay[]): Group[] {
  const out: Group[] = [];
  for (const v of rows) {
    const last = out[out.length - 1];
    if (last && last.first.url === v.url && last.first.visitId === v.visitId && dayLabel(last.first.at) === dayLabel(v.at)) last.plays.push(v);
    else out.push({ first: v, plays: [v] });
  }
  return out;
}

/** Older first in the list order: later time, then higher id. */
function before(a: VideoPlay, b: VideoPlay): boolean {
  return a.at < b.at || (a.at === b.at && a.id < b.id);
}

function open(url: string, onDone: (message: string) => void) {
  void backend.openLink(url).catch((err) => onDone(`Could not open it: ${errorText(err)}`));
}

/** How many videos one page brings. */
const PAGE = 60;

/** History, Videos: what the worlds played while I was there, from VRChat's own log on this PC. */
export function VideosPlayed({
  worlds,
  tick,
  onDone,
  renderVisit,
}: {
  worlds: Map<string, World>;
  tick: number;
  onDone: (message: string) => void;
  /** Who was in the stay a video played in, with the ways back there. */
  renderVisit: (v: VideoPlay) => ReactNode;
}) {
  const [rows, setRows] = useState<VideoPlay[] | null>(null);
  const [total, setTotal] = useState(0);
  const [more, setMore] = useState(false);
  const [query, setQuery] = useState("");
  const [asked, setAsked] = useState("");
  const [skipOn, setSkipOn] = useState(false);
  const [problem, setProblem] = useState("");
  const [opened, setOpened] = useState<number | null>(null);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const shownFor = useRef("");
  const latest = useRef(0);

  // The search goes to the history a moment after typing stops.
  useEffect(() => {
    const t = window.setTimeout(() => setAsked(query.trim()), 250);
    return () => window.clearTimeout(t);
  }, [query]);

  // The newest page: from scratch for a new search, and merged in on top when the history changes.
  useEffect(() => {
    const key = `${asked}\u0000${skipOn}`;
    const fresh = key !== shownFor.current;
    const n = ++latest.current;
    void backend
      .videosPage({ query: asked, skipArrival: skipOn, limit: PAGE })
      .then(({ rows: page, total: all }) => {
        if (n !== latest.current) return;
        setProblem("");
        setTotal(all);
        if (fresh) {
          shownFor.current = key;
          setRows(page);
          setMore(page.length === PAGE);
          setOpened(null);
          return;
        }
        setRows((old) => {
          if (!old || page.length === 0) return page.length ? page : (old ?? []);
          const ids = new Set(page.map((v) => v.id));
          // A full page with nothing in common with the list: more came in than one page holds,
          // so the list starts over from the top rather than leave a gap.
          if (page.length === PAGE && !old.some((v) => ids.has(v.id))) {
            setMore(true);
            return page;
          }
          const oldest = page[page.length - 1];
          return [...page, ...old.filter((v) => !ids.has(v.id) && before(v, oldest))];
        });
      })
      .catch((err) => {
        if (n !== latest.current) return;
        setProblem(errorText(err));
        setRows((old) => old ?? []);
      });
  }, [asked, skipOn, tick]);

  async function loadOlder() {
    if (!rows || rows.length === 0) return;
    const key = shownFor.current;
    const last = rows[rows.length - 1];
    setLoadingOlder(true);
    try {
      const { rows: page } = await backend.videosPage({ query: asked, skipArrival: skipOn, before: { at: last.at, id: last.id }, limit: PAGE });
      // A new search meanwhile makes this page stale.
      if (key !== shownFor.current) return;
      setRows((old) => {
        const have = new Set((old ?? []).map((v) => v.id));
        return [...(old ?? []), ...page.filter((v) => !have.has(v.id))];
      });
      setMore(page.length === PAGE);
    } catch (err) {
      onDone(`Older videos could not be read: ${errorText(err)}`);
    } finally {
      setLoadingOlder(false);
    }
  }

  const titles = useTitles(rows ?? []);
  if (rows === null) return <p className="empty">Reading the videos from your history</p>;
  if (rows.length === 0 && !asked && !skipOn) {
    return problem ? (
      <p className="banner" role="alert">
        The videos could not be read: {problem}
      </p>
    ) : (
      <p className="empty">
        No videos yet. When a world plays a video while you are in it, VRChat writes that in its own log on this PC, and it shows up here with the world it played in.
      </p>
    );
  }

  const groups = grouped(rows);
  let lastDay = "";

  return (
    <div className="videos">
      <div className="list-bar videos-bar">
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === "Escape" && query && (e.stopPropagation(), setQuery(""))}
          placeholder="Search worlds, titles, links and who put it on"
          aria-label="Search the videos"
        />
        <button className={`plain${skipOn ? " on" : ""}`} aria-pressed={skipOn} onClick={() => setSkipOn(!skipOn)} title="Leave out videos that were already playing, or that the world started, as you arrived">
          Leave out ones already on
        </button>
        <span className="list-count">{asked || skipOn ? `${total.toLocaleString()} found` : `${total.toLocaleString()} in all`}</span>
      </div>
      {problem && (
        <p className="banner" role="alert">
          The videos could not be read: {problem}
        </p>
      )}
      {groups.length === 0 ? (
        <p className="empty">No video matches that. A title is only found once the app has fetched it; the world, the link and who put it on always are.</p>
      ) : (
        <ol className="video-list">
          {groups.map(({ first: v, plays }) => {
            const day = dayLabel(v.at);
            const header = day !== lastDay ? (
              <li className="day" key={`day-${v.id}`}>
                {day}
              </li>
            ) : null;
            lastDay = day;
            const { title, channel } = nameOf(v, titles);
            const world = tidy(v.worldName || worlds.get(v.worldId)?.name || "");
            const putOnBy = [...new Set(plays.map((p) => p.requestedBy).filter(Boolean))];
            const isOpen = opened === v.id;
            return [
              header,
              <li className={`video-row${isOpen ? " open" : ""}`} key={v.id}>
                <time dateTime={v.at} title={plays.length > 1 ? plays.map((p) => clock(p.at)).join(", ") : undefined}>
                  {clock(v.at)}
                </time>
                <VideoThumb v={v} />
                <span className="video-main">
                  <span className="video-title">
                    <span className="video-title-text" title={title}>
                      {title}
                    </span>
                    {plays.length > 1 && <span className="video-times">{plays.length} times</span>}
                  </span>
                  <span className="video-sub">
                    {channel && <span>{channel}</span>}
                    {world ? (
                      v.worldId ? (
                        <button className="linkish" onClick={() => openThing("world", v.worldId)} title="Open the world">
                          {world}
                        </button>
                      ) : (
                        <span>{world}</span>
                      )
                    ) : (
                      <span>A world the history has no name for</span>
                    )}
                    {putOnBy.length > 0 ? (
                      <span>
                        put on by <span className="hide-name">{putOnBy.join(", ")}</span>
                      </span>
                    ) : (
                      onArrival(v) && <span className="video-tag">already on as you arrived</span>
                    )}
                  </span>
                </span>
                <span className="video-actions">
                  <button className="plain" onClick={() => open(v.url, onDone)} title="Opens the video in your browser">
                    Watch
                  </button>
                  <button
                    className="plain"
                    onClick={() =>
                      void navigator.clipboard.writeText(v.url).then(
                        () => onDone("Video link copied."),
                        () => onDone("Could not copy."),
                      )
                    }
                  >
                    Copy video link
                  </button>
                  {v.visitId !== null && (
                    <button className={`plain${isOpen ? " on" : ""}`} aria-expanded={isOpen} onClick={() => setOpened(isOpen ? null : v.id)} title="Who was in that stay, and the ways back there">
                      Who was there
                    </button>
                  )}
                </span>
                {isOpen && <div className="video-visit">{renderVisit(v)}</div>}
              </li>,
            ];
          })}
        </ol>
      )}
      {more && (
        <button className="older" disabled={loadingOlder} onClick={() => void loadOlder()}>
          {loadingOlder ? "Reading older videos" : "Show older videos"}
        </button>
      )}
      <p className="setting-hint video-note">
        From VRChat's own log on this PC. A video shows up when a world's player starts it while you are there. For a YouTube video the app asks YouTube once for its title, and the picture loads from YouTube each time the
        video is on screen.
      </p>
    </div>
  );
}

/** The videos of one stay, under its people in the world history. */
export function StayVideos({ list, onDone }: { list: VideoPlay[]; onDone: (message: string) => void }) {
  const titles = useTitles(list);
  if (list.length === 0) return null;
  return (
    <div className="stay-videos">
      <h4>Videos played here</h4>
      <ul>
        {list.map((v) => {
          const { title } = nameOf(v, titles);
          return (
            <li key={v.id}>
              <time dateTime={v.at}>{clock(v.at)}</time>
              <span className="stay-video-title" title={title}>
                {title}
                {v.requestedBy && (
                  <small>
                    {" "}
                    put on by <span className="hide-name">{v.requestedBy}</span>
                  </small>
                )}
              </span>
              <button className="plain" onClick={() => open(v.url, onDone)}>
                Watch
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
