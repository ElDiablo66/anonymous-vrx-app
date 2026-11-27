import { useEffect, useRef, useState } from "react";
import { backend, errorText, type Friend, type MyStats, type World } from "./backend";
import { Avatar, firstWeekday, tidy } from "./bits";
import { openThing } from "./Detail";
import { span } from "./Find";
import { Bars, hourName } from "./Habits";
import { openProfile } from "./Profile";

type Range = "7d" | "30d" | "year" | "all";
const ranges: Array<[Range, string]> = [
  ["7d", "7 days"],
  ["30d", "30 days"],
  ["year", "A year"],
  ["all", "All time"],
];

const weekdayNames = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const weekdayPlural = ["Sundays", "Mondays", "Tuesdays", "Wednesdays", "Thursdays", "Fridays", "Saturdays"];

/** "2026-11-25" for a date, in the PC's own time. */
function keyOf(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/** Hours as the big numbers show them: "12.5 h", "40 min". */
function hoursText(seconds: number): string {
  if (seconds < 3600) return `${Math.round(seconds / 60)} min`;
  const h = seconds / 3600;
  return `${h >= 100 ? Math.round(h) : Math.round(h * 10) / 10} h`;
}

/** The bars of the main chart: one per day for a week or a month, one per week or month for longer. */
function buckets(data: MyStats, range: Range): Array<{ label: string; tip: string; seconds: number }> {
  const played = new Map(data.days);
  const start = new Date(data.from);
  const today = new Date();
  start.setHours(0, 0, 0, 0);
  const days: Array<[Date, number]> = [];
  for (let d = new Date(start), guard = 0; d <= today && guard < 4000; d.setDate(d.getDate() + 1), guard++) days.push([new Date(d), played.get(keyOf(d)) ?? 0]);
  if (range === "7d" || range === "30d") {
    return days.map(([d, s]) => ({
      label: range === "7d" ? weekdayNames[d.getDay()] : String(d.getDate()),
      tip: `${d.toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" })}: ${s ? span(s) : "no play"}`,
      seconds: s,
    }));
  }
  const first = firstWeekday();
  const byWeek = days.length <= 7 * 70;
  const out: Array<{ label: string; tip: string; seconds: number; key: string }> = [];
  for (const [d, s] of days) {
    const head = new Date(d);
    if (byWeek) head.setDate(d.getDate() - ((d.getDay() - first + 7) % 7));
    else head.setDate(1);
    const key = keyOf(head);
    const last = out[out.length - 1];
    if (last && last.key === key) last.seconds += s;
    else
      out.push({
        key,
        seconds: s,
        label: byWeek ? `${head.getDate()} ${head.toLocaleDateString([], { month: "short" })}` : head.toLocaleDateString([], { month: "short", year: "2-digit" }),
        tip: "",
      });
  }
  return out.map((b) => ({ ...b, tip: `${byWeek ? "Week of" : ""} ${b.label}: ${b.seconds ? span(b.seconds) : "no play"}`.trim() }));
}

/** History, In numbers: how long I played, when, where and with whom. From this PC's history only. */
export function MyNumbers({ worlds, friends, tick }: { worlds: Map<string, World>; friends: Map<string, Friend>; tick: number }) {
  const [range, setRange] = useState<Range>("30d");
  // The answer is kept with the time span it is for, so a switch never shows another span's numbers.
  const [answer, setAnswer] = useState<{ range: Range; d: MyStats } | null>(null);
  const [problem, setProblem] = useState("");
  const [othersOpen, setOthersOpen] = useState(false);
  const asked = useRef({ range: "", at: 0 });
  const latest = useRef(0);

  useEffect(() => {
    const read = () => {
      asked.current = { range, at: Date.now() };
      // Only the newest answer counts, so a slow one for another time span cannot land last.
      const n = ++latest.current;
      setProblem("");
      void backend
        .myStats(range)
        .then((d) => n === latest.current && setAnswer({ range, d }))
        .catch((err) => n === latest.current && setProblem(errorText(err)));
    };
    // The history changes with every join and leave. The numbers are read again at most once a
    // minute for that; a change inside the minute is read when the minute is over.
    const wait = asked.current.range === range ? 60_000 - (Date.now() - asked.current.at) : 0;
    if (wait <= 0) {
      read();
      return;
    }
    const later = window.setTimeout(read, wait);
    return () => window.clearTimeout(later);
  }, [range, tick]);
  const data = answer && answer.range === range ? answer.d : null;

  const picker = (
    <div className="filters num-ranges" role="radiogroup" aria-label="Time span">
      {ranges.map(([id, label]) => (
        <button key={id} role="radio" aria-checked={range === id} className={range === id ? "on" : ""} onClick={() => setRange(id)}>
          {label}
        </button>
      ))}
    </div>
  );

  if (problem) {
    return (
      <div className="numbers">
        {picker}
        <p className="banner" role="alert">
          The numbers could not be worked out: {problem}
        </p>
      </div>
    );
  }
  if (!data) {
    return (
      <div className="numbers">
        {picker}
        <p className="empty">Adding up your history</p>
      </div>
    );
  }
  if (data.totalSeconds === 0) {
    return (
      <div className="numbers">
        {picker}
        <p className="empty">Nothing in this time span yet. The numbers come from VRChat's own log files on this PC, so play a session and they fill in.</p>
      </div>
    );
  }

  const bars = buckets(data, range);
  const most = Math.max(...bars.map((b) => b.seconds), 1);
  const peak = bars.reduce((best, b, i) => (b.seconds > bars[best].seconds ? i : best), 0);
  const perDay = data.playDays > 0 ? data.totalSeconds / data.playDays : 0;
  const topWorld = Math.max(...data.worlds.map((w) => w.seconds), 1);
  const friendsTop = data.people.filter((p) => p.friend);
  const others = data.people.filter((p) => !p.friend);
  const first = firstWeekday();
  const order = [0, 1, 2, 3, 4, 5, 6].map((i) => (first + i) % 7);
  const bestHour = data.hours.indexOf(Math.max(...data.hours));
  const bestDay = data.weekdays.indexOf(Math.max(...data.weekdays));
  const worldName = (id: string, name: string) => tidy(name) || tidy(worlds.get(id)?.name ?? "") || "A world without a name yet";
  const labelEvery = Math.max(1, Math.ceil(bars.length / 10));

  return (
    <div className="numbers">
      {picker}
      <section className="num-sums" aria-label="Totals">
        <div className="num-lead">
          <span>Hours played</span>
          <b>{hoursText(data.totalSeconds)}</b>
          <small>
            {data.stays} {data.stays === 1 ? "stay" : "stays"} in {data.worldsCount} {data.worldsCount === 1 ? "world" : "worlds"}
          </small>
        </div>
        <dl>
          <div>
            <dt>Days you played</dt>
            <dd>{data.playDays}</dd>
          </div>
          <div>
            <dt>On a day you played</dt>
            <dd>{hoursText(perDay)}</dd>
          </div>
          <div>
            <dt>People you met</dt>
            <dd>{data.peopleCount}</dd>
          </div>
          {data.longest && (
            <div className="num-longest">
              <dt>Longest stay</dt>
              <dd>{span(data.longest.seconds)}</dd>
              <small>
                in{" "}
                {data.longest.worldId ? (
                  <button className="linkish" onClick={() => openThing("world", data.longest!.worldId)}>
                    {worldName(data.longest.worldId, data.longest.worldName)}
                  </button>
                ) : (
                  worldName("", data.longest.worldName)
                )}
              </small>
            </div>
          )}
        </dl>
      </section>

      <section className="num-card" aria-label="Time played">
        <h3>{range === "7d" || range === "30d" ? "Each day" : bars.length > 0 && bars[0].tip.startsWith("Week") ? "Each week" : "Each month"}</h3>
        <div className="num-bars" role="img" aria-label={`Time played. Most: ${bars[peak]?.tip ?? ""}.`}>
          {bars.map((b, i) => (
            <span key={i} className={b.seconds === 0 ? "none" : i === peak ? "top" : ""} title={b.tip}>
              <i style={{ height: `${b.seconds ? Math.max(4, Math.round((b.seconds / most) * 100)) : 2}%` }} />
              <small>{(bars.length - 1 - i) % labelEvery === 0 ? b.label : ""}</small>
            </span>
          ))}
        </div>
      </section>

      <div className="num-columns">
        <section className="num-card" aria-label="Where you spend your time">
          <h3>Where you spend your time</h3>
          <ol className="num-worlds">
            {data.worlds.map((w) => {
              const pic = worlds.get(w.worldId)?.image ?? "";
              return (
                <li key={w.worldId || w.name}>
                  <span className="thumb num-thumb" style={pic ? { backgroundImage: `url("${pic}")` } : undefined} />
                  <span className="num-what">
                    {w.worldId ? (
                      <button className="linkish num-name" onClick={() => openThing("world", w.worldId)}>
                        {worldName(w.worldId, w.name)}
                      </button>
                    ) : (
                      <span className="num-name">{worldName("", w.name)}</span>
                    )}
                    <span className="num-share" aria-hidden="true">
                      <i style={{ width: `${Math.max(3, Math.round((w.seconds / topWorld) * 100))}%` }} />
                    </span>
                  </span>
                  <span className="num-time">
                    {span(w.seconds)}
                    <small>
                      {w.times} {w.times === 1 ? "stay" : "stays"}
                    </small>
                  </span>
                </li>
              );
            })}
          </ol>
        </section>

        <section className="num-card" aria-label="Who you spend it with">
          <h3>Who you spend it with</h3>
          {friendsTop.length === 0 ? (
            <p className="tile-note">No friends in your stays in this time span.</p>
          ) : (
            <ol className="num-people">
              {friendsTop.map((p) => (
                <li key={p.userId || p.name}>
                  <button className="num-person" onClick={() => p.userId && openProfile(p.userId)} disabled={!p.userId}>
                    <Avatar name={p.name} image={friends.get(p.userId)?.image ?? ""} size={34} />
                    <span className="hide-name num-name">{tidy(friends.get(p.userId)?.displayName ?? p.name)}</span>
                    <span className="num-time">
                      {span(p.seconds)}
                      <small>
                        {p.times} {p.times === 1 ? "stay" : "stays"}
                      </small>
                    </span>
                  </button>
                </li>
              ))}
            </ol>
          )}
          {others.length > 0 && (
            <>
              <button className="linkish num-others-toggle" aria-expanded={othersOpen} onClick={() => setOthersOpen(!othersOpen)}>
                {othersOpen ? "Hide people not on your friend list" : "People not on your friend list"}
              </button>
              {othersOpen && (
                <ol className="num-people num-others">
                  {others.map((p) => (
                    <li key={p.userId || p.name}>
                      <button className="num-person" onClick={() => p.userId && openProfile(p.userId)} disabled={!p.userId}>
                        <Avatar name={p.name} image="" size={30} />
                        <span className="hide-name num-name">{tidy(p.name)}</span>
                        <span className="num-time">{span(p.seconds)}</span>
                      </button>
                    </li>
                  ))}
                </ol>
              )}
            </>
          )}
        </section>
      </div>

      <section className="num-card" aria-label="When you play">
        <h3>When you play</h3>
        <p className="habit-line">
          Most around {hourName(bestHour)}, and most on {weekdayPlural[bestDay]}.
        </p>
        <div className="habit-charts">
          <div>
            <Bars values={data.hours} names={(i) => hourName(i)} label="By hour of the day" />
            <p className="habit-axis habit-axis-hours">
              {[0, 6, 12, 18].map((h) => (
                <span key={h}>{hourName(h)}</span>
              ))}
            </p>
          </div>
          <div>
            <Bars values={order.map((d) => data.weekdays[d])} names={(i) => weekdayNames[order[i]]} label="By day of the week" />
            <p className="habit-axis habit-axis-days">
              {order.map((d) => (
                <span key={d}>{weekdayNames[d]}</span>
              ))}
            </p>
          </div>
        </div>
      </section>

      <p className="setting-hint num-note">
        From VRChat's log files on this PC. VRChat is only asked for the names of worlds the app has not looked up yet. Time with the same person, or two records of the same stay, counts once.
        {data.openNow && " The stay you are in now counts up to this minute."}
        {data.capped > 0 &&
          ` ${data.capped === 1 ? "One stay" : `${data.capped} stays`} ran for over a day without VRChat closing ${data.capped === 1 ? "it" : "them"}; only the first 24 hours count.`}
      </p>
    </div>
  );
}
