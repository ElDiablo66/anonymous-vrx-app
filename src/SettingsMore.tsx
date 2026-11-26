import { useEffect, useState, type ReactNode } from "react";
import { backend, errorText, type GameCache } from "./backend";
import { clock, dateText } from "./bits";
import { headingFonts, rankDefaults, rankWords, textFonts, type RankKey, type Settings } from "./settings";

type Change = (patch: Partial<Settings>) => void;
type Props = { settings: Settings; change: Change };

export function Switch({ label, hint, on, set }: { label: string; hint?: string; on: boolean; set: (on: boolean) => void }) {
  return (
    <label className="setting">
      <span>
        <span className="setting-name">{label}</span>
        {hint && <span className="setting-hint">{hint}</span>}
      </span>
      <input type="checkbox" role="switch" checked={on} onChange={(e) => set(e.target.checked)} />
    </label>
  );
}

export function Row({ label, hint, children }: { label: string; hint?: ReactNode; children: ReactNode }) {
  return (
    <div className="setting">
      <span>
        <span className="setting-name">{label}</span>
        {hint && <span className="setting-hint">{hint}</span>}
      </span>
      {children}
    </div>
  );
}

export function Choices<T extends string>({ label, value, set, options }: { label: string; value: T; set: (value: T) => void; options: Array<[T, string]> }) {
  return (
    <div className="choices" role="radiogroup" aria-label={label}>
      {options.map(([id, words]) => (
        <button key={id} role="radio" aria-checked={value === id} className={value === id ? "on" : ""} onClick={() => set(id)}>
          {words}
        </button>
      ))}
    </div>
  );
}

/** A font out of a list, or the name of any other font installed on this PC. */
function FontPick({ label, hint, value, list, set }: { label: string; hint: string; value: string; list: string[]; set: (font: string) => void }) {
  const listed = list.includes(value);
  const [own, setOwn] = useState(listed ? "" : value);
  const [typing, setTyping] = useState(!listed);
  return (
    <Row label={label} hint={hint}>
      <span className="font-pick">
        <select
          aria-label={label}
          value={typing ? "other" : value}
          onChange={(e) => {
            if (e.target.value === "other") {
              setTyping(true);
              if (own.trim()) set(own.trim());
            } else {
              setTyping(false);
              set(e.target.value);
            }
          }}
        >
          {list.map((f, n) => (
            <option key={f} value={f}>
              {f.replace(" Variable", "")}
              {n === 0 ? " (the usual one)" : ""}
            </option>
          ))}
          <option value="other">Another font on this PC</option>
        </select>
        {typing && (
          <input
            type="text"
            value={own}
            maxLength={80}
            placeholder="Its name, as Windows lists it"
            aria-label={`${label}, name of the font`}
            onChange={(e) => {
              setOwn(e.target.value);
              if (e.target.value.trim()) set(e.target.value.trim());
            }}
          />
        )}
      </span>
    </Row>
  );
}

export function FontSettings({ settings, change }: Props) {
  return (
    <section data-group="look">
      <h2>Fonts</h2>
      <FontPick label="Headings" hint="Titles, names of screens and the big numbers." value={settings.headingFont} list={headingFonts} set={(headingFont) => change({ headingFont })} />
      <FontPick label="Everything else" hint="Lists, buttons and longer text." value={settings.textFont} list={textFonts} set={(textFont) => change({ textFont })} />
      <Switch label="Slanted headings" hint="Off sets every heading upright." on={settings.headingSlant} set={(headingSlant) => change({ headingSlant })} />
      <p className="font-sample" aria-hidden="true">
        <b>Friends online tonight</b>
        <span>The Pier, Friends+, 14 people. Joined at {clock(new Date().toISOString())}.</span>
      </p>
    </section>
  );
}

export function DateSettings({ settings, change }: Props) {
  const now = new Date().toISOString();
  return (
    <section data-group="look">
      <h2>Dates and times</h2>
      <Row label="Clock" hint={`Right now that reads ${clock(now)}.`}>
        <Choices
          label="Clock"
          value={settings.timeFormat}
          set={(timeFormat) => change({ timeFormat })}
          options={[
            ["pc", "Like Windows"],
            ["12", "12 hours"],
            ["24", "24 hours"],
          ]}
        />
      </Row>
      <Row label="A week starts on" hint="For the month view of Events.">
        <Choices
          label="A week starts on"
          value={settings.weekStart}
          set={(weekStart) => change({ weekStart })}
          options={[
            ["pc", "Like Windows"],
            ["mon", "Monday"],
            ["sun", "Sunday"],
            ["sat", "Saturday"],
          ]}
        />
      </Row>
      <Row label="Dates" hint={`Today reads ${dateText(now)}.`}>
        <Choices
          label="Dates"
          value={settings.dateFormat}
          set={(dateFormat) => change({ dateFormat })}
          options={[
            ["words", "In words"],
            ["iso", "Year-month-day"],
          ]}
        />
      </Row>
    </section>
  );
}

