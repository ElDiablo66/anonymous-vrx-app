import { useEffect, useState } from "react";
import { backend, errorText } from "./backend";
import { Choices, Row, Switch } from "./SettingsMore";
import type { Settings } from "./settings";
import { drawNotice } from "./vrpanels";

/** The app's own panels in SteamVR: a notice in front of the eyes, a feed on the arm, a dashboard tab and SteamVR's own alerts. */
export function VrPanelSettings({ settings, change }: { settings: Settings; change: (patch: Partial<Settings>) => void }) {
  const [status, setStatus] = useState("");
  const [result, setResult] = useState("");
  const [testing, setTesting] = useState(false);
  const panels = settings.vrOwn || settings.vrWrist !== "off" || settings.vrDashboard || settings.vrDevices;
  const others = settings.vrXs || settings.vrOvrHud || settings.vrOvrWrist;

  // Where the panels stand, asked again every few seconds while this page is open.
  useEffect(() => {
    if (!panels) {
      setStatus("");
      return;
    }
    const ask = () => void backend.vrOverlayStatus().then(setStatus).catch(() => {});
    ask();
    const again = window.setInterval(ask, 3000);
    return () => window.clearInterval(again);
  }, [panels]);

  async function test() {
    setTesting(true);
    setResult("");
    try {
      const state = await backend.vrOverlayStatus();
      if (!state.startsWith("Connected")) {
        setResult(state);
        return;
      }
      const png = await drawNotice("steamvr", "Test", "Anonymous VRX", "This is what a notice looks like in the headset.");
      await backend.vrOverlayNotice(png, settings.vrSeconds, settings.vrOpacity, settings.vrNoticeSpot === "above");
      setResult("Sent. Look in the headset.");
    } catch (err) {
      setResult(errorText(err));
    } finally {
      setTesting(false);
    }
  }

  async function tryNative() {
    setTesting(true);
    setResult("");
    try {
      setResult(await backend.vrOverlayTryNative());
    } catch (err) {
      setResult(errorText(err));
    } finally {
      setTesting(false);
    }
  }

  return (
    <section data-group="vr">
      <h2>
        This app in SteamVR <span className="mark">Beta</span>
      </h2>
      <p className="setting-hint">
        Panels drawn by this app itself, with no other program needed. They are new and have had little time in a headset, so a panel may need a
        nudge before it sits right.
      </p>

      <Switch label="Notices in front of you" hint="Shown on this app's own panel." on={settings.vrOwn} set={(vrOwn) => change({ vrOwn })} />
      {settings.vrOwn && (
        <Row label="Where the notice floats" hint="Over the middle keeps it clear of XSOverlay's notices, which sit just under the middle.">
          <Choices
            label="Where the notice floats"
            value={settings.vrNoticeSpot}
            set={(vrNoticeSpot) => change({ vrNoticeSpot })}
            options={[
              ["above", "Over the middle"],
              ["low", "Under the middle"],
            ]}
          />
        </Row>
      )}
      {(settings.vrOwn || settings.vrDevices) && others && (
        <Row
          label="Which one shows a notice"
          hint={
            settings.vrXs && (settings.vrOvrHud || settings.vrOvrWrist)
              ? "Each notice shows in one place only: the first one here that is running. XSOverlay comes before OVR Toolkit."
              : "Each notice shows in one place only. The second one gets it only when the first is not running."
          }
        >
          <Choices
            label="Which one shows a notice"
            value={settings.vrRoute}
            set={(vrRoute) => change({ vrRoute })}
            options={[
              ["xs", settings.vrXs && !settings.vrOvrHud && !settings.vrOvrWrist ? "XSOverlay first" : settings.vrXs ? "XSOverlay or OVR Toolkit first" : "OVR Toolkit first"],
              ["own", "This app's panel first"],
            ]}
          />
        </Row>
      )}

      <Row label="Feed on the arm" hint="The clock, how many friends are in VRChat, the six newest feed entries and the batteries. The controller in that hand has to be on.">
        <Choices
          label="Feed on the arm"
          value={settings.vrWrist}
          set={(vrWrist) => change({ vrWrist })}
          options={[
            ["off", "Off"],
            ["left", "Left hand"],
            ["right", "Right hand"],
          ]}
        />
      </Row>
      {settings.vrWrist !== "off" && (
        <>
          <Row
            label="Where on the arm"
            hint="It lies sideways on the arm and turns with the wrist, like XSOverlay's wrist panel. When XSOverlay's is on the same hand, the feed lines up with it: up the arm sits just above it, so both stay in view, and the back of the wrist takes its place and covers it."
          >
            <Choices
              label="Where on the arm"
              value={settings.vrWristSpot}
              set={(vrWristSpot) => change({ vrWristSpot })}
              options={[
                ["forearm", "Up the arm"],
                ["wrist", "Back of the wrist"],
                ["inside", "Inside of the wrist"],
              ]}
            />
          </Row>
          <Row label="Move it along the arm" hint="More moves it towards the elbow, less back towards the hand.">
            <label className="dim">
              <input
                type="range"
                min={-6}
                max={12}
                step={1}
                value={settings.vrWristShift}
                aria-label="Centimetres along the arm"
                onChange={(e) => change({ vrWristShift: Number(e.target.value) })}
              />
              {settings.vrWristShift > 0 ? "+" : ""}
              {settings.vrWristShift} cm
            </label>
          </Row>
          <Row label="Size of the feed" hint="How wide the panel is.">
            <div className="choices" role="radiogroup" aria-label="Size of the feed on the arm">
              {(
                [
                  [0.14, "Small"],
                  [0.18, "Medium"],
                  [0.24, "Large"],
                ] as Array<[number, string]>
              ).map(([w, words]) => (
                <button key={w} role="radio" aria-checked={settings.vrWristSize === w} className={settings.vrWristSize === w ? "on" : ""} onClick={() => change({ vrWristSize: w })}>
                  {words}
                </button>
              ))}
            </div>
          </Row>
        </>
      )}

      <Switch
        label="Tab in SteamVR's dashboard"
        hint="More of the feed and every battery, in SteamVR's own menu, and these settings too, so they can be changed with the pointer. It lives inside that menu, so it never covers XSOverlay."
        on={settings.vrDashboard}
        set={(vrDashboard) => change({ vrDashboard })}
      />
      <Switch
        label="Battery, tracking and SteamVR alerts"
        hint="A notice when a controller or tracker drops to 20% and again at 10%, loses tracking for 3 seconds or switches off. Several losing tracking at once make one notice, and nothing is said while the headset itself has lost tracking. It shows on this app's panel, or on XSOverlay or OVR Toolkit when those come first. SteamVR closing goes to Windows. Read from SteamVR on this PC; nothing goes to VRChat."
        on={settings.vrDevices}
        set={(vrDevices) => change({ vrDevices })}
      />

      {panels && (
        <>
          <Row label="Stays up for" hint="How long a notice shows.">
            <div className="choices" role="radiogroup" aria-label="Seconds a notice stays up in SteamVR">
              {[2, 4, 6, 10].map((n) => (
                <button key={n} role="radio" aria-checked={settings.vrSeconds === n} className={settings.vrSeconds === n ? "on" : ""} onClick={() => change({ vrSeconds: n })}>
                  {n} s
                </button>
              ))}
            </div>
          </Row>
          <Row label="How solid the panels are" hint="Lower lets the world show through.">
            <label className="dim">
              <input type="range" min={0.2} max={1} step={0.05} value={settings.vrOpacity} aria-label="How solid the panels are" onChange={(e) => change({ vrOpacity: Number(e.target.value) })} />
              {Math.round(settings.vrOpacity * 100)}%
            </label>
          </Row>
        </>
      )}
      <div className="data-actions">
        <button className="plain" disabled={!(settings.vrOwn || settings.vrDevices) || testing} onClick={() => void test()}>
          {testing ? "Sending" : "Send a test notice"}
        </button>
        <button
          className="plain"
          disabled={!panels || testing}
          onClick={() => void tryNative()}
          title="Sends one of SteamVR's own notifications, to see whether this SteamVR still shows them"
        >
          Try SteamVR's own notice <span className="mark">Beta</span>
        </button>
      </div>
      {result && (
        <p className="setting-hint" role="status">
          {result}
        </p>
      )}
      {panels && status && (
        <p className="setting-hint" role="status">
          Right now: {status}
        </p>
      )}
    </section>
  );
}
