import { useEffect, useState } from "react";
import { backend, errorText } from "./backend";
import { SafeImg } from "./Pic";

/** Any part of the window can show a picture full size this way. */
export function openPicture(url: string, name: string) {
  if (url) window.dispatchEvent(new CustomEvent("open-picture", { detail: { url, name } }));
}

export function PictureViewer({ onDone }: { onDone: (message: string) => void }) {
  const [shown, setShown] = useState<{ url: string; name: string } | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const open = (e: Event) => setShown((e as CustomEvent).detail);
    window.addEventListener("open-picture", open);
    return () => window.removeEventListener("open-picture", open);
  }, []);

  useEffect(() => {
    if (!shown) return;
    // Escape closes the picture first, and only the picture.
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopImmediatePropagation();
      setShown(null);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [shown]);

  if (!shown) return null;

  async function save() {
    if (!shown) return;
    setBusy(true);
    try {
      const path = await backend.savePicture(shown.url, shown.name);
      onDone(`Saved to ${path}`);
    } catch (err) {
      onDone(`The picture was not saved: ${errorText(err)}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="viewer" role="dialog" aria-modal="true" aria-label={shown.name} onClick={() => setShown(null)}>
      <SafeImg src={shown.url} alt={shown.name} onClick={(e) => e.stopPropagation()} />
      <div className="viewer-bar" onClick={(e) => e.stopPropagation()}>
        <span className="viewer-name">{shown.name}</span>
        <button className="go" disabled={busy} onClick={() => void save()}>
          {busy ? "Saving" : "Save picture"}
        </button>
        <button onClick={() => void backend.openPicturesFolder().catch((err) => onDone(errorText(err)))}>Open saved pictures</button>
        <button onClick={() => setShown(null)}>Close</button>
      </div>
    </div>
  );
}
