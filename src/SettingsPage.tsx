import type { Favorites, Me } from "./backend";
import { useEffect, useRef, useState } from "react";
import { backend, errorText, type BackupInfo } from "./backend";
import { StatusRuleSettings } from "./RuleSettings";
import { Avatar } from "./bits";
import { DEFAULT_AVATAR_PROVIDER, EXTRA_AVATAR_PROVIDERS, shrinkPicture, sizes, themeColours, type Settings } from "./settings";
import { DateSettings, FontSettings, UpdateSettings, GameCacheSettings, InstanceSettings, PeopleColourSettings, PerformanceSettings, ProfileLookSettings, VrNoticeSettings } from "./SettingsMore";

function Swatch({ label, value, set }: { label: string; value: string; set: (value: string) => void }) {
  // Dragging in the colour picker reports a new colour dozens of times a second. The swatch
  // follows at once, but the whole app is only recoloured about eight times a second, which
  // keeps the picker smooth.
  const [picked, setPicked] = useState(value);
  const waiting = useRef<number | undefined>(undefined);
  const latest = useRef(value);
  useEffect(() => {
    if (waiting.current === undefined) setPicked(value);
  }, [value]);
  useEffect(() => () => window.clearTimeout(waiting.current), []);
  return (
    <label className="swatch">
      <input
        type="color"
        value={picked}
        onChange={(e) => {
          latest.current = e.target.value;
          setPicked(e.target.value);
          if (waiting.current !== undefined) return;
          waiting.current = window.setTimeout(() => {
            waiting.current = undefined;
            set(latest.current);
          }, 120);
        }}
      />
      {label}
    </label>
  );
}

/** The pages of Settings. Each section belongs to one, by its `data-group`. */
const settingsTabs = [
  ["general", "General"],
  ["look", "Look and privacy"],
  ["alerts", "Notifications"],
  ["feed", "Feed and Discord"],
  ["vr", "VR and VRChat"],
  ["data", "Data and account"],
  ["about", "About"],
] as const;

/** The page that was open last, kept while the app runs. */
let lastTab: (typeof settingsTabs)[number][0] = "general";