export function PeopleColourSettings({ settings, change, swatch }: Props & { swatch: (label: string, value: string, set: (value: string) => void) => ReactNode }) {
  const ranks = Object.keys(rankDefaults) as RankKey[];
  const changed = Object.keys(settings.rankColours).length > 0;
  return (
    <section data-group="look">
      <h2>Colours of names</h2>
      <Row label="A name in a list is coloured" hint="One colour per person gives everyone a colour of their own that never changes, so a name is easy to find again.">
        <Choices
          label="Colour of names"
          value={settings.nameColours}
          set={(nameColours) => change({ nameColours })}
          options={[
            ["rank", "By trust rank"],
            ["person", "One per person"],
            ["plain", "Not at all"],
          ]}
        />
      </Row>
      <Row label="Trust rank colours" hint="Used for rank labels on profiles and in search, and for names when they follow the rank.">
        <span className="swatches rank-swatches">
          {ranks.map((key) => (
            <span key={key} style={{ color: settings.rankColours[key] ?? rankDefaults[key] }}>
              {swatch(rankWords[key], settings.rankColours[key] ?? rankDefaults[key], (value) => change({ rankColours: { ...settings.rankColours, [key]: value } }))}
            </span>
          ))}
          <button className="plain" disabled={!changed} onClick={() => change({ rankColours: {} })}>
            The usual colours
          </button>
        </span>
      </Row>
    </section>
  );
}

export function ProfileLookSettings({ settings, change }: Props) {
  return (
    <section data-group="look">
      <h2>Profiles</h2>
      <Switch
        label="Icon frames and profile effects"
        hint="Shows the frame around a person's picture and the effect they picked in VRChat."
        on={settings.profileCosmetics}
        set={(profileCosmetics) => change({ profileCosmetics })}
      />
      <Switch
        label="Tint a profile with the person's banner"
        hint="The window takes on their banner colour, or a soft copy of their banner picture."
        on={settings.profileBackdrop}
        set={(profileBackdrop) => change({ profileBackdrop })}
      />
      {settings.profileBackdrop && (
        <Row label="How strong the tint is" hint="Stronger looks more like their profile and makes text a little harder to read.">
          <label className="dim">
            <input
              type="range"
              min={0.1}
              max={0.9}
              step={0.05}
              value={settings.profileBackdropStrength}
              aria-label="How strong the tint is"
              onChange={(e) => change({ profileBackdropStrength: Number(e.target.value) })}
            />
            {Math.round(settings.profileBackdropStrength * 100)}%
          </label>
        </Row>
      )}
    </section>
  );
}

export function InstanceSettings({ settings, change }: Props) {
  return (
    <section data-group="general">
      <h2>Instances</h2>
      <Switch label="Show an instance's number" hint="The #12345 next to what kind of instance it is." on={settings.showInstanceId} set={(showInstanceId) => change({ showInstanceId })} />
      <Switch
        label="List 18+ instances"
        hint="Instances only people VRChat has verified as 18 or older get into. They carry an 18+ mark. Off leaves them out of the lists of open instances."
        on={settings.showAgeGated}
        set={(showAgeGated) => change({ showAgeGated })}
      />
      <p className="setting-hint">Closed instances have no switch: the app only lists open ones, and your world history says so when one has closed.</p>
    </section>
  );
}

