import { useEffect, useRef, useState } from "react";
import { backend, errorText, type FoundPerson, type FoundWorld, type Friend, type LogStay, type Sighting } from "./backend";
import { Avatar, accessName, clock, dateText, dayLabel, statusTone, tidy } from "./bits";
import { openThing } from "./Detail";
import { parsePlace } from "./place";
import { openProfile } from "./Profile";

/** "9 h 20 min", "3 days 4 h", for time spent. */
export function span(seconds: number): string {
  const minutes = Math.round(seconds / 60);
  if (minutes < 1) return "under a minute";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return minutes % 60 ? `${hours} h ${minutes % 60} min` : `${hours} h`;
  return `${Math.floor(hours / 24)} days ${hours % 24} h`;
}

type Pick = { kind: "person"; person: FoundPerson } | { kind: "world"; world: FoundWorld };
const keyOf = (p: Pick) => (p.kind === "person" ? `p:${p.person.userId || p.person.name}` : `w:${p.world.worldId || p.world.name}`);

/** History, Find: one box over my own history. Who I saw, where I was, and every stay behind it. */
export function Find({ friends }: { friends: Map<string, Friend> }) {
  const [words, setWords] = useState("");
  const [found, setFound] = useState<{ people: FoundPerson[]; worlds: FoundWorld[] } | null>(null);
  const [asked, setAsked] = useState("");
  const [looking, setLooking] = useState(false);
  const [problem, setProblem] = useState("");
  const [pick, setPick] = useState<Pick | null>(null);
  const box = useRef<HTMLInputElement>(null);

  useEffect(() => box.current?.focus(), []);

  // The search runs a moment after the typing stops.
  useEffect(() => {
    const q = words.trim();
    if (q.length < 2) {
      setFound(null);
      setAsked("");
      setLooking(false);
      setProblem("");
      return;
    }
    let alive = true;
    setLooking(true);
    const id = window.setTimeout(() => {
      void backend
        .logFind(q)
        .then((r) => {
          if (!alive) return;
          setFound(r);
          setAsked(q);
          setProblem("");
          // The first hit opens by itself, so one name typed gives the whole answer.
          const first: Pick | null = r.people[0] ? { kind: "person", person: r.people[0] } : r.worlds[0] ? { kind: "world", world: r.worlds[0] } : null;
          setPick((old) => {
            const all: Pick[] = [...r.people.map((person) => ({ kind: "person" as const, person })), ...r.worlds.map((world) => ({ kind: "world" as const, world }))];
            return (old && all.find((p) => keyOf(p) === keyOf(old))) || first;
          });
        })
        .catch((err) => alive && setProblem(errorText(err)))
        .finally(() => alive && setLooking(false));
    }, 260);
    return () => {
      alive = false;
      window.clearTimeout(id);
    };
  }, [words]);

  const none = found !== null && found.people.length + found.worlds.length === 0;

  return (
    <div className="find">
      <label className="find-box">
        <span className="find-label">Find in my history</span>
        <input ref={box} type="search" value={words} onChange={(e) => setWords(e.target.value)} placeholder="A person's name, or part of a world's name" aria-label="Find in my history" />
      </label>

      {problem && (
        <p className="banner" role="alert">
          {problem}
        </p>
      )}

      {found === null && !looking && (
        <div className="find-start">
          <p>Type a name to see when you last saw someone, how often and for how long, and every stay you shared.</p>
          <p>Type part of a world's name to see each time you were there, and who was around.</p>
          <p className="find-fine">Everything here comes from VRChat's own log on this PC, from the day the app first read it. Nothing is asked of VRChat.</p>
        </div>
      )}

      {found === null && looking && (
        <ul className="find-wait" aria-label="Looking">
          <li />
          <li />
          <li />
        </ul>
      )}

      {none && (
        <p className="empty">
          Nothing in your history has "{asked}" in its name. The history holds the people and worlds VRChat's log on this PC named while the app was reading it.
        </p>
      )}

      {found !== null && !none && (
        <div className="find-cols">
          <div className="find-hits" aria-busy={looking}>
            {found.people.length > 0 && (
              <section>
                <h3>People</h3>
                <ul>
                  {found.people.map((p) => {
                    const friend = friends.get(p.userId);
                    const mine = pick?.kind === "person" && keyOf(pick) === keyOf({ kind: "person", person: p });
                    return (
                      <li key={p.userId || p.name}>
                        <button className={mine ? "find-hit on" : "find-hit"} aria-pressed={mine} onClick={() => setPick({ kind: "person", person: p })}>
                          <Avatar name={p.name} image={friend?.image ?? ""} tone={friend ? statusTone(friend) : undefined} />
                          <span className="find-hit-text">
                            <b className="hide-name">{p.name}</b>
                            <small>
                              {p.times} {p.times === 1 ? "time" : "times"}, last {dayLabel(p.last).toLowerCase()}
                              {p.matched !== p.name && (
                                <>
                                  , then <span className="hide-name">"{p.matched}"</span>
                                </>
                              )}
                            </small>
                          </span>
                          {friend && <span className="mark">Friend</span>}
                        </button>
                      </li>
                    );
                  })}
                </ul>
              </section>
            )}
            {found.worlds.length > 0 && (
              <section>
                <h3>Worlds</h3>
                <ul>
                  {found.worlds.map((w) => {
                    const mine = pick?.kind === "world" && keyOf(pick) === keyOf({ kind: "world", world: w });
                    return (
                      <li key={w.worldId || w.name}>
                        <button className={mine ? "find-hit on" : "find-hit"} aria-pressed={mine} onClick={() => setPick({ kind: "world", world: w })}>
                          <span className="find-hit-text">
                            <b>{tidy(w.name) || "A world without a name"}</b>
                            <small>
                              {w.times} {w.times === 1 ? "visit" : "visits"}, last {dayLabel(w.last).toLowerCase()}
                            </small>
                          </span>
                        </button>
                      </li>
                    );
                  })}
                </ul>
              </section>
            )}
          </div>
          {pick && <Stays key={keyOf(pick)} pick={pick} friends={friends} />}
        </div>
      )}
    </div>
  );
}

