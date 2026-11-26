import { useEffect, useState } from "react";
import { errorText, type Me } from "./backend";
import { openProfile } from "./Profile";
import { saveStatusByHand, useRuleNow } from "./statusrules";

const choices: Array<[string, string, string]> = [
  ["join me", "Join me", "Anyone can join you"],
  ["active", "Online", "Friends see where you are"],
  ["ask me", "Ask me", "People ask before joining"],
  ["busy", "Do not disturb", "No invites or requests"],
];

/** Quick change of my own VRChat status, from the picture at the bottom of the rail. */
export function MyStatus({ me, onSaved, onClose }: { me: Me; onSaved: (me: Me, message: string) => void; onClose: () => void }) {
  const [status, setStatus] = useState(me.status || "active");
  const [text, setText] = useState(me.statusDescription);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState("");
  const rule = useRuleNow();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  async function save() {
    setBusy(true);
    setProblem("");
    try {
      await saveStatusByHand(me.id, { status, text: text.trim() });
      onSaved({ ...me, status, statusDescription: text.trim() }, "Status changed.");
    } catch (err) {
      setProblem(errorText(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="veil" onClick={onClose}>
      <div className="dialog" role="dialog" aria-modal="true" aria-labelledby="status-title" onClick={(e) => e.stopPropagation()}>
        <h2 id="status-title">My status</h2>
        <div className="status-choices" role="radiogroup" aria-label="Status">
          {choices.map(([value, label, hint]) => (
            <button key={value} role="radio" aria-checked={status === value} className={status === value ? "on" : ""} onClick={() => setStatus(value)}>
              <i className={`dot dot-${value.replace(" ", "")}`} />
              <span>
                <b>{label}</b>
                <small>{hint}</small>
              </span>
            </button>
          ))}
        </div>
        <label className="status-text">
          Status text
          <input value={text} maxLength={32} onChange={(e) => setText(e.target.value)} placeholder="What you are up to" />
          <small>{32 - text.length} left</small>
        </label>
        {rule.fit && rule.holding && <p className="status-rule-note">A status rule set this. What you save here stays until no rule fits any more.</p>}
        {problem && (
          <p className="banner" role="alert">
            {problem}
          </p>
        )}
        <div className="dialog-actions">
          <button
            onClick={() => {
              onClose();
              openProfile(me.id);
            }}
          >
            My profile
          </button>
          <button onClick={onClose}>Cancel</button>
          <button className="go" disabled={busy} onClick={() => void save()}>
            {busy ? "Saving" : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
}