export function PerformanceSettings() {
  const [off, setOff] = useState(false);
  const [started, setStarted] = useState(false);
  const [problem, setProblem] = useState("");
  useEffect(() => {
    void backend
      .getNoGpu()
      .then((v) => {
        setOff(v.wanted);
        setStarted(v.now);
      })
      .catch(() => {});
  }, []);
  return (
    <section data-group="general">
      <h2>Drawing</h2>
      <Switch
        label="Draw without the graphics card"
        hint="For when the window flickers, shows black areas, or takes graphics memory you want VRChat to have. The window may scroll less smoothly."
        on={off}
        set={(on) => {
          setProblem("");
          setOff(on);
          void backend.setNoGpu(on).catch((err) => {
            setOff(!on);
            setProblem(errorText(err));
          });
        }}
      />
      {off !== started && (
        <div className="data-actions">
          <span className="setting-hint">This counts from the next time the app starts.</span>
          <button className="plain go" onClick={() => void backend.restartApp()}>
            Restart the app now
          </button>
        </div>
      )}
      {problem && (
        <p className="setting-hint" role="alert">
          That could not be saved: {problem}
        </p>
      )}
    </section>
  );
}

function gb(bytes: number): string {
  const g = bytes / 1024 ** 3;
  return g >= 10 ? g.toFixed(1) : g.toFixed(2);
}

export function GameCacheSettings() {
  const [info, setInfo] = useState<GameCache | null>(null);
  const [problem, setProblem] = useState("");
  const [asking, setAsking] = useState<"" | "old" | "all">("");
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState("");
  const [limit, setLimit] = useState("");

  const count = () => {
    setProblem("");
    return backend
      .gameCache()
      .then((i) => {
        setInfo(i);
        setLimit(String(i.limitGb));
      })
      .catch((err) => setProblem(errorText(err)));
  };
  useEffect(() => void count(), []);

  async function clear(oldOnly: boolean) {
    setBusy(true);
    setNote("");
    try {
      const done = await backend.clearGameCache(oldOnly);
      setNote(
        `Freed ${gb(done.bytes)} GB.${done.kept > 0 ? ` ${done.kept} ${done.kept === 1 ? "folder" : "folders"} could not be removed because Windows has them in use.` : ""} VRChat downloads a world or avatar again the next time it needs it.`,
      );
      await count();
    } catch (err) {
      setNote(`Nothing was cleared: ${errorText(err)}`);
    } finally {
      setBusy(false);
      setAsking("");
    }
  }

  async function saveLimit() {
    setNote("");
    try {
      const written = await backend.setGameCacheLimit(Math.round(Number(limit)) || 30);
      setLimit(String(written));
      setInfo((old) => (old ? { ...old, limitGb: written } : old));
      setNote(`VRChat keeps its cache under ${written} GB from its next start.`);
    } catch (err) {
      setNote(`The size was not saved: ${errorText(err)}`);
    }
  }

  const share = info ? Math.min(1, info.bytes / (info.limitGb * 1024 ** 3)) : 0;
  return (
    <section data-group="vr">
      <h2>VRChat's cache</h2>
      <p className="setting-hint">
        VRChat keeps every world and avatar it has downloaded, so the next visit loads fast. Clearing it frees disk space and costs nothing but a
        second download.
      </p>
      {problem && (
        <p className="setting-hint" role="alert">
          The cache could not be measured: {problem}
        </p>
      )}
      {!info && !problem && <p className="setting-hint">Counting what is in the cache</p>}
      {info && !info.found && <p className="setting-hint">No cache folder at {info.path}. VRChat makes it the first time it runs.</p>}
      {info && info.found && (
        <>
          <div className="cache-meter" role="img" aria-label={`${gb(info.bytes)} of ${info.limitGb} GB used`}>
            <span style={{ width: `${Math.max(1, share * 100)}%` }} className={share > 0.9 ? "full" : ""} />
          </div>
          <p className="cache-line">
            <b>
              {gb(info.bytes)} GB of {info.limitGb} GB
            </b>
            <span>
              {info.items.toLocaleString()} worlds and avatars
              {info.oldVersions > 0 ? `, ${info.oldVersions.toLocaleString()} old versions taking ${gb(info.oldBytes)} GB` : ""}
            </span>
          </p>
          {asking === "" ? (
            <div className="data-actions">
              <button className="plain" disabled={busy || info.oldVersions === 0} onClick={() => setAsking("old")} title="Removes versions a newer download has replaced. Nothing has to be downloaded again">
                Clear old versions
              </button>
              <button className="plain" disabled={busy || info.bytes === 0} onClick={() => setAsking("all")}>
                Clear the whole cache
              </button>
              <button className="plain" disabled={busy} onClick={() => void count()}>
                Count again
              </button>
            </div>
          ) : (
            <div className="data-actions backup-ask">
              <span>
                {asking === "old"
                  ? `Remove ${info.oldVersions.toLocaleString()} old versions (${gb(info.oldBytes)} GB)? The newest of each stays.`
                  : `Remove all ${gb(info.bytes)} GB? VRChat downloads each world and avatar again when it next needs it.`}
              </span>
              <button className="plain go" disabled={busy} onClick={() => void clear(asking === "old")}>
                {busy ? "Clearing" : "Yes, clear it"}
              </button>
              <button className="plain" disabled={busy} onClick={() => setAsking("")}>
                No
              </button>
            </div>
          )}
          <Row label="Largest size" hint="In GB. VRChat does not go under 30. It reads this when it starts.">
            <span className="quiet-times">
              <input type="number" min={30} max={2000} step={5} value={limit} aria-label="Largest cache size in GB" onChange={(e) => setLimit(e.target.value)} />
              <button className="plain" disabled={!limit || Number(limit) === info.limitGb} onClick={() => void saveLimit()}>
                Save
              </button>
            </span>
          </Row>
        </>
      )}
      {note && (
        <p className="setting-hint" role="status">
          {note}
        </p>
      )}
    </section>
  );
}

