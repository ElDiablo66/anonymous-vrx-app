import { useEffect, useRef, useState, type CSSProperties, type KeyboardEvent as KeyEvent } from "react";
import { errorText, type Me } from "./backend";
import { Avatar } from "./bits";
import { openProfile } from "./Profile";
import { useEscape, useVeil } from "./escape";
import { saveStatusByHand, useRuleNow } from "./statusrules";

/** VRChat's four statuses: its own word, the name shown, what it means, and its colour. */
const choices: Array<[string, string, string, string]> = [
  ["join me", "Join me", "Anyone can join you", "var(--join)"],
  ["active", "Online", "Friends see where you are", "var(--online)"],
  ["ask me", "Ask me", "People ask before joining", "var(--ask)"],
  ["busy", "Do not disturb", "No invites or requests", "var(--busy)"],
];

const RECENT = "status-texts";

function recentTexts(): string[] {
  try {
    const list = JSON.parse(localStorage.getItem(RECENT) ?? "[]") as unknown;
    return Array.isArray(list) ? list.filter((t): t is string => typeof t === "string" && t.trim() !== "").slice(0, 5) : [];
  } catch {
    return [];
  }
}

function remember(text: string) {
  if (!text) return;
  try {
    localStorage.setItem(RECENT, JSON.stringify([text, ...recentTexts().filter((t) => t !== text)].slice(0, 5)));
  } catch {
    // Then the texts used before are not offered next time.
  }
}

/** Quick change of my own VRChat status, from the picture at the bottom of the rail. */
export function MyStatus({ me, onSaved, onClose, onEdit }: { me: Me; onSaved: (me: Me, message: string) => void; onClose: () => void; onEdit: () => void }) {
  const [status, setStatus] = useState(me.status || "active");
  const [text, setText] = useState(me.statusDescription);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState("");
  // Until I pick or type something, the window follows my status as it changes (a rule, the game).
  const [touched, setTouched] = useState(false);
  const rule = useRuleNow();
  const field = useRef<HTMLInputElement>(null);
  const dialog = useRef<HTMLDivElement>(null);
  const veil = useVeil(onClose);
  const recent = recentTexts().filter((t) => t !== text.trim());
  const now = choices.find(([v]) => v === (me.status || "active"));
  // VRChat can report a status that is none of the four, such as offline; it is named as it is.
  const nowLabel = now?.[1] ?? (me.status === "offline" ? "Offline or invisible" : me.status);
  const picked = choices.find(([v]) => v === status);
  const changed = status !== (me.status || "active") || text.trim() !== me.statusDescription;
  // VRChat only takes one of the four, so a status such as offline has to be swapped for one before saving.
  const canSave = changed && picked !== undefined;

  useEscape(onClose);

  useEffect(() => {
    if (touched) return;
    setStatus(me.status || "active");
    setText(me.statusDescription);
  }, [me.status, me.statusDescription, touched]);

  // The keyboard starts on the status that is picked, and goes back where it was when this closes.
  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    dialog.current?.querySelector<HTMLElement>('.my-status-grid [aria-checked="true"], .my-status-grid button')?.focus();
    return () => opener?.focus?.();
  }, []);

  // Tab goes round inside this window, never to the app behind it.
  function keepTabInside(e: KeyEvent<HTMLDivElement>) {
    if (e.key !== "Tab") return;
    const all = [...e.currentTarget.querySelectorAll<HTMLElement>("button:not(:disabled), input:not(:disabled)")];
    if (all.length === 0) return;
    const first = all[0];
    const last = all[all.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }

  async function save() {
    setBusy(true);
    setProblem("");
    try {
      await saveStatusByHand(me.id, { status, text: text.trim() });
      remember(text.trim());
      onSaved({ ...me, status, statusDescription: text.trim() }, "Status changed.");
    } catch (err) {
      setProblem(errorText(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="veil" {...veil}>
      <div
        ref={dialog}
        className="dialog my-status"
        role="dialog"
        aria-modal="true"
        aria-labelledby="status-title"
        style={{ "--c": picked?.[3] ?? "var(--offline)" } as CSSProperties}
        onKeyDown={keepTabInside}
      >
        <header className="my-status-head">
          <Avatar name={me.displayName} image={me.image} size={52} />
          <div>
            <h2 id="status-title">My status</h2>
            <p className="my-status-now">
              Now <b style={{ color: now?.[3] ?? "var(--offline)" }}>{nowLabel}</b>
              {me.statusDescription && (
                <>
                  , <span className="hide-name">"{me.statusDescription}"</span>
                </>
              )}
            </p>
          </div>
        </header>

        <div className="my-status-grid" role="radiogroup" aria-label="Status">
          {choices.map(([value, label, hint, colour]) => (
            <button
              key={value}
              role="radio"
              aria-checked={status === value}
              className={status === value ? "on" : ""}
              style={{ "--c": colour } as CSSProperties}
              onClick={() => {
                setStatus(value);
                setTouched(true);
              }}
            >
              <i className="my-status-dot" aria-hidden="true" />
              <span>
                <b>{label}</b>
                <small>{hint}</small>
              </span>
            </button>
          ))}
        </div>

        <label className="my-status-text">
          <span>Status text</span>
          <span className="my-status-field">
            <input ref={field} value={text} maxLength={32} onChange={(e) => {
                setText(e.target.value);
                setTouched(true);
              }} onKeyDown={(e) => e.key === "Enter" && !busy && canSave && void save()} placeholder="What you are up to" />
            <small aria-live="polite">{32 - text.length}</small>
            {text && (
              <button type="button" className="my-status-clear" aria-label="Clear the status text" onClick={() => {
                  setText("");
                  setTouched(true);
                  field.current?.focus();
                }}>
                ×
              </button>
            )}
          </span>
        </label>
        {recent.length > 0 && (
          <div className="my-status-recent" aria-label="Used lately">
            <span>Used lately</span>
            {recent.map((t) => (
              <button
                key={t}
                className="hide-name"
                onClick={() => {
                  // The chip goes away once its text is in the box, so the keyboard moves to the box.
                  setText(t);
                  setTouched(true);
                  field.current?.focus();
                }}
              >
                {t}
              </button>
            ))}
          </div>
        )}

        {changed && !picked && <p className="status-rule-note">Pick a status first.</p>}
        {rule.fit && rule.holding && <p className="status-rule-note">A status rule set this. What you save here stays until no rule fits any more.</p>}
        {problem && (
          <p className="banner" role="alert">
            {problem}
          </p>
        )}

        <footer className="my-status-foot">
          <span className="my-status-links">
            <button
              className="linkish"
              onClick={() => {
                onClose();
                openProfile(me.id);
              }}
            >
              View my profile
            </button>
            <button className="linkish" onClick={onEdit} title="Change your bio, pronouns, links, languages and how your profile looks">
              Edit my profile
            </button>
          </span>
          <span className="my-status-buttons">
            <button onClick={onClose}>Cancel</button>
            <button className="go" disabled={busy || !canSave} onClick={() => void save()}>
              {busy ? "Saving" : "Save"}
            </button>
          </span>
        </footer>
      </div>
    </div>
  );
}
