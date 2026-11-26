import type { Settings } from "./settings";
import { Switch } from "./SettingsMore";
import { useState } from "react";
import { clock } from "./bits";
import { clearRuleLog, mostRules, newRule, ruleLog, statusNames, useRuleNow, type StatusRule, type StatusValue } from "./statusrules";

type Change = (patch: Partial<Settings>) => void;

const whens: Array<[StatusRule["when"], string]> = [
  ["access", "I am in an instance that is"],
  ["time", "the time is between"],
  ["world", "the world's name has"],
  ["friends", "friends in my instance are at least"],
];

const kinds: Array<[StatusRule["access"], string]> = [
  ["invite", "Invite or Invite+"],
  ["friends", "Friends or Friends+"],
  ["group", "a group instance"],
  ["public", "Public"],
];

const dots: Record<StatusValue, string> = { "join me": "joinme", active: "active", "ask me": "askme", busy: "busy" };

/** Settings: my status changes by itself when a rule fits, and comes back afterwards. */
export function StatusRuleSettings({ settings, change }: { settings: Settings; change: Change }) {
  const rules = settings.statusRules;
  const now = useRuleNow();
  const set = (id: string, patch: Partial<StatusRule>) => change({ statusRules: rules.map((r) => (r.id === id ? { ...r, ...patch } : r)) });
  const move = (at: number, by: number) => {
    const next = [...rules];
    const [rule] = next.splice(at, 1);
    next.splice(at + by, 0, rule);
    change({ statusRules: next });
  };
  const fitAt = rules.findIndex((r) => r.id === now.fit);
  const [logOpen, setLogOpen] = useState(false);
  const [copied, setCopied] = useState(false);
  const log = ruleLog();
  const stamp = (at: number) => clock(new Date(at).toISOString());

  return (
    <section data-group="general">
      <h2>Status by rules</h2>
      <Switch
        label="Change my status by rules"
        hint="The first rule that fits decides. When no rule fits any more, the status you had before comes back. A status you set yourself in the meantime, here or in VRChat, stays until no rule fits any more."
        on={settings.statusRulesOn}
        set={(on) => change({ statusRulesOn: on })}
      />
      <p className="rule-fine">
        VRChat can take a minute to show a new status. The game's own menu often shows it only after your next world
        change, and the website catches up a little later. The list at the bottom says what the app set and when.
      </p>

      {rules.length === 0 ? (
        <div className="rule-empty">
          <p>No rules yet. A first one could set Do not disturb while you are in an Invite instance, so nobody sends requests into a private hangout.</p>
          <button className="plain go" onClick={() => change({ statusRules: [newRule()], statusRulesOn: true })}>
            Add that rule
          </button>
        </div>
      ) : (
        <>
          <ol className={settings.statusRulesOn ? "rules" : "rules rules-off"}>
            {rules.map((r, at) => (
              <li key={r.id} className={`rule${r.on ? "" : " rule-idle"}${settings.statusRulesOn && now.fit === r.id ? " rule-fits" : ""}`}>
                <span className="rule-place" aria-hidden="true">
                  {at + 1}
                </span>
                <div className="rule-lines">
                  <p className="rule-line">
                    <span className="rule-word">When</span>
                    <select aria-label={`Rule ${at + 1}: when`} value={r.when} onChange={(e) => set(r.id, { when: e.target.value as StatusRule["when"] })}>
                      {whens.map(([value, words]) => (
                        <option key={value} value={value}>
                          {words}
                        </option>
                      ))}
                    </select>
                    {r.when === "access" && (
                      <select aria-label={`Rule ${at + 1}: kind of instance`} value={r.access} onChange={(e) => set(r.id, { access: e.target.value as StatusRule["access"] })}>
                        {kinds.map(([value, words]) => (
                          <option key={value} value={value}>
                            {words}
                          </option>
                        ))}
                      </select>
                    )}
                    {r.when === "time" && (
                      <span className="rule-span">
                        <input type="time" aria-label={`Rule ${at + 1}: from`} value={r.from} onChange={(e) => e.target.value && set(r.id, { from: e.target.value })} />
                        <span className="rule-word">and</span>
                        <input type="time" aria-label={`Rule ${at + 1}: to`} value={r.to} onChange={(e) => e.target.value && set(r.id, { to: e.target.value })} />
                      </span>
                    )}
                    {r.when === "world" && <input className="rule-words" aria-label={`Rule ${at + 1}: part of the world's name`} value={r.words} maxLength={60} placeholder="club" onChange={(e) => set(r.id, { words: e.target.value })} />}
                    {r.when === "friends" && (
                      <input
                        className="rule-count"
                        type="number"
                        min={1}
                        max={80}
                        aria-label={`Rule ${at + 1}: number of friends`}
                        value={r.count}
                        onChange={(e) => {
                          const n = Math.round(Number(e.target.value));
                          if (n >= 1 && n <= 80) set(r.id, { count: n });
                        }}
                      />
                    )}
                  </p>
                  <p className="rule-line">
                    <span className="rule-word">set my status to</span>
                    <span className="rule-status">
                      <i className={`dot dot-${dots[r.status]}`} />
                      <select aria-label={`Rule ${at + 1}: status`} value={r.status} onChange={(e) => set(r.id, { status: e.target.value as StatusValue })}>
                        {(Object.keys(statusNames) as StatusValue[]).map((value) => (
                          <option key={value} value={value}>
                            {statusNames[value]}
                          </option>
                        ))}
                      </select>
                    </span>
                    <span className="rule-word">with the text</span>
                    <input className="rule-text" aria-label={`Rule ${at + 1}: status text`} value={r.text} maxLength={32} placeholder="Keep my own text" onChange={(e) => set(r.id, { text: e.target.value })} />
                  </p>
                  {r.when === "time" && r.from === r.to && <p className="rule-warn">From and to are the same time, so this rule never fits.</p>}
                  {r.when === "world" && r.words.trim() === "" && <p className="rule-warn">Type a part of a world's name, or this rule never fits.</p>}
                </div>
                <div className="rule-tools">
                  <label className="rule-on">
                    <span>{r.on ? "On" : "Off"}</span>
                    <input type="checkbox" role="switch" aria-label={`Rule ${at + 1} on`} checked={r.on} onChange={(e) => set(r.id, { on: e.target.checked })} />
                  </label>
                  <span className="rule-moves">
                    <button className="plain" disabled={at === 0} onClick={() => move(at, -1)} title="Earlier rules win over later ones">
                      Up
                    </button>
                    <button className="plain" disabled={at === rules.length - 1} onClick={() => move(at, 1)}>
                      Down
                    </button>
                    <button className="plain" onClick={() => change({ statusRules: rules.filter((x) => x.id !== r.id) })}>
                      Remove
                    </button>
                  </span>
                </div>
              </li>
            ))}
          </ol>
          <div className="rule-foot">
            <p role="status">
              {!settings.statusRulesOn
                ? "The rules are switched off, so your status stays as you set it."
                : fitAt < 0
                  ? "Right now no rule fits."
                  : now.paused
                    ? `Rule ${fitAt + 1} fits, but you set your status yourself, so it is left alone until no rule fits any more.`
                    : `Right now rule ${fitAt + 1} fits.`}
              {settings.statusRulesOn && now.scene && <span className="rule-scene">The app sees you in {now.scene}.</span>}
            </p>
            <button className="plain" disabled={rules.length >= mostRules} onClick={() => change({ statusRules: [...rules, newRule()] })}>
              Add a rule
            </button>
          </div>
          <div className="rule-log">
            <button className="fold" aria-expanded={logOpen} onClick={() => setLogOpen(!logOpen)}>
              {logOpen ? "Hide what the rules did" : `What the rules did (${log.length})`}
            </button>
            {logOpen &&
              (log.length === 0 ? (
                <p className="rule-log-none">Nothing yet. Each time a rule starts to fit, changes your status or steps back, a line lands here.</p>
              ) : (
                <>
                  <ol>
                    {log.map((n, i) => (
                      <li key={`${n.at}-${i}`}>
                        <time>{stamp(n.at)}</time>
                        <span>{n.text}</span>
                      </li>
                    ))}
                  </ol>
                  <span className="rule-log-tools">
                    <button
                      className="plain"
                      onClick={() =>
                        void navigator.clipboard.writeText(log.map((n) => `${new Date(n.at).toLocaleTimeString()}  ${n.text}`).join("\n")).then(
                          () => setCopied(true),
                          () => setCopied(false),
                        )
                      }
                    >
                      {copied ? "Copied" : "Copy"}
                    </button>
                    <button className="plain" onClick={() => (setCopied(false), clearRuleLog())}>
                      Clear
                    </button>
                  </span>
                </>
              ))}
          </div>
        </>
      )}
    </section>
  );
}