function Switch({ label, hint, on, set }: { label: string; hint?: string; on: boolean; set: (on: boolean) => void }) {
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

export function SettingsPage({
  settings,
  change,
  fitted,
  picture,
  setPicture,
  me,
  favorites,
  onSignOut,
}: {
  favorites: Favorites;
  settings: Settings;
  change: (patch: Partial<Settings>) => void;
  fitted: number;
  picture: string;
  setPicture: (data: string) => boolean;
  me: Me;
  onSignOut: () => void;
}) {
  const [testResult, setTestResult] = useState("");
  const [pictureNote, setPictureNote] = useState("");
  const base = themeColours[settings.background];

  async function pickPicture(file: File | undefined) {
    if (!file) return;
    setPictureNote("");
    try {
      // Try smaller and smaller copies until one fits in the window's storage.
      for (const longest of [1920, 1440, 1080]) {
        if (setPicture(await shrinkPicture(file, longest))) return;
      }
      setPictureNote("That picture is too large to keep, even shrunk. Try a smaller one.");
    } catch (err) {
      setPictureNote(errorText(err));
    }
  }
  const [backups, setBackups] = useState<BackupInfo[]>([]);
  const [dataNote, setDataNote] = useState("");
  const [asking, setAsking] = useState("");
  const [restored, setRestored] = useState(false);
  // Moving over from VRCX: whether its history is on this PC, and how the last run went.
  const [vrcx, setVrcx] = useState(false);
  const [bringing, setBringing] = useState(false);
  const [brought, setBrought] = useState("");
  useEffect(() => {
    void backend
      .vrcxFound()
      .then(setVrcx)
      .catch(() => {});
  }, []);
  async function bringOver() {
    setBringing(true);
    setBrought("");
    try {
      const { broughtOver: got, backup } = await backend.importVrcx(JSON.stringify(settings));
      const total = got.feed + got.friendLog + got.visits + got.notes + got.favourites + got.avatars + got.tags;
      if (total > 0) window.dispatchEvent(new Event("history:imported"));
      setBrought(
        total === 0
          ? "Nothing new: everything VRCX has for this account is already here."
          : `Brought over ${got.feed.toLocaleString()} activity entries, ${got.friendLog.toLocaleString()} friend log entries, ${got.visits.toLocaleString()} world visits with ${got.people.toLocaleString()} people, ${got.avatars.toLocaleString()} avatars you wore, ${got.notes} notes, ${got.tags} avatar tags and ${got.favourites} favourites kept on this PC. A backup from just before is saved as "${backup}". The lists show it after the app is opened again.`,
      );
    } catch (err) {
      setBrought(`Nothing was brought over: ${errorText(err)}`);
    } finally {
      setBringing(false);
    }
  }
  const [tab, setTab] = useState(lastTab);
  // Starting with Windows is kept by Windows, so it is asked for and not read from my settings.
  const [autostart, setAutostart] = useState(false);
  useEffect(() => {
    void backend
      .getAutostart()
      .then(setAutostart)
      .catch(() => {});
  }, []);
  const [working, setWorking] = useState(false);

  const loadBackups = () =>
    backend
      .backups()
      .then(setBackups)
      .catch(() => {});
  useEffect(() => void loadBackups(), []);

  async function backUp() {
    setWorking(true);
    setDataNote("");
    try {
      const made = await backend.backupNow(JSON.stringify(settings));
      setDataNote(`Saved "${made.name}" in ${made.path.replace(/[\\/][^\\/]*$/, "")}.`);
      await loadBackups();
    } catch (err) {
      setDataNote(`The backup failed: ${errorText(err)}`);
    } finally {
      setWorking(false);
    }
  }

  async function restore(name: string) {
    setWorking(true);
    setDataNote("");
    try {
      const old = await backend.restoreBackup(name, JSON.stringify(settings));
      if (old) {
        try {
          change(JSON.parse(old));
        } catch {
          // Settings in the backup were unreadable; the history still comes back.
        }
      }
      setRestored(true);
      setAsking("");
      await loadBackups();
    } catch (err) {
      setDataNote(`The restore failed: ${errorText(err)}`);
    } finally {
      setWorking(false);
    }
  }

  async function sendTest() {
    setTestResult("");
    try {
      const allowed = await backend.notify("Anonymous VRX", "This is what a notification looks like.", true);
      setTestResult(
        allowed
          ? "Handed to Windows. If nothing appeared by the clock, Windows is holding it back: check Settings, System, Notifications, and turn off Do not disturb. Windows is also more reliable about this once the app is installed with its installer."
          : "Windows said this app is not allowed to send notifications.",
      );
    } catch (err) {
      setTestResult(`Windows refused the notification: ${errorText(err)}`);
    }
  }

  return (
    <div className="settings">
      <div className="tabs settings-tabs" role="tablist" aria-label="Settings">
        {settingsTabs.map(([id, label]) => (
          <button
            key={id}
            role="tab"
            aria-selected={tab === id}
            className={tab === id ? "on" : ""}
            onClick={() => {
              lastTab = id;
              setTab(id);
            }}
          >
            {label}
          </button>
        ))}
      </div>
      <div className="settings-cols" data-show={tab}>
      <section data-group="look">
        <h2>Privacy</h2>
        <Switch
          label="Hide names and pictures"
          hint="Covers people with plain bars and blocks: their names, pictures, statuses and bios, your own included, and the names and pictures in My uploads. Nothing is blurred, so nothing can be sharpened back. Worlds stay readable, and so do the frame, effect and name plate a person picked for their profile. What you type into the history search is covered too. Ctrl+Shift+H switches it from anywhere. Windows notifications still show names."
          on={settings.hidePeople}
          set={(on) => change({ hidePeople: on })}
        />
      </section>
      <section data-group="look">
        <h2>Look</h2>
        <div className="setting">
          <span>
            <span className="setting-name">Size</span>
            <span className="setting-hint">
              "Fit the window" picks a size from how wide the window is. Right now that is {Math.round(fitted * 100)}%.
            </span>
          </span>
          <div className="choices" role="radiogroup" aria-label="Size">
            <button role="radio" aria-checked={settings.size === null} className={settings.size === null ? "on" : ""} onClick={() => change({ size: null })}>
              Fit the window
            </button>
            {sizes.map((s) => (
              <button key={s} role="radio" aria-checked={settings.size === s} className={settings.size === s ? "on" : ""} onClick={() => change({ size: s })}>
                {Math.round(s * 100)}%
              </button>
            ))}
          </div>
        </div>
        <div className="setting">
          <span>
            <span className="setting-name">Background</span>
            <span className="setting-hint">Pick a look, two colours of your own, or a picture from this PC.</span>
          </span>
          <div className="choices" role="radiogroup" aria-label="Background">
            {(
              [
                ["sunset", "Sunset"],
                ["night", "Night"],
                ["mono", "Black and white"],
                ["red", "Red and black"],
                ["custom", "My colours"],
                ["image", "My picture"],
              ] as const
            ).map(([value, label]) => (
              <button key={value} role="radio" aria-checked={settings.background === value} className={settings.background === value ? "on" : ""} onClick={() => change({ background: value })}>
                {label}
              </button>
            ))}
          </div>
        </div>
        {settings.background === "custom" && (
          <div className="setting">
            <span>
              <span className="setting-name">Background colours</span>
              <span className="setting-hint">The background blends from the first to the second.</span>
            </span>
            <div className="swatches">
              <Swatch label="From" value={settings.bgA} set={(v) => change({ bgA: v })} />
              <Swatch label="To" value={settings.bgB} set={(v) => change({ bgB: v })} />
            </div>
          </div>
        )}
        {settings.background === "image" && (
          <div className="setting setting-top">
            <span>
              <span className="setting-name">Picture</span>
              <span className="setting-hint">
                Stays on this PC. Large pictures are shrunk to fit. Darken it until the text is easy to read.
              </span>
              {pictureNote && <span className="setting-hint">{pictureNote}</span>}
            </span>
            <div className="picture-tools">
              <label className="plain file-pick">
                {picture ? "Choose another picture" : "Choose a picture"}
                <input type="file" accept="image/*" onChange={(e) => void pickPicture(e.target.files?.[0])} />
              </label>
              {picture && (
                <button className="plain" onClick={() => setPicture("")}>
                  Remove
                </button>
              )}
              <label className="dim">
                Darken
                <input
                  type="range"
                  min={0}
                  max={0.85}
                  step={0.05}
                  value={settings.imageDim}
                  onChange={(e) => change({ imageDim: Number(e.target.value) })}
                />
              </label>
            </div>
          </div>
        )}
        <div className="setting">
          <span>
            <span className="setting-name">Colours</span>
            <span className="setting-hint">
              Main colour is for buttons and highlights, second colour for links and tags. Reset goes back to what
              the background came with.
            </span>
          </span>
          <div className="swatches">
            <Swatch label="Main" value={settings.accent ?? base.accent} set={(v) => change({ accent: v })} />
            <Swatch label="Second" value={settings.accent2 ?? base.accent2} set={(v) => change({ accent2: v })} />
            <button className="plain" disabled={settings.accent === null && settings.accent2 === null} onClick={() => change({ accent: null, accent2: null })}>
              Reset
            </button>
          </div>
        </div>
        <div className="setting">
          <span>
            <span className="setting-name">Text colours</span>
            <span className="setting-hint">Main text, and the quieter text used for details.</span>
          </span>
          <div className="swatches">
            <Swatch label="Text" value={settings.textColor ?? base.text} set={(v) => change({ textColor: v })} />
            <Swatch label="Details" value={settings.softTextColor ?? base.soft} set={(v) => change({ softTextColor: v })} />
            <button className="plain" disabled={settings.textColor === null && settings.softTextColor === null} onClick={() => change({ textColor: null, softTextColor: null })}>
              Reset
            </button>
          </div>
        </div>
        <div className="setting">
          <span>
            <span className="setting-name">World cards</span>
            <span className="setting-hint">
              In order keeps cards in rows, read left to right, with the next row starting on the left. Packed tight
              slots each card under the shortest column, which wastes no space but changes the order.
            </span>
          </span>
          <div className="choices" role="radiogroup" aria-label="World cards">
            <button role="radio" aria-checked={settings.cardPacking === "rows"} className={settings.cardPacking === "rows" ? "on" : ""} onClick={() => change({ cardPacking: "rows" })}>
              In order
            </button>
            <button role="radio" aria-checked={settings.cardPacking === "tight"} className={settings.cardPacking === "tight" ? "on" : ""} onClick={() => change({ cardPacking: "tight" })}>
              Packed tight
            </button>
          </div>
        </div>
        <div className="setting">
          <span>
            <span className="setting-name">Rows</span>
            <span className="setting-hint">Tight rows fit more friends on screen.</span>
          </span>
          <div className="choices" role="radiogroup" aria-label="Rows">
            <button role="radio" aria-checked={settings.rows === "roomy"} className={settings.rows === "roomy" ? "on" : ""} onClick={() => change({ rows: "roomy" })}>
              Roomy
            </button>
            <button role="radio" aria-checked={settings.rows === "tight"} className={settings.rows === "tight" ? "on" : ""} onClick={() => change({ rows: "tight" })}>
              Tight
            </button>
          </div>
        </div>
      </section>

      <FontSettings settings={settings} change={change} />
      <DateSettings settings={settings} change={change} />
      <PeopleColourSettings settings={settings} change={change} swatch={(label, value, set) => <Swatch label={label} value={value} set={set} />} />
      <ProfileLookSettings settings={settings} change={change} />

      <section data-group="feed">
        <h2>Activity feed</h2>
        <Switch
          label="Group repeated world changes"
          hint="When a friend hops between worlds several times in a row, show one line with the rest tucked behind it."
          on={settings.feedFold}
          set={(on) => change({ feedFold: on })}
        />
        <Switch
          label="Show my own changes too"
          hint="Puts your own world changes, avatar changes and status changes into the feed, written as You. Recorded from the moment this is switched on."
          on={settings.feedMine}
          set={(on) => change({ feedMine: on })}
        />
        <div className={Object.keys(settings.feedHidden).length ? "setting setting-top" : "setting"}>
          <span>
            <span className="setting-name">Friends left out</span>
            <span className="setting-hint">
              Hidden with the Hide button on a feed entry. Their activity is still saved, just not shown.
            </span>
          </span>
          {Object.keys(settings.feedHidden).length === 0 ? (
            <span className="setting-hint">Nobody</span>
          ) : (
            <ul className="left-out">
              {Object.entries(settings.feedHidden).map(([id, name]) => (
                <li key={id}>
                  {name}
                  <button
                    className="plain"
                    onClick={() => {
                      const rest = { ...settings.feedHidden };
                      delete rest[id];
                      change({ feedHidden: rest });
                    }}
                  >
                    Show again
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      </section>

      <section data-group="feed">
        <h2>Discord</h2>
        <Switch
          label="Show where I am on Discord"
          hint="Puts the world you are in on your Discord profile while VRChat runs. It talks only to the Discord app on this PC. Off until you turn it on."
          on={settings.discordOn}
          set={(on) => change({ discordOn: on })}
        />
        <div className="setting setting-top">
          <span>
            <span className="setting-name">Discord application ID</span>
            <span className="setting-hint">
              Discord shows an activity under an application, and this one has to be your own. At
              discord.com/developers/applications choose New Application, give it the name Discord should show after
              "Playing", and copy its Application ID here. It is a public number, not a password.
            </span>
          </span>
          <div className="provider">
            <input
              type="text"
              inputMode="numeric"
              value={settings.discordAppId}
              onChange={(e) => change({ discordAppId: e.target.value.replace(/\D/g, "").slice(0, 20) })}
              placeholder="18 or 19 digits"
              aria-label="Discord application ID"
              spellCheck={false}
            />
          </div>
        </div>
        <p className="setting-hint">
          For the small round logo in the corner of the picture: in your Discord application open Rich Presence, Art
          Assets, upload a picture and name it logo. Without it the corner stays empty.
        </p>
        {settings.discordOn && !/^\d{17,20}$/.test(settings.discordAppId) && (
          <p className="setting-hint">Nothing is shown until an application ID is filled in.</p>
        )}
        <div className="setting">
          <span>
            <span className="setting-name">Wording</span>
            <span className="setting-hint">
              Plain reads "Muckelig" and "Friends+, 3 people". Night out reads "Out in Muckelig" and "Friends+, three
              deep", with "Off the grid" for private worlds.
            </span>
          </span>
          <div className="choices" role="radiogroup" aria-label="Discord wording">
            <button role="radio" aria-checked={settings.discordStyle === "night"} className={settings.discordStyle === "night" ? "on" : ""} onClick={() => change({ discordStyle: "night" })}>
              Night out
            </button>
            <button role="radio" aria-checked={settings.discordStyle === "plain"} className={settings.discordStyle === "plain" ? "on" : ""} onClick={() => change({ discordStyle: "plain" })}>
              Plain
            </button>
          </div>
        </div>
        <Switch label="Name the world" hint="With its picture. Off shows only that you are in VRChat." on={settings.discordWorld} set={(on) => change({ discordWorld: on })} />
        <Switch label="Say what kind of instance" hint="Public, Friends+, Friends and so on." on={settings.discordAccess} set={(on) => change({ discordAccess: on })} />
        <Switch label="Say how many people are there" on={settings.discordPeople} set={(on) => change({ discordPeople: on })} />
        <Switch
          label="Add a Join button"
          hint="Only on Public and Friends+ instances, where a link lets others in. Anyone who sees your Discord profile can press it."
          on={settings.discordJoin}
          set={(on) => change({ discordJoin: on })}
        />
        <Switch
          label="Add a world page button"
          hint="Opens the world's page on VRChat's website, so friends can look it up. It does not let anyone into your instance."
          on={settings.discordWorldPage}
          set={(on) => change({ discordWorldPage: on })}
        />
        <div className="setting">
          <span>
            <span className="setting-name">In private worlds</span>
            <span className="setting-hint">
              For Invite and Invite+ instances. Hide the details shows only "Off the grid" (or "In a private world"),
              with no world name, picture, head count or buttons. Show nothing takes the activity off your profile
              while you are there.
            </span>
          </span>
          <div className="choices" role="radiogroup" aria-label="In private worlds">
            {(
              [
                ["hush", "Hide the details"],
                ["none", "Show nothing"],
                ["show", "Show everything"],
              ] as const
            ).map(([id, label]) => (
              <button key={id} role="radio" aria-checked={settings.discordPrivateMode === id} className={settings.discordPrivateMode === id ? "on" : ""} onClick={() => change({ discordPrivateMode: id })}>
                {label}
              </button>
            ))}
          </div>
        </div>
      </section>


      <section data-group="general">
        <h2>Joining worlds</h2>
        <div className="setting">
          <span>
            <span className="setting-name">Start VRChat in</span>
            <span className="setting-hint">
              Used by "Join in VRChat" when the game is not open yet. Automatic picks VR only if SteamVR is already
              running, so a desktop session does not wake your headset. If the game is already open, it just moves
              you there.
            </span>
          </span>
          <div className="choices" role="radiogroup" aria-label="Start VRChat in">
            {(
              [
                ["auto", "Automatic"],
                ["desktop", "Desktop"],
                ["vr", "VR"],
              ] as const
            ).map(([value, label]) => (
              <button key={value} role="radio" aria-checked={settings.launchMode === value} className={settings.launchMode === value ? "on" : ""} onClick={() => change({ launchMode: value })}>
                {label}
              </button>
            ))}
          </div>
        </div>
        <label className="setting">
          <span>
            <span className="setting-name">
              Join by itself when a full instance has room <span className="mark">Beta</span>
            </span>
            <span className="setting-hint">
              On a full instance you can ask the app to tell you when a place is free. Off, it tells you and you press
              Join. On, it starts VRChat there or moves you the moment it sees room. The room alarm is new and has not
              been tried on many full instances yet.
            </span>
          </span>
          <input type="checkbox" role="switch" checked={settings.queueJoin} onChange={(e) => change({ queueJoin: e.target.checked })} />
        </label>
      </section>

      <InstanceSettings settings={settings} change={change} />

      <StatusRuleSettings settings={settings} change={change} />

      <section data-group="general">
        <h2>Starting up</h2>
        <Switch
          label="Start with Windows"
          hint="Starts in the tray when you sign in to Windows, so it is already recording by the time VRChat opens."
          on={autostart}
          set={(on) => {
            setAutostart(on);
            void backend.setAutostart(on).catch(() => setAutostart(!on));
          }}
        />
        <Switch
          label="Open the window when VRChat starts"
          hint="Works while the app is running, in the tray included."
          on={settings.openWithGame}
          set={(on) => change({ openWithGame: on })}
        />
        <div className="setting">
          <span>
            <span className="setting-name">When VRChat crashes</span>
            <span className="setting-hint">
              A crash is VRChat closing with no goodbye in its log while you are in a world. Ask shows a note with a
              Go back button. By itself starts VRChat again in the same instance, at most once every three minutes.
              An invite-only instance may need a new invite.
            </span>
          </span>
          <div className="choices" role="radiogroup" aria-label="When VRChat crashes">
            {(
              [
                ["off", "Do nothing"],
                ["ask", "Ask"],
                ["auto", "Go back by itself"],
              ] as const
            ).map(([value, label]) => (
              <button key={value} role="radio" aria-checked={settings.rejoin === value} className={settings.rejoin === value ? "on" : ""} onClick={() => change({ rejoin: value })}>
                {label}
              </button>
            ))}
          </div>
        </div>
      </section>

      <section data-group="general">
        <h2>Closing the window</h2>
        <div className="setting">
          <span>
            <span className="setting-name">When I close the window</span>
            <span className="setting-hint">
              In the tray the app keeps recording friend activity and sending notifications. Its icon sits by the
              clock, bottom right; click it to open the window again. After quitting, nothing is recorded until the
              next start.
            </span>
          </span>
          <div className="choices" role="radiogroup" aria-label="When I close the window">
            {(
              [
                ["ask", "Ask me"],
                ["tray", "Keep running in the tray"],
                ["quit", "Quit"],
              ] as const
            ).map(([value, label]) => (
              <button key={value} role="radio" aria-checked={settings.onClose === value} className={settings.onClose === value ? "on" : ""} onClick={() => change({ onClose: value })}>
                {label}
              </button>
            ))}
          </div>
        </div>
      </section>

      <PerformanceSettings />

      <VrNoticeSettings settings={settings} change={change} />
      <GameCacheSettings settings={settings} change={change} />

      <section data-group="general">
        <h2>Avatar search</h2>
        <div className="setting setting-top">
          <span>
            <span className="setting-name">Avatar database</span>
            <span className="setting-hint">
              VRChat has no avatar search for outside apps, so this asks a public, community-run database. It is
              not run by VRChat or by this app. Only the words you search for are sent to it, never your account.
            </span>
          </span>
          <div className="provider">
            <input
              type="url"
              value={settings.avatarProvider}
              onChange={(e) => change({ avatarProvider: e.target.value.trim() })}
              aria-label="Avatar database address"
              spellCheck={false}
            />
            <button className="plain" disabled={settings.avatarProvider === DEFAULT_AVATAR_PROVIDER} onClick={() => change({ avatarProvider: DEFAULT_AVATAR_PROVIDER })}>
              Reset
            </button>
          </div>
        </div>
        {EXTRA_AVATAR_PROVIDERS.map((p) => {
          const on = (settings.avatarExtras ?? []).includes(p.url);
          return (
            <Switch
              key={p.url}
              label={`Also ask ${p.name}`}
              hint="Another community database. Avatar search asks it too and joins the results."
              on={on}
              set={(next) => change({ avatarExtras: next ? [...(settings.avatarExtras ?? []), p.url] : (settings.avatarExtras ?? []).filter((u) => u !== p.url) })}
            />
          );
        })}
      </section>

      <section data-group="alerts">
        <h2>Windows notifications</h2>
        <Switch label="A favourite comes online" on={settings.alertFavouriteOnline} set={(on) => change({ alertFavouriteOnline: on })} />
        <Switch
          label="A favourite leaves a private world"
          hint="Only when they move to a world you can see."
          on={settings.alertFavouriteLeavesPrivate}
          set={(on) => change({ alertFavouriteLeavesPrivate: on })}
        />
        {(settings.alertFavouriteOnline || settings.alertFavouriteLeavesPrivate) && favorites.groups.length > 0 && (
          <div className="setting setting-top">
            <span>
              <span className="setting-name">Which favourite groups</span>
              <span className="setting-hint">The two notifications above only come for friends in the groups switched on here.</span>
            </span>
            <div className="group-alerts">
              {favorites.groups.map((g) => {
                const quiet = (settings.alertGroupsOff ?? []).includes(g.key);
                return (
                  <label key={g.key}>
                    <input
                      type="checkbox"
                      role="switch"
                      checked={!quiet}
                      onChange={(e) => {
                        const off = new Set(settings.alertGroupsOff ?? []);
                        if (e.target.checked) off.delete(g.key);
                        else off.add(g.key);
                        change({ alertGroupsOff: [...off] });
                      }}
                    />
                    {g.name}
                  </label>
                );
              })}
            </div>
          </div>
        )}
        {Object.keys(settings.alertPeople ?? {}).length > 0 && (
          <div className="setting setting-top">
            <span>
              <span className="setting-name">People with a rule of their own</span>
              <span className="setting-hint">Set on a friend's profile, under Notifications.</span>
            </span>
            <ul className="left-out">
              {Object.entries(settings.alertPeople).map(([id, who]) => (
                <li key={id}>
                  <span className="who">{who.name}</span>
                  <span className="setting-hint">{who.rule === "always" ? "always" : "never"}</span>
                  <button
                    className="plain"
                    onClick={() => {
                      const rest = { ...settings.alertPeople };
                      delete rest[id];
                      change({ alertPeople: rest });
                    }}
                  >
                    Remove
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}
        <Switch label="Invites and invite requests" on={settings.alertInvites} set={(on) => change({ alertInvites: on })} />
        <Switch label="Friend requests" on={settings.alertFriendRequests} set={(on) => change({ alertFriendRequests: on })} />
        <Switch label="Boops" on={settings.alertBoops} set={(on) => change({ alertBoops: on })} />
        <Switch
          label="Everything else"
          hint="Group invites, announcements and replies to your invites."
          on={settings.alertEverythingElse}
          set={(on) => change({ alertEverythingElse: on })}
        />
        <div className="setting">
          <span>
            <span className="setting-name">Before an event I follow starts</span>
            <span className="setting-hint">Follow events on the Events screen. Each one is announced once.</span>
          </span>
          <div className="choices" role="radiogroup" aria-label="Event reminder">
            {(
              [
                [0, "Off"],
                [5, "5 min"],
                [15, "15 min"],
                [30, "30 min"],
                [60, "1 hour"],
              ] as const
            ).map(([minutes, label]) => (
              <button key={minutes} role="radio" aria-checked={settings.eventReminder === minutes} className={settings.eventReminder === minutes ? "on" : ""} onClick={() => change({ eventReminder: minutes })}>
                {label}
              </button>
            ))}
          </div>
        </div>
        <div className="data-actions">
          <button className="plain" onClick={() => void sendTest()}>
            Send a test notification
          </button>
        </div>
        {testResult && (
          <p className="setting-hint" role="status">
            {testResult}
          </p>
        )}
      </section>

      <section data-group="alerts">
        <h2>Quiet times</h2>
        <p className="setting-hint">These hold back every Windows notification from this app. Nothing is lost: the Inbox and the feed still fill up.</p>
        <Switch label="Quiet hours" on={settings.quietHours} set={(on) => change({ quietHours: on })} />
        {settings.quietHours && (
          <div className="setting">
            <span>
              <span className="setting-name">From and until</span>
              <span className="setting-hint">Your PC's own time. It can run past midnight.</span>
            </span>
            <span className="quiet-times">
              <input type="time" value={settings.quietFrom} aria-label="Quiet from" onChange={(e) => e.target.value && change({ quietFrom: e.target.value })} />
              <input type="time" value={settings.quietTo} aria-label="Quiet until" onChange={(e) => e.target.value && change({ quietTo: e.target.value })} />
            </span>
          </div>
        )}
        <Switch
          label="Quiet while VRChat is running"
          hint="For when you would see it in the game anyway."
          on={settings.quietInGame}
          set={(on) => change({ quietInGame: on })}
        />
      </section>

      <section data-group="data">
        <h2>Your data</h2>
        <p className="setting-hint">
          A backup holds your friends list as last seen, the activity feed, world history, handled notifications
          and these settings. Your VRChat sign-in is never included. Backups are plain folders in Documents, so you
          can copy them to another drive.
        </p>
        <Switch
          label="Back up by itself once a day"
          hint="Made while the app runs, into the same folder, named automatic backup. Only automatic ones are ever removed; backups you make by hand stay until you delete them."
          on={settings.autoBackup}
          set={(on) => change({ autoBackup: on })}
        />
        {settings.autoBackup && (
          <div className="setting">
            <span>
              <span className="setting-name">Keep the newest</span>
              <span className="setting-hint">Older automatic backups are removed when a new one is made.</span>
            </span>
            <div className="choices" role="radiogroup" aria-label="Automatic backups kept">
              {[3, 7, 14, 30].map((n) => (
                <button key={n} role="radio" aria-checked={settings.autoBackupKeep === n} className={settings.autoBackupKeep === n ? "on" : ""} onClick={() => change({ autoBackupKeep: n })}>
                  {n}
                </button>
              ))}
            </div>
          </div>
        )}
        <div className="data-actions">
          <button className="plain go" disabled={working} onClick={() => void backUp()}>
            Back up now
          </button>
          <button className="plain" onClick={() => void backend.openBackupsFolder()}>
            Open the backups folder
          </button>
        </div>
        {dataNote && (
          <p className="setting-hint" role="status">
            {dataNote}
          </p>
        )}
        {restored && (
          <p className="banner" role="status">
            Restore is ready. It loads the next time the app starts. Your data from before the restore was saved
            as a backup too. <button onClick={() => void backend.quit()}>Quit now</button>
          </p>
        )}
        {backups.length > 0 && (
          <ul className="backups">
            {backups.map((b) => (
              <li key={b.name}>
                <span className="backup-name">{b.name}</span>
                <span className="backup-size">{(b.bytes / 1024 / 1024).toFixed(1)} MB</span>
                {asking === b.name ? (
                  <span className="backup-ask">
                    Replace what the app holds now with this backup?
                    <button className="plain" disabled={working} onClick={() => void restore(b.name)}>
                      Yes, restore
                    </button>
                    <button className="plain" onClick={() => setAsking("")}>
                      No
                    </button>
                  </span>
                ) : (
                  <button className="plain" disabled={working || restored} onClick={() => setAsking(b.name)}>
                    Restore
                  </button>
                )}
              </li>
            ))}
          </ul>
        )}
      </section>

      <section data-group="data">
        <h2>Moving over from VRCX</h2>
        <p className="setting-hint">
          Copies what VRCX recorded for this account into this app: friend activity, the friend log, world visits with who was there, the avatars you wore, your notes and avatar tags, and
          the favourites VRCX kept on this PC. VRCX's own file is only read, never changed, and a backup of this app's history is made first. Running
          it again only adds what is new.
        </p>
        <Switch
          label="Keep taking over what is new"
          hint="When the app starts, and once an hour after that, it looks whether VRCX has written anything and brings over only that. You can keep using both."
          on={settings.vrcxAuto}
          set={(on) => change({ vrcxAuto: on })}
        />
        <div className="data-actions">
          <button className="plain go" disabled={!vrcx || bringing} onClick={() => void bringOver()}>
            {bringing ? "Bringing it over" : "Bring my VRCX history over"}
          </button>
        </div>
        {!vrcx && <p className="setting-hint">VRCX's history was not found on this PC.</p>}
        {brought && (
          <p className="setting-hint" role="status">
            {brought}
          </p>
        )}
      </section>

      <section data-group="data">
        <h2>Account</h2>
        <div className="setting">
          <span className="person">
            <Avatar name={me.displayName} image={me.image} size={36} />
            {me.displayName}
          </span>
          <button className="plain" onClick={onSignOut}>
            Sign out
          </button>
        </div>
        <p className="setting-hint">
          Signing out removes the saved sign-in from this PC. Your history stays on the disk.
        </p>
      </section>

      <UpdateSettings settings={settings} change={change} />

      <section data-group="about">
        <h2>About</h2>
        <p className="about-line">Your VRChat night, on one screen.</p>
        <p className="setting-hint">
          Anonymous VRX {__APP_VERSION__}. The sun goes down, the city switches on, and this keeps it all in view: who
          is out, which world the lights are on in, and what happened while you were gone.
        </p>
        <p className="setting-hint">
          An independent app, not made by or affiliated with VRChat. Your password is sent to VRChat only and never
          saved; the sign-in cookie is kept in the Windows credential store; history is a file on this PC.
        </p>
      </section>

      <section data-group="about">
        <h2>Credits</h2>
        <p className="setting-hint">This app stands on other people's work. Thank you to all of them.</p>
        <ul className="credits">
          <li><b>Tauri</b> and <b>React</b>, with the Rust crates they bring along. The window and the interface. MIT and Apache 2.0.</li>
          <li>
            <b>Instrument Sans</b>, <b>Barlow Condensed</b>, <b>Yellowtail</b>, <b>Rajdhani</b>, <b>Orbitron</b>, <b>Outfit</b> and <b>Space Grotesk</b>. The fonts. SIL Open Font License 1.1
            and Apache 2.0.
          </li>
          <li>
            <b>VRChat</b> by VRChat Inc., and the community avatar databases that answer avatar search.
          </li>
        </ul>
        <button
          className="plain"
          onClick={() => void backend.openLicencesFolder().catch((err) => setTestResult(errorText(err)))}
        >
          Open the licence texts
        </button>
      </section>
      </div>
    </div>
  );
}
