import { useEffect, useRef, type MouseEvent } from "react";

/** The Escape of every open window, panel and menu, the newest last. */
const open: Array<{ close: () => void }> = [];
let listening = false;

function onKey(e: KeyboardEvent) {
  if (e.key !== "Escape" || open.length === 0) return;
  // A search box takes Escape for itself: the first press empties it, and leaving an empty
  // box never closes the window around it.
  const box = e.target;
  if (box instanceof HTMLInputElement && box.type === "search") return;
  // Only the newest one closes, and nothing behind it hears the key.
  e.stopImmediatePropagation();
  // Holding the key down steps back once, not through every window.
  if (e.repeat) return;
  open[open.length - 1].close();
}

/**
 * Escape closes whatever called this, but only while it is the newest thing open: a panel over
 * a window closes first, then the window on the next press. `on` false leaves it out.
 */
export function useEscape(close: () => void, on = true) {
  const latest = useRef(close);
  latest.current = close;
  useEffect(() => {
    if (!on) return;
    const entry = { close: () => latest.current() };
    open.push(entry);
    if (!listening) {
      window.addEventListener("keydown", onKey, true);
      listening = true;
    }
    return () => {
      const at = open.indexOf(entry);
      if (at >= 0) open.splice(at, 1);
    };
  }, [on]);
}

/**
 * Props for a window's dark backdrop: it closes the window only on a press that both starts and
 * ends on the backdrop, so selecting text that runs past the window's edge closes nothing.
 */
export function useVeil(close: () => void) {
  const down = useRef(false);
  const up = useRef(false);
  const latest = useRef(close);
  latest.current = close;
  return {
    onMouseDown: (e: MouseEvent) => {
      down.current = e.target === e.currentTarget;
    },
    onMouseUp: (e: MouseEvent) => {
      up.current = e.target === e.currentTarget;
    },
    onClick: (e: MouseEvent) => {
      if (down.current && up.current && e.target === e.currentTarget) latest.current();
    },
  };
}