/** Everything behind one hit: the sums, then each stay, which opens to show who was there. */
function Stays({ pick, friends }: { pick: Pick; friends: Map<string, Friend> }) {
  const [stays, setStays] = useState<LogStay[] | null>(null);
  const [open, setOpen] = useState<number | null>(null);
  const [people, setPeople] = useState<Sighting[] | null>(null);
  const [id, name] = pick.kind === "person" ? [pick.person.userId, pick.person.userId ? pick.person.name : pick.person.matched] : [pick.world.worldId, pick.world.name];

  useEffect(() => {
    let alive = true;
    void backend
      .logStays(pick.kind, id, name)
      .then((list) => alive && setStays(list))
      .catch(() => alive && setStays([]));
    return () => {
      alive = false;
    };
  }, [pick.kind, id, name]);

  useEffect(() => {
    if (open === null) return;
    let alive = true;
    setPeople(null);
    void backend
      .visitPeople(open)
      .then((list) => alive && setPeople(list))
      .catch(() => alive && setPeople([]));
    return () => {
      alive = false;
    };
  }, [open]);

  const person = pick.kind === "person" ? pick.person : null;
  const world = pick.kind === "world" ? pick.world : null;
  const title = person ? person.name : tidy(world!.name) || "A world without a name";
  let lastDay = "";

  return (
    <article className="find-detail">
      <header>
        <h3 className={person ? "hide-name" : undefined}>{title}</h3>
        {person && person.userId && (
          <button className="plain" onClick={() => openProfile(person.userId)}>
            Open profile
          </button>
        )}
        {world && world.worldId && (
          <button className="plain" onClick={() => openThing("world", world.worldId)}>
            Open the world
          </button>
        )}
      </header>
      <dl className="find-sums">
        <div>
          <dt>{person ? "Times together" : "Visits"}</dt>
          <dd>{person ? person.times : world!.times}</dd>
        </div>
        <div>
          <dt>{person ? "Time together" : "Time there"}</dt>
          <dd>{span(person ? person.seconds : world!.seconds)}</dd>
        </div>
        {person && (
          <div>
            <dt>First seen</dt>
            <dd>{person.first ? dateText(person.first, "short") : "Not known"}</dd>
          </div>
        )}
        <div>
          <dt>{person ? "Last seen" : "Last visit"}</dt>
          <dd>{dateText(person ? person.last : world!.last, "short")}</dd>
        </div>
      </dl>
      {person && (
        <p className="find-last">
          Last together in{" "}
          {person.lastWorldId ? (
            <button className="name-link" onClick={() => openThing("world", person.lastWorldId)}>
              {tidy(person.lastWorld) || "a world"}
            </button>
          ) : (
            tidy(person.lastWorld) || "a world"
          )}
          {accessName(parsePlace(person.lastLocation)) && `, ${accessName(parsePlace(person.lastLocation))}`}.
        </p>
      )}

      {stays === null ? (
        <ul className="find-wait" aria-label="Looking">
          <li />
          <li />
        </ul>
      ) : (
        <ol className="find-stays">
          {stays.map((s) => {
            const label = dayLabel(s.from);
            const head = label !== lastDay;
            lastDay = label;
            const isOpen = open === s.visitId;
            const others = s.people;
            return (
              <li key={s.visitId}>
                {head && <p className="find-day">{label}</p>}
                <button className="find-stay" aria-expanded={isOpen} onClick={() => setOpen(isOpen ? null : s.visitId)} title="Show who was there">
                  <time dateTime={s.from}>{clock(s.from)}</time>
                  <span className="find-stay-text">
                    <b>{person ? tidy(s.worldName) || "A world" : accessName(parsePlace(s.location)) || "Instance"}</b>
                    <small>
                      {person ? "together " : ""}
                      {span(s.seconds)}
                      {others > 0 ? `, ${others} ${others === 1 ? "other" : "others"} there` : person ? ", just the two of you" : ", nobody else seen"}
                    </small>
                  </span>
                </button>
                {isOpen && (
                  <div className="find-there">
                    {people === null ? (
                      <p className="find-fine">Looking up who was there.</p>
                    ) : people.length === 0 ? (
                      <p className="find-fine">The log named nobody for this stay.</p>
                    ) : (
                      <ul className="chips">
                        {people.map((p, at) => {
                          const f = friends.get(p.userId);
                          return (
                            <li
                              className={`chip${f ? " chip-friend" : ""}${p.userId ? " chip-open" : ""}`}
                              data-person={p.userId || undefined}
                              data-person-name={p.displayName}
                              role={p.userId ? "button" : undefined}
                              tabIndex={p.userId ? 0 : undefined}
                              key={`${at}${p.userId}${p.displayName}`}
                              onClick={() => p.userId && openProfile(p.userId)}
                              onKeyDown={(e) => e.key === "Enter" && p.userId && openProfile(p.userId)}
                            >
                              <Avatar name={p.displayName} image={f?.image ?? ""} tone={f ? statusTone(f) : undefined} />
                              <span className="chip-text">
                                <span className="chip-name">{p.displayName}</span>
                              </span>
                            </li>
                          );
                        })}
                      </ul>
                    )}
                    <button className="plain" onClick={() => openThing("instance", s.location)}>
                      Open this instance
                    </button>
                  </div>
                )}
              </li>
            );
          })}
          {stays.length === 200 && <li className="find-fine">The newest 200 stays are listed.</li>}
        </ol>
      )}
    </article>
  );
}