export function VrNoticeSettings({ settings, change }: Props) {
  const [results, setResults] = useState<Array<{ target: string; ok: boolean; problem: string }> | null>(null);
  const [testing, setTesting] = useState(false);
  const any = settings.vrXs || settings.vrOvrHud || settings.vrOvrWrist;

  async function test() {
    setTesting(true);
    setResults(null);
    try {
      setResults(await backend.testVrNotice({ own: false, xs: settings.vrXs, ovrHud: settings.vrOvrHud, ovrWrist: settings.vrOvrWrist, seconds: settings.vrSeconds, opacity: settings.vrOpacity }));
    } catch (err) {
      setResults([{ target: "The test", ok: false, problem: errorText(err) }]);
    } finally {
      setTesting(false);
    }
  }

  return (
    <section data-group="vr">
      <h2>
        Notifications in the headset <span className="mark">Beta</span>
      </h2>
      <p className="setting-hint">
        While VRChat runs, everything this app would tell Windows is also handed to an overlay program on this PC, which shows it in VR. Quiet
        hours still hold. Which notifications are sent is set under Notifications. This part is new and has seen little testing in a headset.
      </p>
      <Switch label="XSOverlay" hint="Shown where XSOverlay shows its own notifications." on={settings.vrXs} set={(vrXs) => change({ vrXs })} />
      <Switch label="OVR Toolkit, in view" hint="A notice in front of you." on={settings.vrOvrHud} set={(vrOvrHud) => change({ vrOvrHud })} />
      <Switch label="OVR Toolkit, on the wrist" hint="One line on the wrist overlay." on={settings.vrOvrWrist} set={(vrOvrWrist) => change({ vrOvrWrist })} />
      {settings.vrXs && (
        <>
          <Row label="Stays up for" hint="XSOverlay only.">
            <div className="choices" role="radiogroup" aria-label="Seconds a notice stays up">
              {[2, 4, 6, 10].map((n) => (
                <button key={n} role="radio" aria-checked={settings.vrSeconds === n} className={settings.vrSeconds === n ? "on" : ""} onClick={() => change({ vrSeconds: n })}>
                  {n} s
                </button>
              ))}
            </div>
          </Row>
          <Row label="How solid it is" hint="XSOverlay only. Lower lets the world show through.">
            <label className="dim">
              <input type="range" min={0.2} max={1} step={0.05} value={settings.vrOpacity} aria-label="How solid a notice is" onChange={(e) => change({ vrOpacity: Number(e.target.value) })} />
              {Math.round(settings.vrOpacity * 100)}%
            </label>
          </Row>
        </>
      )}
      <div className="data-actions">
        <button className="plain" disabled={!any || testing} onClick={() => void test()}>
          {testing ? "Sending" : "Send a test notice"}
        </button>
      </div>
      {results && (
        <ul className="vr-results" role="status">
          {results.map((r) => (
            <li key={r.target} className={r.ok ? "ok" : "bad"}>
              <b>{r.target}</b> {r.ok ? "took it. Look in the headset." : r.problem}
            </li>
          ))}
        </ul>
      )}
      <p className="setting-hint">One of these programs has to be running. They are made by other people and are not part of this app.</p>
    </section>
  );
}
