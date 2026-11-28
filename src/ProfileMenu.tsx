import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import { useEscape } from "./escape";
import { backend, errorText } from "./backend";
import { tidy } from "./bits";

type Json = Record<string, unknown>;
type Kind = "block" | "mute" | "muteChat" | "hideAvatar" | "showAvatar" | "interactOff";

/** How wide the panel is, and how much room it wants below its button before it opens upward. */
const WIDE = 300;
const TALL = 560;

/**
 * The rest of what I can do about one person, behind "More" at the end of their profile's action
 * row: decide what I see and hear of them, bring them into a group, copy their details. Inviting,
 * asking and booping stay on the row itself. The panel sits on top of the whole window, so no
 * card or column cuts it off.
 */
export function ProfileMenu({ userId, name, friend, onDone }: { userId: string; name: string; friend: boolean; onDone: (message: string) => void }) {
  const [open, setOpen] = useState(false);
  const [place, setPlace] = useState<CSSProperties>({});
  const [view, setView] = useState<"main" | "groups">("main");
  const [kinds, setKinds] = useState<Kind[] | null>(null);
  const [kindsFailed, setKindsFailed] = useState(false);
  const [busy, setBusy] = useState("");
  const [sure, setSure] = useState(false);
  const [groups, setGroups] = useState<Json[] | null>(null);
  const [groupsFailed, setGroupsFailed] = useState("");
  const [problem, setProblem] = useState("");
  const [focusTo, setFocusTo] = useState("");
  const button = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const armedAt = useRef(0);
  const asked = useRef(0);
  const lastPlace = useRef("");
  const id = `pmenu-${userId}`;

  function askKinds() {
    // Only the newest question counts: a slow answer to an older one is dropped.
    const ask = ++asked.current;
    setKinds(null);
    setKindsFailed(false);
    void backend
      .moderationsFor(userId)
      .then((k) => ask === asked.current && setKinds(k as Kind[]))
      .catch(() => ask === asked.current && setKindsFailed(true));
  }

  // When the button that had the keyboard goes away (Block armed, Keep, Yes), the keyboard moves to
  // the control that took its place, once nothing is locked.
  useEffect(() => {
    if (!focusTo || busy) return;
    panel.current?.querySelector<HTMLElement>(focusTo)?.focus();
    setFocusTo("");
  }, [focusTo, busy, sure, kinds]);

  // Where the panel goes: under its button, or above it when the window has no room below.
  // The window can be enlarged in Settings, so places are divided by that size.
  function measure() {
    const b = button.current;
    if (!b) return;
    const size = parseFloat(document.documentElement.style.zoom) || 1;
    const r = b.getBoundingClientRect();
    const w = window.innerWidth / size;
    const h = window.innerHeight / size;
    const width = Math.min(WIDE, w - 24);
    const left = Math.max(12, Math.min(r.right / size - width, w - width - 12));
    const below = h - r.bottom / size - 12;
    const above = r.top / size - 12;
    const next: CSSProperties =
      below >= Math.min(TALL, h * 0.6) || below >= above
        ? { left, top: r.bottom / size + 8, width, maxHeight: Math.max(220, below - 8) }
        : { left, bottom: h - r.top / size + 8, width, maxHeight: Math.max(220, above - 8) };
    // Only a real move changes the place, so measuring after every render cannot loop.
    const key = JSON.stringify(next);
    if (key === lastPlace.current) return;
    lastPlace.current = key;
    setPlace(next);
  }

  // Measured after every render while open: a line that appears above the button (Block adds one)
  // moves the button, and the panel moves with it.
  useLayoutEffect(() => {
    if (open) measure();
    else lastPlace.current = "";
  });

  useEffect(() => {
    if (!open) return;
    setView("main");
    setSure(false);
    setProblem("");
    askKinds();
    // Focus goes into the panel, so the keyboard can reach it at once.
    requestAnimationFrame(() => panel.current?.querySelector<HTMLElement>("button:not(:disabled), input:not(:disabled)")?.focus());
    const away = (e: PointerEvent) => {
      const t = e.target as Node;
      if (!panel.current?.contains(t) && !button.current?.contains(t)) setOpen(false);
    };
    const move = () => measure();
    // The button sits in a column that scrolls, so the panel follows it. Scrolling inside the
    // panel itself moves nothing.
    const scrolled = (e: Event) => {
      if (!panel.current?.contains(e.target as Node)) measure();
    };
    // Tabbing past the last control, or anywhere else, closes the panel.
    const left = (e: FocusEvent) => {
      const t = e.target as Node;
      if (!panel.current?.contains(t) && !button.current?.contains(t)) setOpen(false);
    };
    window.addEventListener("pointerdown", away, true);
    window.addEventListener("resize", move);
    window.addEventListener("scroll", scrolled, true);
    window.addEventListener("focusin", left, true);
    return () => {
      // An answer that comes after the panel closed is not shown next time it opens.
      asked.current++;
      window.removeEventListener("pointerdown", away, true);
      window.removeEventListener("resize", move);
      window.removeEventListener("scroll", scrolled, true);
      window.removeEventListener("focusin", left, true);
    };
  }, [open, userId]); // eslint-disable-line react-hooks/exhaustive-deps

  // Escape closes the panel and nothing behind it, and the keyboard goes back to More.
  useEscape(() => {
    setOpen(false);
    button.current?.focus();
  }, open);

  // A Block that was armed and not confirmed goes back to normal after a few seconds.
  useEffect(() => {
    if (!sure) return;
    const t = window.setTimeout(() => {
      // The keyboard goes back to Block only if it was on the question, not if it moved on.
      const onQuestion = panel.current?.querySelector(".pmenu-confirm")?.contains(document.activeElement);
      setSure(false);
      if (onQuestion) setFocusTo(".pmenu-blockbtn");
    }, 4000);
    return () => window.clearTimeout(t);
  }, [sure]);

  const has = (k: Kind) => kinds?.includes(k) ?? false;
  const locked = busy !== "";

  async function run(label: string, job: () => Promise<unknown>, done: string, close = true) {
    setBusy(label);
    setProblem("");
    try {
      await job();
      onDone(done);
      if (close) setOpen(false);
    } catch (err) {
      setProblem(errorText(err));
    } finally {
      setBusy("");
    }
  }

  async function toggle(kind: Kind, on: boolean, done: string) {
    await run(
      kind,
      async () => {
        await backend.setModeration(userId, kind, on);
        setKinds((old) => (on ? [...(old ?? []).filter((k) => k !== kind), kind] : (old ?? []).filter((k) => k !== kind)));
        if (kind === "block" || kind === "mute") window.dispatchEvent(new CustomEvent("moderation:changed", { detail: { userId, kind, on } }));
        if (kind === "block") setFocusTo(".pmenu-blockbtn");
      },
      done,
      false,
    );
  }

  /** Their avatar for me: as my rank settings decide, always shown, or always hidden. */
  async function avatar(choice: "rank" | "show" | "hide") {
    await run(
      "avatar",
      async () => {
        try {
          if (has("hideAvatar") && choice !== "hide") await backend.setModeration(userId, "hideAvatar", false);
          if (has("showAvatar") && choice !== "show") await backend.setModeration(userId, "showAvatar", false);
          if (choice === "show" && !has("showAvatar")) await backend.setModeration(userId, "showAvatar", true);
          if (choice === "hide" && !has("hideAvatar")) await backend.setModeration(userId, "hideAvatar", true);
        } finally {
          // Whatever went through, the panel shows what VRChat now holds.
          const fresh = await backend.moderationsFor(userId).catch(() => null);
          if (fresh) setKinds(fresh as Kind[]);
        }
      },
      choice === "rank" ? `${name}'s avatar goes by your rank settings again.` : choice === "show" ? `You always see ${name}'s avatar now.` : `${name}'s avatar is hidden for you now.`,
      false,
    );
  }

  function openGroups() {
    setView("groups");
    setProblem("");
    requestAnimationFrame(() => panel.current?.querySelector<HTMLElement>(".pmenu-back")?.focus());
    if (groups && !groupsFailed) return;
    setGroups(null);
    setGroupsFailed("");
    void backend
      .myGroups()
      .then((list) => setGroups(list.filter((g) => g && typeof g === "object")))
      .catch((err) => {
        setGroups([]);
        setGroupsFailed(errorText(err));
      });
  }

  function copy(text: string, done: string) {
    void navigator.clipboard.writeText(text).then(
      () => onDone(done),
      () => onDone("Could not copy."),
    );
    setOpen(false);
  }

  const avatarChoice = has("hideAvatar") ? "hide" : has("showAvatar") ? "show" : "rank";

  const body =
    view === "main" ? (
      <>
        <section aria-labelledby={`${id}-see`}>
          <h4 id={`${id}-see`}>What you see and hear of them</h4>
          {kindsFailed ? (
            <div className="pmenu-failed">
              <p className="pmenu-note">VRChat did not say what you set for {friend ? "this friend" : "this person"}, so these choices are hidden for now.</p>
              <button className="pmenu-item" onClick={askKinds}>
                Ask again
              </button>
            </div>
          ) : kinds === null ? (
            <p className="pmenu-note">Asking VRChat</p>
          ) : (
            <>
              <div className="pmenu-seg" role="radiogroup" aria-label="Their avatar">
                <span>Their avatar</span>
                {(
                  [
                    ["rank", "By rank"],
                    ["show", "Show"],
                    ["hide", "Hide"],
                  ] as const
                ).map(([value, label]) => (
                  <button key={value} role="radio" aria-checked={avatarChoice === value} className={avatarChoice === value ? "on" : ""} disabled={locked} onClick={() => avatarChoice !== value && void avatar(value)}>
                    {label}
                  </button>
                ))}
              </div>
              <Check label="Mute their voice" on={has("mute")} busy={locked} set={(on) => void toggle("mute", on, on ? `${name} is muted for you.` : `${name} is no longer muted.`)} />
              <Check label="Mute their chatbox" on={has("muteChat")} busy={locked} set={(on) => void toggle("muteChat", on, on ? `${name}'s chatbox is hidden for you.` : `${name}'s chatbox shows again.`)} />
              <Check
                label="Stop their avatar touching yours"
                on={has("interactOff")}
                busy={locked}
                set={(on) => void toggle("interactOff", on, on ? `${name}'s avatar can no longer touch yours.` : `${name}'s avatar can touch yours again if your interaction settings allow it.`)}
              />
              {has("block") ? (
                <button className="pmenu-item pmenu-danger pmenu-blockbtn" disabled={locked} onClick={() => void toggle("block", false, `${name} is no longer blocked.`)}>
                  Unblock
                </button>
              ) : sure ? (
                <div className="pmenu-confirm" role="group" aria-label="Block this person?">
                  <span>Block {friend ? "this friend" : "this person"}? You stop seeing and hearing each other in VRChat.</span>
                  <button
                    className="pmenu-yes"
                    disabled={locked}
                    onClick={(e) => {
                      // A double click on Block must not confirm it on the second click.
                      if (e.detail > 1 || Date.now() - armedAt.current < 450) return;
                      setSure(false);
                      void toggle("block", true, `${name} is blocked. You no longer see or hear each other in VRChat.`);
                    }}
                  >
                    Yes, block
                  </button>
                  <button
                    className="pmenu-keep"
                    onClick={() => {
                      setSure(false);
                      setFocusTo(".pmenu-blockbtn");
                    }}
                  >
                    Keep
                  </button>
                </div>
              ) : (
                <button
                  className="pmenu-item pmenu-danger pmenu-blockbtn"
                  disabled={locked}
                  onClick={() => {
                    armedAt.current = Date.now();
                    setSure(true);
                    // The keyboard lands on Keep, the safe answer.
                    setFocusTo(".pmenu-keep");
                  }}
                >
                  Block
                </button>
              )}
            </>
          )}
        </section>

        <section aria-labelledby={`${id}-more`}>
          <h4 id={`${id}-more`}>Groups and links</h4>
          <button className="pmenu-item pmenu-groupsbtn" disabled={locked} onClick={openGroups}>
            Invite to one of my groups
            <span aria-hidden="true">›</span>
          </button>
          <button className="pmenu-item" onClick={() => copy(userId, "User ID copied.")}>
            Copy user ID
          </button>
          <button className="pmenu-item" onClick={() => copy(`https://vrchat.com/home/user/${userId}`, "Profile link copied.")}>
            Copy profile link
          </button>
        </section>
      </>
    ) : (
      <section className="pmenu-groups" aria-labelledby={`${id}-groups`}>
        <button
          className="pmenu-back"
          onClick={() => {
            setView("main");
            setFocusTo(".pmenu-groupsbtn");
          }}
        >
          ‹ Back
        </button>
        <h4 id={`${id}-groups`}>Invite to one of my groups</h4>
        {groups === null ? (
          <p className="pmenu-note">Asking VRChat for your groups</p>
        ) : groupsFailed ? (
          <div className="pmenu-failed">
            <p className="pmenu-note">Your groups could not be read: {groupsFailed}</p>
            <button className="pmenu-item" onClick={openGroups}>
              Ask again
            </button>
          </div>
        ) : groups.length === 0 ? (
          <p className="pmenu-note">You are in no groups.</p>
        ) : (
          <ul>
            {groups.map((g) => {
              const gid = String(g.groupId ?? g.id ?? "");
              const pic = String(g.iconUrl ?? "");
              const gname = tidy(String(g.name ?? "")) || "A group";
              return (
                <li key={gid}>
                  <button className="pmenu-item pmenu-groupitem" disabled={locked} onClick={() => void run(gid, () => backend.inviteToGroup(gid, userId), `Invited ${name} to ${gname}.`)}>
                    <span className="pmenu-pic" style={pic ? { backgroundImage: `url("${pic}")` } : undefined} />
                    <span className="pmenu-group hide-name">{gname}</span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
        <p className="pmenu-note">VRChat only lets you invite to groups where your role is allowed to.</p>
      </section>
    );

  return (
    <div className="pmenu-wrap">
      <button
        ref={button}
        className="pmenu-button"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        title="Mute, block, their avatar, invite to a group, copy their ID"
        onClick={() => setOpen(!open)}
      >
        More
        <span className="pmenu-chev" aria-hidden="true">▾</span>
      </button>
      {open &&
        createPortal(
          <div ref={panel} id={id} className="pmenu" role="dialog" aria-label={`Actions for ${name}`} style={place}>
            {body}
            {problem && (
              <p className="pmenu-problem" role="alert">
                {problem}
              </p>
            )}
          </div>,
          document.body,
        )}
    </div>
  );
}

function Check({ label, on, busy, set }: { label: string; on: boolean; busy: boolean; set: (on: boolean) => void }) {
  return (
    <label className="pmenu-check">
      <span>{label}</span>
      <input type="checkbox" role="switch" checked={on} disabled={busy} onChange={(e) => set(e.target.checked)} />
    </label>
  );
}
