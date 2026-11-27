import { useEffect, useState } from "react";
import { backend, errorText, type Friend, type Habits as HabitData, type World } from "./backend";
import { Avatar, firstWeekday, statusTone, tidy } from "./bits";
import { openThing } from "./Detail";
import { span } from "./Find";
import { openProfile } from "./Profile";
import { live } from "./settings";

/** Weekday names as JavaScript counts days: 0 is Sunday. */
const dayNames = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const dayPlural = ["Sundays", "Mondays", "Tuesdays", "Wednesdays", "Thursdays", "Fridays", "Saturdays"];

/** The hour as my clock setting writes it: "9pm" or "21". */
export function hourName(hour: number): string {
  const pick = live().timeFormat;
  const twelve = pick === "12" || (pick === "pc" && new Intl.DateTimeFormat([], { hour: "numeric" }).resolvedOptions().hour12 === true);
  if (!twelve) return String(hour).padStart(2, "0");
  return `${hour % 12 || 12}${hour < 12 ? "am" : "pm"}`;
}

/** Amounts per hour of the day and per weekday, in this PC's own time. */
function spread(points: Array<[string, number]>): { hours: number[]; days: number[] } {
  const hours = new Array<number>(24).fill(0);
  const days = new Array<number>(7).fill(0);
  for (const [at, amount] of points) {
    const start = Date.parse(at);
    if (Number.isNaN(start)) continue;
    // A long stay is spread over the hours it ran through, an hour at a time.
    // Nobody is together for more than a day at a stretch; a longer row is a stay the log never closed.
    let left = Math.min(Math.max(60, amount), 24 * 3600);
    let t = start;
    for (let guard = 0; left > 0 && guard < 30; guard++) {
      const d = new Date(t);
      const untilNext = 3600 - (d.getMinutes() * 60 + d.getSeconds());
      const part = Math.min(left, untilNext);
      hours[d.getHours()] += part;
      days[d.getDay()] += part;
      left -= part;
      t += part * 1000;
    }
  }
  return { hours, days };
}

export function Bars({ values, names, label }: { values: number[]; names: (i: number) => string; label: string }) {
  const most = Math.max(...values, 1);
  const best = values.indexOf(Math.max(...values));
  return (
    <div className={`habit-bars habit-bars-${values.length}`} role="img" aria-label={`${label}. Most at ${names(best)}.`}>
      {values.map((v, i) => (
        <span key={i} className={v > 0 && i === best ? "top" : ""} title={names(i)}>
          <i style={{ height: `${Math.round((v / most) * 100)}%` }} />
        </span>
      ))}
    </div>
  );
}

function When({ points, what }: { points: Array<[string, number]>; what: string }) {
  const { hours, days } = spread(points);
  if (hours.every((h) => h === 0)) return null;
  const bestHour = hours.indexOf(Math.max(...hours));
  const bestDay = days.indexOf(Math.max(...days));
  // The week starts on the day Settings says, the same as on the Events month view.
  const first = firstWeekday();
  const order = [0, 1, 2, 3, 4, 5, 6].map((i) => (first + i) % 7);
  return (
    <div className="habit-when">
      <p className="habit-line">
        {what} around {hourName(bestHour)}, most on {dayPlural[bestDay]}.
      </p>
      <div className="habit-charts">
        <div>
          <Bars values={hours} names={(i) => hourName(i)} label="By hour of the day" />
          <p className="habit-axis habit-axis-hours">
            {[0, 6, 12, 18].map((h) => (
              <span key={h}>{hourName(h)}</span>
            ))}
          </p>
        </div>
        <div>
          <Bars values={order.map((d) => days[d])} names={(i) => dayNames[order[i]]} label="By day of the week" />
          <p className="habit-axis habit-axis-days">
            {order.map((d) => (
              <span key={d}>{dayNames[d]}</span>
            ))}
          </p>
        </div>
      </div>
    </div>
  );
}

/** On a profile: where and when I meet this person, and where they go by themselves. From this PC's history only. */
export function Habits({ userId, name, friends, worlds }: { userId: string; name: string; friends: Map<string, Friend>; worlds: Map<string, World> }) {
  // A name the history lacks comes from the worlds the app has looked up, as soon as it has it.
  const worldName = (w: { worldId: string; name: string }) => tidy(w.name) || tidy(worlds.get(w.worldId)?.name ?? "") || "Unnamed world (private, removed, or still being looked up)";
  const [data, setData] = useState<HabitData | null>(null);
  const [problem, setProblem] = useState("");
  useEffect(() => {
    let alive = true;
    setData(null);
    setProblem("");
    void backend
      .habits(userId)
      .then((h) => alive && setData(h))
      .catch((err) => alive && setProblem(errorText(err)));
    return () => {
      alive = false;
    };
  }, [userId]);
  if (problem) return <p className="habit-fine">The history of where you meet could not be read: {problem}</p>;
  if (!data) return null;
  const withMe = data.worlds.length > 0;
  const alone = data.ownWorlds.length > 0 || data.seenAt.length > 2;
  if (!withMe && !alone) return null;

  return (
    <>
      {withMe && (
        <section className="habits">
          <h3>With you</h3>
          <p className="habit-fine">From the stays VRChat's log on this PC recorded. Nothing is asked of VRChat.</p>
          <div className="habit-cols">
            <div>
              <h4>Where you meet</h4>
              <ol className="habit-list">
                {data.worlds.map((w) => (
                  <li key={w.worldId || w.name}>
                    <button className="name-link world-name" disabled={!w.worldId} onClick={() => openThing("world", w.worldId)}>
                      {worldName(w)}
                    </button>
                    <span>
                      {w.times} {w.times === 1 ? "time" : "times"}, {span(w.seconds)}
                    </span>
                  </li>
                ))}
              </ol>
            </div>
            {data.company.length > 0 && (
              <div>
                <h4>Usually there too</h4>
                <ul className="chips">
                  {data.company.map((p) => {
                    const f = friends.get(p.userId);
                    return (
                      <li className={`chip chip-open${f ? " chip-friend" : ""}`} data-person={p.userId} data-person-name={p.name} role="button" tabIndex={0} key={p.userId} onClick={() => openProfile(p.userId)} onKeyDown={(e) => e.key === "Enter" && openProfile(p.userId)}>
                        <Avatar name={p.name} image={f?.image ?? ""} tone={f ? statusTone(f) : undefined} />
                        <span className="chip-text">
                          <span className="chip-name">{p.name}</span>
                          <span className="chip-note">{p.times} stays</span>
                        </span>
                      </li>
                    );
                  })}
                </ul>
              </div>
            )}
          </div>
          <When points={data.together} what="You two are mostly together" />
        </section>
      )}
      {alone && (
        <section className="habits">
          <h3>Where the app saw them, last 90 days</h3>
          <p className="habit-fine">
            From the friend activity the app recorded while it was open, the times you were there too included. It only knows what VRChat showed you about{" "}
            <span className="hide-me">{name}</span>.
          </p>
          {data.ownWorlds.length > 0 && (
            <>
              <h4>Worlds they went to most</h4>
              <ol className="habit-list">
                {data.ownWorlds.map((w) => (
                  <li key={w.worldId}>
                    <button className="name-link world-name" onClick={() => openThing("world", w.worldId)}>
                      {worldName(w)}
                    </button>
                    <span>
                      {w.times} {w.times === 1 ? "time" : "times"}
                    </span>
                  </li>
                ))}
              </ol>
            </>
          )}
          <When points={data.seenAt.map((at) => [at, 60] as [string, number])} what="The app sees them come online or move mostly" />
        </section>
      )}
    </>
  );
}
