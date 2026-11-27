import { useState } from "react";
import { backend, errorText, type ExportInfo } from "./backend";

/** The parts of the history that can go into a spreadsheet, with what each file holds. */
const parts: Array<[string, string, string]> = [
  ["feed", "Activity feed", "Who came online, changed world, status or avatar"],
  ["friends", "Friend log", "Friends added, removed and renamed"],
  ["visits", "World visits", "Every instance you were in, and for how long"],
  ["people", "People in worlds", "Who was in each instance with you"],
  ["avatars", "Avatars worn", "Every avatar change, with how long you wore it"],
  ["videos", "Videos played", "What the worlds played while you were there"],
];

/** Writes the history out as CSV files, one per part, into a new folder in Documents. */
export function ExportHistory() {
  const [picked, setPicked] = useState<string[]>(parts.map(([id]) => id));
  const [busy, setBusy] = useState(false);
  const [made, setMade] = useState<ExportInfo | null>(null);
  const [problem, setProblem] = useState("");

  async function run() {
    setBusy(true);
    setMade(null);
    setProblem("");
    try {
      setMade(await backend.exportHistory(picked));
    } catch (err) {
      setProblem(`The export failed: ${errorText(err)}`);
    } finally {
      setBusy(false);
    }
  }

  const rows = made?.files.reduce((n, f) => n + f.rows, 0) ?? 0;
  return (
    <div className="export">
      <p className="setting-hint">
        CSV files that Excel, LibreOffice or Google Sheets open, one for each part you pick, in a new folder in Documents, Anonymous VRX exports. They hold real names and ids, so keep them as private as a backup. Hide names
        only covers the screen.
      </p>
      <div className="export-parts" role="group" aria-label="What to export">
        {parts.map(([id, label, hint]) => {
          const on = picked.includes(id);
          return (
            <button
              key={id}
              className={on ? "on" : ""}
              aria-pressed={on}
              disabled={busy}
              onClick={() => setPicked(on ? picked.filter((p) => p !== id) : [...picked, id])}
            >
              <i aria-hidden="true" />
              <span>
                <b>{label}</b>
                <small>{hint}</small>
              </span>
            </button>
          );
        })}
      </div>
      <div className="data-actions">
        <button className="plain go" disabled={busy || picked.length === 0} onClick={() => void run()}>
          {busy ? "Writing the files" : picked.length === parts.length ? "Export everything" : `Export ${picked.length} of ${parts.length}`}
        </button>
        <button className="plain" onClick={() => void backend.openExportsFolder().catch((err) => setProblem(errorText(err)))}>
          Open the exports folder
        </button>
      </div>
      {made && (
        <div className="export-done" role="status">
          <p>
            Saved {made.files.length} {made.files.length === 1 ? "file" : "files"} with {rows.toLocaleString()} {rows === 1 ? "row" : "rows"} in "{made.name}".{" "}
            <button className="linkish" onClick={() => void backend.openExportsFolder(made.path).catch((err) => setProblem(errorText(err)))}>
              Show it
            </button>
          </p>
          <ul>
            {made.files.map((f) => (
              <li key={f.name}>
                <span>{f.name}</span>
                <span>{f.rows.toLocaleString()}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
      {problem && (
        <p className="banner" role="alert">
          {problem}
        </p>
      )}
    </div>
  );
}
