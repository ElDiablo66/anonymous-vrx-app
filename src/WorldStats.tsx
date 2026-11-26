import { useEffect, useMemo, useState } from "react";
import { backend, type WorldDay } from "./backend";

/** What the app noted about my worlds, by world, oldest day first. Read when `on` turns true. */
export function useWorldStats(on: boolean): Map<string, WorldDay[]> {
  const [days, setDays] = useState<WorldDay[]>([]);
  useEffect(() => {
    if (!on) return;
    let alive = true;
    void backend
      .worldStats(60)
      .then((d) => alive && setDays(d))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [on]);
  return useMemo(() => {
    const by = new Map<string, WorldDay[]>();
    for (const d of days) by.set(d.worldId, [...(by.get(d.worldId) ?? []), d]);
    return by;
  }, [days]);
}

const daysBetween = (a: string, b: string) => Math.round((Date.parse(b) - Date.parse(a)) / 86_400_000);

/** How a world moved over about the last week: what was gained, and over how many days. */
function lately(days: WorldDay[]): { visits: number; favorites: number; span: number } | null {
  if (days.length < 2) return null;
  const last = days[days.length - 1];
  // The note from a week back, or the oldest one when the app has not kept track that long.
  const base = [...days].reverse().find((d) => daysBetween(d.day, last.day) >= 7) ?? days[0];
  return { visits: last.visits - base.visits, favorites: last.favorites - base.favorites, span: daysBetween(base.day, last.day) };
}

/** What was gained from one note to the next, newest last. */
function gains(days: WorldDay[], pick: (d: WorldDay) => number): number[] {
  return days.slice(1).map((d, i) => Math.max(0, pick(d) - pick(days[i])));
}

const signed = (n: number) => `${n > 0 ? "+" : ""}${n.toLocaleString()}`;
const spanWords = (span: number) => (span === 1 ? "since yesterday" : `in ${span} days`);

/** Small bars, one per day, tallest for the best day. */
function Bars({ values, label }: { values: number[]; label: string }) {
  const top = Math.max(1, ...values);
  return (
    <span className="trend-bars" role="img" aria-label={label}>
      {values.map((v, i) => (
        <i key={i} style={{ height: `${Math.max(6, Math.round((v / top) * 100))}%` }} className={v === 0 ? "none" : ""} />
      ))}
    </span>
  );
}

/** Under a world's tile: what it gained lately, with a bar per day. */
export function WorldTrend({ days }: { days: WorldDay[] | undefined }) {
  if (!days || days.length === 0) return null;
  const move = lately(days);
  if (!move) return <span className="tile-note trend">Keeping count from today. Changes show from tomorrow.</span>;
  const perDay = gains(days, (d) => d.visits).slice(-14);
  return (
    <span className="tile-note trend" title="From the times this app looked at your worlds">
      <span>
        <b className={move.visits > 0 ? "up" : ""}>{signed(move.visits)}</b> visits {spanWords(move.span)}
        {move.favorites !== 0 && (
          <>
            , <b className={move.favorites > 0 ? "up" : "down"}>{signed(move.favorites)}</b> {Math.abs(move.favorites) === 1 ? "favourite" : "favourites"}
          </>
        )}
      </span>
      <Bars values={perDay} label={`Visits gained per day over the last ${perDay.length} days`} />
    </span>
  );
}

/** Above the list of my worlds: all of them together. */
export function WorldSummary({ stats, now }: { stats: Map<string, WorldDay[]>; now: { visits: number; favorites: number; occupants: number; count: number } }) {
  const total = useMemo(() => {
    // Every world's gains, added up by day.
    const perDay = new Map<string, number>();
    let visits = 0;
    let favorites = 0;
    let span = 0;
    let any = false;
    for (const days of stats.values()) {
      const move = lately(days);
      if (move) {
        any = true;
        visits += move.visits;
        favorites += move.favorites;
        span = Math.max(span, move.span);
      }
      days.slice(1).forEach((d, i) => perDay.set(d.day, (perDay.get(d.day) ?? 0) + Math.max(0, d.visits - days[i].visits)));
    }
    const bars = [...perDay.entries()].sort(([a], [b]) => a.localeCompare(b)).map(([, v]) => v).slice(-30);
    return { visits, favorites, span, any, bars };
  }, [stats]);

  if (now.count === 0) return null;
  return (
    <div className="world-summary">
      <dl>
        <div>
          <dt>Visits</dt>
          <dd>{now.visits.toLocaleString()}</dd>
          {total.any && (
            <small className={total.visits > 0 ? "up" : ""}>
              {signed(total.visits)} {spanWords(total.span)}
            </small>
          )}
        </div>
        <div>
          <dt>Favourites</dt>
          <dd>{now.favorites.toLocaleString()}</dd>
          {total.any && (
            <small className={total.favorites > 0 ? "up" : total.favorites < 0 ? "down" : ""}>
              {signed(total.favorites)} {spanWords(total.span)}
            </small>
          )}
        </div>
        <div>
          <dt>In them right now</dt>
          <dd>{now.occupants.toLocaleString()}</dd>
          <small>
            across {now.count} {now.count === 1 ? "world" : "worlds"}
          </small>
        </div>
      </dl>
      {total.bars.length > 1 ? (
        <div className="world-chart">
          <Bars values={total.bars} label={`Visits gained per day over the last ${total.bars.length} days`} />
          <small>Visits gained per day, last {total.bars.length} days</small>
        </div>
      ) : (
        <p className="world-chart-none">The app notes these numbers each day it runs. A chart shows here once it has a few days.</p>
      )}
    </div>
  );
}
