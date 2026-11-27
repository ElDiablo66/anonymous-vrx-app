import { useEffect, useMemo, useRef, useState } from "react";
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

/** Plays of the same video in the same world on the same day, next to each other in the list, shown as one row. */
interface Group {
  first: VideoPlay;
  plays: VideoPlay[];
}

function grouped(rows: VideoPlay[]): Group[] {
  const out: Group[] = [];
  for (const v of rows) {
    const last = out[out.length - 1];
    if (last && last.first.url === v.url && last.first.worldId === v.worldId && dayLabel(last.first.at) === dayLabel(v.at)) last.plays.push(v);
    else out.push({ first: v, plays: [v] });
  }
  return out;
}

function open(url: string, onDone: (message: string) => void) {
  void backend.openLink(url).catch((err) => onDone(`Could not open it: ${errorText(err)}`));
}

/** The most rows the tab reads at once; the history keeps every play. */
const MOST = 2000;

/** History, Videos: what the worlds played while I was there, from VRChat's own log on this PC. */
export function VideosPlayed({ worlds, tick, onDone }: { worlds: Map<string, World>; tick: number; onDone: (message: string) => void }) {
  const [rows, setRows] = useState<VideoPlay[] | null>(null);
  const [query, setQuery] = useState("");
  const [shown, setShown] = useState(80);
  const [skipOn, setSkipOn] = useState(false);

  useEffect(() => {
    let alive = true;
    void backend
      .videos(MOST)
      .then((list) => alive && setRows(list))
      .catch(() => alive && setRows([]));
    return () => {
      alive = false;
    };
  }, [tick]);

  const kept = useMemo(() => (rows ?? []).filter((v) => !(skipOn && onArrival(v) && !v.requestedBy)), [rows, skipOn]);
  // Titles are asked for the rows this filter shows, so a filtered list gets its titles too.
  const titles = useTitles(useMemo(() => kept.slice(0, shown), [kept, shown]));
  if (rows === null) return <p className="empty">Reading the videos from your history</p>;
  if (rows.length === 0) {
    return (
      <p className="empty">
        No videos yet. When a world plays a video while you are in it, VRChat writes that in its own log on this PC, and it shows up here with the world it played in.
      </p>
    );
  }

  const q = query.trim().toLowerCase();
  const matching = kept.filter((v) => {
    if (!q) return true;
    const { title, channel } = nameOf(v, titles);
    const world = v.worldName || worlds.get(v.worldId)?.name || "";
    return [title, channel, world, v.requestedBy, v.url].some((s) => s.toLowerCase().includes(q));
  });
  const groups = grouped(matching.slice(0, shown));
  const all = rows.length >= MOST ? `latest ${rows.length}` : `${rows.length} in all`;
  let lastDay = "";

  return (
    <div className="videos">
      <div className="list-bar videos-bar">
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === "Escape" && query && (e.stopPropagation(), setQuery(""))}
          placeholder="Search titles, worlds and who put it on"
          aria-label="Search the videos"
        />
        <button className={`plain${skipOn ? " on" : ""}`} aria-pressed={skipOn} onClick={() => setSkipOn(!skipOn)} title="Leave out videos that were already playing, or that the world started, as you arrived">
          Leave out ones already on
        </button>
        <span className="list-count">{q || skipOn ? `${matching.length} of ${all}` : all}</span>
      </div>
      {groups.length === 0 ? (
        <p className="empty">No video matches that.</p>
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
            return [
              header,
              <li className="video-row" key={v.id}>
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
                        () => onDone("Link copied."),
                        () => onDone("Could not copy."),
                      )
                    }
                  >
                    Copy link
                  </button>
                </span>
              </li>,
            ];
          })}
        </ol>
      )}
      {matching.length > shown && (
        <button className="older" onClick={() => setShown(shown + 80)}>
          Show older videos
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
