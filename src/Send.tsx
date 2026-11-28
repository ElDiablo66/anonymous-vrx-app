import { useEscape, useVeil } from "./escape";
import { useEffect, useRef, useState, type CSSProperties } from "react";
import { backend, errorText, type BackendError, type Friend, type InviteMessage } from "./backend";
import { colourOf, textureName, textures, textureUrl, useDress } from "./look";
import { live } from "./settings";

export { useEscape };

/** My saved lines as VRChat sends them, with anything missing filled in so a line is never sent to the wrong slot. */
function lines(raw: unknown): InviteMessage[] {
  const list = Array.isArray(raw) ? raw : [];
  return list
    .filter((m): m is Record<string, unknown> => !!m && typeof m === "object" && typeof (m as { slot?: unknown }).slot === "number" && typeof (m as { message?: unknown }).message === "string")
    .map((m) => ({
      slot: m.slot as number,
      message: m.message as string,
      remainingCooldownMinutes: typeof m.remainingCooldownMinutes === "number" ? m.remainingCooldownMinutes : 0,
      canBeUpdated: m.canBeUpdated !== false,
    }))
    .sort((a, b) => a.slot - b.slot);
}

/** VRChat's own boop emoji, by its name there, with the nearest character to show for it. */
const emoji: Array<[string, string, string]> = [
  ["default_heart", "❤️", "Heart"], ["default_in_love", "😍", "In love"], ["default_kiss", "😘", "Kiss"], ["default_blushing", "😊", "Blushing"],
  ["default_smile", "🙂", "Smile"], ["default_laugh", "😂", "Laugh"], ["default_tongue_out", "😛", "Tongue out"], ["default_wow", "😮", "Wow"],
  ["default_thinking", "🤔", "Thinking"], ["default_stoic", "😐", "Stoic"], ["default_frown", "🙁", "Frown"], ["default_crying", "😢", "Crying"],
  ["default_angry", "😠", "Angry"], ["default_shush", "🤫", "Shush"], ["default_cantsee", "🙈", "Cannot see"], ["default_sunglasses", "😎", "Sunglasses"],
  ["default_neon_shades", "🕶️", "Neon shades"], ["default_zzz", "💤", "Sleepy"], ["default_hand_wave", "👋", "Wave"], ["default_hang_ten", "🤙", "Hang ten"],
  ["default_thumbs_up", "👍", "Thumbs up"], ["default_thumbs_down", "👎", "Thumbs down"], ["default_arrowpoint", "👉", "Pointing"], ["default_exclamation", "❗", "Exclamation"],
  ["default_question", "❓", "Question"], ["default_go", "🟢", "Go"], ["default_stop", "🛑", "Stop"], ["default_broken_heart", "💔", "Broken heart"],
  ["default_fire", "🔥", "Fire"], ["default_confetti", "🎉", "Confetti"], ["default_gift", "🎁", "Gift"], ["default_gifts", "🎀", "Gifts"],
  ["default_music_note", "🎵", "Music"], ["default_money", "💰", "Money"], ["default_pizza", "🍕", "Pizza"], ["default_ice_cream", "🍦", "Ice cream"],
  ["default_candy", "🍬", "Candy"], ["default_tomato", "🍅", "Tomato"], ["default_pineapple", "🍍", "Pineapple"], ["default_beer", "🍺", "Beer"],
  ["default_drink", "🍹", "Drink"], ["default_champagne", "🍾", "Champagne"], ["default_keyboard", "⌨️", "Keyboard"], ["default_nomic", "🔇", "No microphone"],
  ["default_noheadphones", "🎧", "No headphones"], ["default_portal", "🌀", "Portal"], ["default_hourglass", "⏳", "Hourglass"], ["default_cloud", "☁️", "Cloud"],
  ["default_splash", "💦", "Splash"], ["default_beachball", "🏐", "Beach ball"], ["default_life_ring", "🛟", "Life ring"], ["default_sun_lotion", "🧴", "Sun lotion"],
  ["default_skull", "💀", "Skull"], ["default_spooky_ghost", "👻", "Ghost"], ["default_boo", "😱", "Boo"], ["default_bats", "🦇", "Bats"],
  ["default_web", "🕸️", "Web"], ["default_jack_o_lantern", "🎃", "Pumpkin"], ["default_candy_corn", "🌽", "Candy corn"], ["default_snow_fall", "❄️", "Snow"],
  ["default_snowball", "⚪", "Snowball"], ["default_candy_cane", "🍭", "Candy cane"], ["default_gingerbread", "🍪", "Gingerbread"], ["default_mistletoe", "🌿", "Mistletoe"],
  ["default_coal", "🪨", "Coal"],
];

type Ask = { kind: "boop" | "message"; userId: string; name: string };

/** Opens the small window for a boop with an emoji, or an invite with one of my saved messages. */
export function openSend(ask: Ask) {
  window.dispatchEvent(new CustomEvent("open-send", { detail: ask }));
}

function lastUsed(): string[] {
  try {
    const list = JSON.parse(localStorage.getItem("boop-recent") ?? "[]") as unknown;
    return Array.isArray(list) ? list.filter((x): x is string => typeof x === "string").slice(0, 6) : [];
  } catch {
    return [];
  }
}

/** Mounted once in the window; shows itself when something asks to send a boop or a message. */
export function SendPanel({ inWorld, friends, onDone }: { inWorld: boolean; friends: Map<string, Friend>; onDone: (message: string) => void }) {
  const [ask, setAsk] = useState<Ask | null>(null);
  useEffect(() => {
    const open = (e: Event) => setAsk((e as CustomEvent<Ask>).detail);
    window.addEventListener("open-send", open);
    return () => window.removeEventListener("open-send", open);
  }, []);
  useEscape(() => setAsk(null), ask !== null);
  if (!ask) return null;
  const close = () => setAsk(null);
  return (
    <div className="veil" onClick={close}>
      <div className="dialog send" role="dialog" aria-modal="true" aria-label={ask.kind === "boop" ? "Boop" : "Invite with a message"} onClick={(e) => e.stopPropagation()}>
        {ask.kind === "boop" ? <Boop ask={ask} onDone={onDone} onClose={close} /> : <Messages ask={ask} inWorld={inWorld} online={friends.get(ask.userId)?.state === "online"} onDone={onDone} onClose={close} />}
      </div>
    </div>
  );
}

function Boop({ ask, onDone, onClose }: { ask: Ask; onDone: (message: string) => void; onClose: () => void }) {
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState("");
  const recent = lastUsed()
    .map((id) => emoji.find((e) => e[0] === id))
    .filter((e): e is [string, string, string] => !!e);

  async function send(id?: string, label?: string) {
    setBusy(true);
    setProblem("");
    try {
      await backend.boop(ask.userId, id);
      if (id) {
        try {
          localStorage.setItem("boop-recent", JSON.stringify([id, ...lastUsed().filter((x) => x !== id)].slice(0, 6)));
        } catch {
          // Then nothing is remembered for next time.
        }
      }
      onDone(label ? `Booped ${ask.name} with ${label.toLowerCase()}.` : `Booped ${ask.name}.`);
      onClose();
    } catch (err) {
      setProblem(errorText(err));
      setBusy(false);
    }
  }

  return (
    <>
      <h2>
        Boop <span className="hide-name">{ask.name}</span>
      </h2>
      <p>A boop shows up for them inside VRChat. Pick an emoji to send with it, or send it plain.</p>
      {recent.length > 0 && (
        <>
          <h3 className="send-sub">Used lately</h3>
          <div className="emoji-grid emoji-recent">
            {recent.map(([id, sign, label]) => (
              <button key={id} disabled={busy} title={label} aria-label={label} onClick={() => void send(id, label)}>
                {sign}
              </button>
            ))}
          </div>
        </>
      )}
      <h3 className="send-sub">VRChat's emoji</h3>
      <div className="emoji-grid">
        {emoji.map(([id, sign, label]) => (
          <button key={id} disabled={busy} title={label} aria-label={label} onClick={() => void send(id, label)}>
            {sign}
          </button>
        ))}
      </div>
      <p className="send-fine">The pictures here stand in for VRChat's own drawings; what arrives is VRChat's emoji of that name.</p>
      {problem && (
        <p className="banner" role="alert">
          {problem}
        </p>
      )}
      <div className="dialog-actions">
        <button onClick={onClose}>Cancel</button>
        <button className="go" disabled={busy} autoFocus onClick={() => void send()}>
          {busy ? "Sending" : "Plain boop"}
        </button>
      </div>
    </>
  );
}

function Messages({ ask, inWorld, online, onDone, onClose }: { ask: Ask; inWorld: boolean; online: boolean; onDone: (message: string) => void; onClose: () => void }) {
  // Asking to join makes sense while they are in VRChat; inviting needs me to be in a world.
  const [kind, setKind] = useState<"message" | "request">(inWorld ? "message" : "request");
  const [lists, setLists] = useState<Partial<Record<"message" | "request", InviteMessage[]>>>({});
  const [problem, setProblem] = useState("");
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const list = lists[kind];

  useEffect(() => {
    if (lists[kind]) return;
    let alive = true;
    setProblem("");
    void backend
      .inviteMessages(kind)
      .then((l) => alive && setLists((old) => ({ ...old, [kind]: lines(l) })))
      .catch((err) => alive && setProblem(errorText(err)));
    return () => {
      alive = false;
    };
  }, [kind]); // eslint-disable-line react-hooks/exhaustive-deps

  const can = kind === "message" ? inWorld : online;
  const why = kind === "message" ? "You are not in a world right now, so there is nowhere to invite them to." : "They are not in VRChat right now, so there is nothing to ask to join.";

  async function send(slot?: number) {
    setBusy(true);
    setProblem("");
    try {
      if (kind === "message") await backend.inviteHere(ask.userId, slot);
      else await backend.requestInvite(ask.userId, slot);
      onDone(kind === "message" ? `Invited ${ask.name} to where you are.` : `Asked ${ask.name} for an invite. Their answer arrives in your Inbox.`);
      onClose();
    } catch (err) {
      setProblem(errorText(err));
      setBusy(false);
    }
  }

  async function save(slot: number) {
    setBusy(true);
    setProblem("");
    try {
      const next = await backend.setInviteMessage(kind, slot, draft.trim());
      setLists((old) => ({ ...old, [kind]: lines(next) }));
      setEditing(null);
    } catch (err) {
      setProblem(errorText(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <h2>
        Message <span className="hide-name">{ask.name}</span>
      </h2>
      <p>VRChat has no chat between friends. An invite, or asking for one, can carry one of your twelve saved lines, and that is what this sends.</p>
      <div className="choices send-kinds" role="radiogroup" aria-label="What to send">
        <button role="radio" aria-checked={kind === "message"} className={kind === "message" ? "on" : ""} onClick={() => (setKind("message"), setEditing(null))}>
          Invite to my world
        </button>
        <button role="radio" aria-checked={kind === "request"} className={kind === "request" ? "on" : ""} onClick={() => (setKind("request"), setEditing(null))}>
          Ask for an invite
        </button>
      </div>
      {!can && <p className="send-fine">{why} You can still change your saved lines here.</p>}
      {problem && (
        <p className="banner" role="alert">
          {problem}
        </p>
      )}
      {!list && !problem && <p className="send-fine">Asking VRChat for your saved lines.</p>}
      {list && (
        <ol className="send-lines">
          {list.map((m) => (
            <li key={m.slot}>
              {editing === m.slot ? (
                <>
                  <input value={draft} maxLength={64} autoFocus aria-label={`Saved line ${m.slot + 1}`} onChange={(e) => setDraft(e.target.value)} onKeyDown={(e) => e.key === "Enter" && !busy && draft.trim() && void save(m.slot)} />
                  <small>{64 - draft.length} left</small>
                  <button className="plain go" disabled={busy || !draft.trim()} onClick={() => void save(m.slot)}>
                    Save
                  </button>
                  <button className="plain" disabled={busy} onClick={() => setEditing(null)}>
                    Cancel
                  </button>
                </>
              ) : (
                <>
                  <span className="send-line">{m.message}</span>
                  <button
                    className="plain"
                    disabled={busy || !m.canBeUpdated}
                    title={m.canBeUpdated ? "Change this line. VRChat lets each line change once an hour" : `VRChat lets this line change again in ${m.remainingCooldownMinutes} min`}
                    onClick={() => {
                      setDraft(m.message);
                      setEditing(m.slot);
                    }}
                  >
                    {m.canBeUpdated ? "Change" : `${m.remainingCooldownMinutes} min`}
                  </button>
                  <button className="plain go" disabled={busy || !can} onClick={() => void send(m.slot)}>
                    Send
                  </button>
                </>
              )}
            </li>
          ))}
        </ol>
      )}
      <div className="dialog-actions">
        <button onClick={onClose}>Cancel</button>
        <button disabled={busy || !can} onClick={() => void send()}>
          Send without a line
        </button>
      </div>
    </>
  );
}

/** Every language VRChat lets a profile name, in plain words, for the language picker. */
const languageNames: Record<string, string> = {
  afr: "Afrikaans", ara: "Arabic", ase: "American Sign Language", asf: "Australian Sign Language", ben: "Bengali", bfi: "British Sign Language",
  bul: "Bulgarian", ces: "Czech", cmn: "Mandarin Chinese", cym: "Welsh", dan: "Danish", deu: "German", dse: "Dutch Sign Language", ell: "Greek",
  eng: "English", epo: "Esperanto", est: "Estonian", fil: "Filipino", fin: "Finnish", fra: "French", fsl: "French Sign Language",
  gla: "Scottish Gaelic", gle: "Irish", gsg: "German Sign Language", heb: "Hebrew", hin: "Hindi", hmn: "Hmong", hrv: "Croatian", hun: "Hungarian",
  hye: "Armenian", ind: "Indonesian", isl: "Icelandic", ita: "Italian", jpn: "Japanese", jsl: "Japanese Sign Language", kor: "Korean",
  kvk: "Korean Sign Language", lav: "Latvian", lit: "Lithuanian", ltz: "Luxembourgish", mar: "Marathi", mkd: "Macedonian", mlt: "Maltese",
  mri: "Maori", msa: "Malay", nld: "Dutch", nor: "Norwegian", nzs: "New Zealand Sign Language", pol: "Polish", por: "Portuguese",
  ron: "Romanian", rus: "Russian", sco: "Scots", slk: "Slovak", slv: "Slovenian", spa: "Spanish", swe: "Swedish", tel: "Telugu", tha: "Thai",
  tok: "Toki Pona", tur: "Turkish", tws: "Teochew", ukr: "Ukrainian", vie: "Vietnamese", wuu: "Wu Chinese", yue: "Cantonese", zho: "Chinese",
};

/**
 * The languages a profile shows: VRChat keeps them as tags, and the newer record repeats them.
 * All of them: a save that stopped halfway can leave more than three for a moment.
 */
function allSpokenOf(raw: Record<string, unknown>): string[] {
  return [
    ...new Set([
      ...(Array.isArray(raw.tags) ? raw.tags : []).filter((t): t is string => typeof t === "string" && t.startsWith("language_")).map((t) => t.slice(9)),
      ...(Array.isArray(raw.languages) ? raw.languages : []).filter((l): l is string => typeof l === "string").map((l) => l.replace(/^language_/, "")),
    ]),
  ];
}

/** The first three, as the picker shows them. */
function spokenOf(raw: Record<string, unknown>): string[] {
  return allSpokenOf(raw).slice(0, 3);
}

type Look = { iconFrame: string; profileEffect: string; nameplateEffect: string; backgroundType: string; top: string; bottom: string; texture: string };

const hex = (v: unknown, fallback: string) => (typeof v === "string" && /^[0-9a-f]{6}$/i.test(v.replace(/^#/, "")) ? v.replace(/^#/, "").toLowerCase() : fallback);

/** A dusk banner for the effect thumbnails, so their edge glow has something to show against. */
const FX_BACKDROP = "radial-gradient(120% 90% at 50% 110%, #ff7a59 0%, rgb(255 122 89 / 0) 55%), linear-gradient(170deg, #4b2380 0%, #2a1252 60%, #1a0b33 100%)";

/** Sent on the window after my own profile was saved. */
export const PROFILE_SAVED = "avrx:profile-saved";

/** My own profile, changed the way VRChat's website changes it: what it says, and how it looks. */
export function EditProfile({ meId, onClose, onBack, onDone }: { meId: string; onClose: () => void; onBack?: () => void; onDone: (message: string) => void }) {
  const [part, setPart] = useState<"about" | "look">("about");
  const [loaded, setLoaded] = useState(false);
  const [name, setName] = useState("");
  const [icon, setIcon] = useState("");
  const [banner, setBanner] = useState("");
  const [bio, setBio] = useState("");
  const [pronouns, setPronouns] = useState("");
  const [links, setLinks] = useState<string[]>(["", "", ""]);
  const [langs, setLangs] = useState<string[]>([]);
  const [look, setLook] = useState<Look>({ iconFrame: "", profileEffect: "", nameplateEffect: "", backgroundType: "default", top: "1a0f35", bottom: "ff4fa3", texture: "grid" });
  const [first, setFirst] = useState<{ bio: string; pronouns: string; links: string[]; langs: string[]; look: Look } | null>(null);
  const [owned, setOwned] = useState<Record<string, string[]> | null>(null);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState("");
  const dress = useDress(true);

  useEffect(() => {
    let alive = true;
    void backend
      .profile(meId)
      .then((p) => {
        if (!alive) return;
        const raw = (p.raw ?? {}) as Record<string, unknown>;
        // Nothing may be saved over a profile that did not arrive whole. The bio and its links come
        // from VRChat's newer profile record, so that record has to have been read.
        if (!p.recordRead || typeof raw.bio !== "string" || !Array.isArray(raw.bioLinks)) {
          setProblem("VRChat did not send your whole profile, so nothing can be changed safely right now. Close this and try again in a minute.");
          return;
        }
        const had = raw.bioLinks.filter((l): l is string => typeof l === "string" && l !== "");
        const spoken = spokenOf(raw);
        const kind = typeof raw.backgroundType === "string" ? raw.backgroundType : "default";
        const nowLook: Look = {
          iconFrame: typeof raw.iconFrame === "string" ? raw.iconFrame : "",
          profileEffect: typeof raw.profileEffect === "string" ? raw.profileEffect : "",
          nameplateEffect: typeof raw.nameplateEffect === "string" ? raw.nameplateEffect : "",
          backgroundType: kind,
          top: hex(raw.backgroundGradientTop, "1a0f35"),
          bottom: hex(raw.backgroundGradientBottom, "ff4fa3"),
          texture: typeof raw.backgroundTextureId === "string" && textureUrl(raw.backgroundTextureId) ? raw.backgroundTextureId : "grid",
        };
        const nowLinks = [...had, "", "", ""].slice(0, Math.max(3, had.length));
        const nowPronouns = typeof raw.pronouns === "string" ? raw.pronouns : "";
        setName(typeof raw.displayName === "string" ? raw.displayName : "");
        setIcon(String(raw.iconUrl || raw.userIcon || raw.profilePicOverride || raw.currentAvatarThumbnailImageUrl || ""));
        setBanner(String(raw.bannerUrl || raw.bannerCustomUrl || ""));
        setBio(raw.bio);
        setPronouns(nowPronouns);
        setLinks(nowLinks);
        setLangs(spoken);
        setLook(nowLook);
        setFirst({ bio: raw.bio, pronouns: nowPronouns, links: nowLinks, langs: spoken, look: nowLook });
        setLoaded(true);
      })
      .catch((err) => alive && setProblem(`Your profile could not be read: ${errorText(err)}`));
    void backend
      .myCosmetics()
      .then((m) => alive && setOwned(m))
      .catch(() => alive && setOwned({}));
    return () => {
      alive = false;
    };
  }, [meId]);

  const bad = links.find((l) => l.trim() !== "" && !/^https?:\/\/\S+$/.test(l.trim()));
  const set = (patch: Partial<Look>) => setLook((old) => ({ ...old, ...patch }));

  // What has changed since the profile was read; leaving with changes asks first.
  const cleanLinks = links.map((l) => l.trim()).filter(Boolean);
  const bioChanged = !!first && (bio !== first.bio || cleanLinks.join("\n") !== first.links.map((l) => l.trim()).filter(Boolean).join("\n"));
  const pronounsChanged = !!first && pronouns.trim() !== first.pronouns.trim();
  const aboutChanged = bioChanged || pronounsChanged;
  const langsChanged = !!first && langs.join() !== first.langs.join();
  const lookChanged = !!first && (["iconFrame", "profileEffect", "nameplateEffect", "backgroundType", "top", "bottom", "texture"] as const).some((k) => look[k] !== first.look[k]);
  const dirty = aboutChanged || langsChanged || lookChanged;
  // Where leaving goes: closed, or back to the window this was opened from. Changes ask first.
  const [leaving, setLeaving] = useState<"" | "close" | "back">("");
  useEffect(() => setLeaving(""), [bio, pronouns, links, langs, look]);
  const tryClose = () => (leaving ? setLeaving("") : dirty ? setLeaving("close") : onClose());
  const tryBack = () => (dirty ? setLeaving("back") : onBack?.());
  // Escape answers the question first; then it goes back to My status when that is where this came from.
  useEscape(() => (leaving ? setLeaving("") : onBack ? tryBack() : tryClose()));
  const veil = useVeil(tryClose);
  // The question sits at the bottom of a long dialog, so it is scrolled into sight and gets the keyboard.
  const keepButton = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!leaving) return;
    keepButton.current?.scrollIntoView({ block: "nearest" });
    keepButton.current?.focus();
  }, [leaving]);

  /**
   * Languages go to VRChat in two steps, so a save that failed can leave them half changed. They
   * are read again and become what the next save starts from, and the note says what VRChat has
   * now. The picks stay, so saving again sends only what is still missing.
   */
  async function readLanguagesAgain(problem: string) {
    try {
      const p = await backend.profile(meId);
      const now = allSpokenOf((p.raw ?? {}) as Record<string, unknown>);
      setFirst((old) => (old ? { ...old, langs: now } : old));
      const named = now.map((c) => languageNames[c] ?? c).join(", ");
      setProblem(`${problem} Your languages on VRChat now: ${named || "none"}.`);
    } catch {
      // The note about the failed save stays as it is.
    }
  }

  async function save() {
    if (!first) return;
    setBusy(true);
    setProblem("");
    // Set while the languages are being saved, so a failure there reads them again.
    let atLanguages = false;
    try {
      // The bio with its links, and the pronouns, go only when they changed.
      if (aboutChanged) await backend.updateProfile(bioChanged ? { bio, links: cleanLinks } : null, pronounsChanged ? pronouns.trim() : null);
      atLanguages = true;
      if (langsChanged) await backend.updateLanguages(langs.filter((c) => !first.langs.includes(c)), first.langs.filter((c) => !langs.includes(c)));
      atLanguages = false;
      // Only what changed goes to VRChat, so nothing I did not touch is written over.
      const changes: Record<string, unknown> = {};
      for (const key of ["iconFrame", "profileEffect", "nameplateEffect"] as const) if (look[key] !== first.look[key]) changes[key] = look[key];
      const bgChanged = look.backgroundType !== first.look.backgroundType || (look.backgroundType === "gradient" && (look.top !== first.look.top || look.bottom !== first.look.bottom)) || (look.backgroundType === "texture" && look.texture !== first.look.texture);
      if (bgChanged && look.backgroundType !== "inventory") {
        changes.backgroundType = look.backgroundType;
        if (look.backgroundType === "gradient") Object.assign(changes, { backgroundGradientTop: look.top, backgroundGradientBottom: look.bottom });
        if (look.backgroundType === "texture") changes.backgroundTextureId = look.texture;
      }
      if (Object.keys(changes).length > 0) await backend.updateLook(changes);
      const changed = aboutChanged || langsChanged || Object.keys(changes).length > 0;
      // An open profile window of mine reads itself again, so it shows what VRChat now has.
      if (changed) window.dispatchEvent(new Event(PROFILE_SAVED));
      onDone(changed ? "Your profile is changed." : "Nothing was changed.");
      onClose();
    } catch (err) {
      setProblem(errorText(err));
      // Not after VRChat asked to wait: then nothing more is sent, not even a read. Save stays
      // off until the read is back, so a second save starts from what VRChat really has.
      const waited = (err as Partial<BackendError> | null)?.kind === "rateLimited" || /\(.*429|asked to wait/i.test(errorText(err));
      if (atLanguages && langsChanged && !waited) await readLanguagesAgain(errorText(err));
      setBusy(false);
    }
  }

  const pickable = (kind: "iconFrame" | "profileEffect" | "nameplateEffect") => {
    const mine = owned?.[kind] ?? [];
    const current = look[kind];
    return current && !mine.includes(current) ? [current, ...mine] : mine;
  };
  const framePic = dress[look.iconFrame]?.picture ?? "";
  const effectPic = dress[look.profileEffect]?.picture ?? "";
  const plate = dress[look.nameplateEffect];
  const plateBg = plate ? [plate.picture && `url("${plate.picture}")`, colourOf(plate.from) && colourOf(plate.to) ? `linear-gradient(90deg, ${colourOf(plate.from)}, ${colourOf(plate.to)})` : ""].filter(Boolean).join(", ") : "";
  // The preview draws the background through the same layers as a profile window, at the same strength.
  const backdrop =
    look.backgroundType === "gradient"
      ? `linear-gradient(180deg, #${look.top}, #${look.bottom})`
      : look.backgroundType === "texture"
        ? `url("${textureUrl(look.texture)}")`
        : "";
  const noneOwned = owned !== null && (["iconFrame", "profileEffect", "nameplateEffect"] as const).every((k) => pickable(k).length === 0);

  return (
    <div className="veil" {...veil}>
      <div className="dialog send profile-edit" role="dialog" aria-modal="true" aria-labelledby="edit-title" onClick={(e) => e.stopPropagation()}>
        {onBack && (
          <button className="edit-back" onClick={tryBack}>
            ‹ Back to My status
          </button>
        )}
        <h2 id="edit-title">My profile</h2>
        <p>What other people see on your profile. It is saved to VRChat the same way its website does it.</p>
        <div className="choices edit-parts" role="tablist" aria-label="Part of the profile">
          <button role="tab" aria-selected={part === "about"} className={part === "about" ? "on" : ""} onClick={() => setPart("about")}>
            About me
            {bad && (
              <span className="tab-warn" aria-label="needs fixing">
                !
              </span>
            )}
          </button>
          <button role="tab" aria-selected={part === "look"} className={part === "look" ? "on" : ""} onClick={() => setPart("look")}>
            How it looks
          </button>
        </div>

        {part === "about" ? (
          <div className="edit-about">
            <label className="status-text">
              Pronouns
              <input value={pronouns} maxLength={32} disabled={!loaded} onChange={(e) => setPronouns(e.target.value)} placeholder="they/them" />
            </label>
            <label className="status-text">
              Bio
              <textarea value={bio} maxLength={512} rows={7} disabled={!loaded} onChange={(e) => setBio(e.target.value)} placeholder="A few lines about you" />
              <small>{512 - bio.length} left</small>
            </label>
            <fieldset className="edit-links" disabled={!loaded}>
              <legend>Links, up to three</legend>
              {links.map((l, i) => (
                <input key={i} value={l} maxLength={200} aria-label={`Link ${i + 1}`} onChange={(e) => setLinks(links.map((x, at) => (at === i ? e.target.value : x)))} placeholder="https://" />
              ))}
              {bad && <small className="rule-warn">A link starts with https:// and has no spaces.</small>}
            </fieldset>
            <fieldset className="edit-langs" disabled={!loaded}>
              <legend>Languages you speak, up to three</legend>
              <div className="edit-lang-chips">
                {langs.length === 0 && <span className="send-fine">None picked.</span>}
                {langs.map((code) => (
                  <span className="edit-lang" key={code}>
                    <span className="hide-name">{languageNames[code] ?? code.toUpperCase()}</span>
                    <button aria-label={`Take ${languageNames[code] ?? code} off`} onClick={() => setLangs(langs.filter((x) => x !== code))}>
                      ×
                    </button>
                  </span>
                ))}
              </div>
              <select
                value=""
                disabled={langs.length >= 3}
                aria-label="Add a language"
                onChange={(e) => e.target.value && setLangs([...langs, e.target.value].slice(0, 3))}
              >
                <option value="">{langs.length >= 3 ? "Three is VRChat's limit" : "Add a language"}</option>
                {Object.entries(languageNames)
                  .filter(([code]) => !langs.includes(code))
                  .sort((a, b) => a[1].localeCompare(b[1]))
                  .map(([code, label]) => (
                    <option key={code} value={code}>
                      {label}
                    </option>
                  ))}
              </select>
            </fieldset>
          </div>
        ) : (
          <fieldset className="edit-look" disabled={!loaded}>
            <div
              className={`look-preview${backdrop ? " backed" : ""}`}
              style={backdrop ? ({ "--bg": backdrop, "--tint-strength": String(live().profileBackdropStrength) } as CSSProperties) : undefined}
              aria-label="How your profile will look"
            >
              <div className="look-banner" style={banner ? { backgroundImage: `url("${banner}")` } : undefined} />
              <div className="look-icon" style={icon ? { backgroundImage: `url("${icon}")` } : undefined}>
                {framePic && <span className="look-frame" style={{ backgroundImage: `url("${framePic}")` }} />}
              </div>
              {/* Over the whole card from the top, the way VRChat shows it. */}
              {effectPic && <span className="look-effect" style={{ backgroundImage: `url("${effectPic}")` }} />}
              <div className={`look-name${plateBg ? " look-plate" : ""}`} style={plateBg ? { backgroundImage: plateBg } : undefined}>
                <b className="hide-name">{name || "You"}</b>
              </div>
            </div>
            {owned === null && <p className="send-fine">Asking VRChat which frames and effects you own.</p>}
            {noneOwned && <p className="send-fine">You don't own any frames, effects or nameplates yet. VRChat gives them out in its shop and at events.</p>}
            {(
              [
                ["iconFrame", "Icon frame", "Around your picture"],
                ["profileEffect", "Profile effect", "Over your whole profile"],
                ["nameplateEffect", "Nameplate", "Behind your name"],
              ] as const
            ).map(([kind, label, where]) => (
              <section className="look-row" key={kind}>
                <h3>
                  {label} <small>{where}</small>
                </h3>
                <div className="look-items">
                  <button className={look[kind] === "" ? "on" : ""} aria-pressed={look[kind] === ""} onClick={() => set({ [kind]: "" })}>
                    <span className="look-none">None</span>
                  </button>
                  {pickable(kind).map((id) => {
                    const item = dress[id];
                    return (
                      <button key={id} className={look[kind] === id ? "on" : ""} aria-pressed={look[kind] === id} title={item?.name || id} onClick={() => set({ [kind]: id })}>
                        <span
                          // An effect is mostly see-through with a glow at its edges, so it sits on a banner of its own.
                          className={`look-thumb${kind === "profileEffect" ? " look-thumb-fx" : ""}`}
                          style={
                            item?.still || item?.picture
                              ? { backgroundImage: kind === "profileEffect" ? `url("${item.still || item.picture}"), ${FX_BACKDROP}` : `url("${item.still || item.picture}")` }
                              : item && colourOf(item.from) && colourOf(item.to)
                                ? { backgroundImage: `linear-gradient(90deg, ${colourOf(item.from)}, ${colourOf(item.to)})` }
                                : undefined
                          }
                        />
                        <small>{item?.name || "Unnamed"}</small>
                      </button>
                    );
                  })}
                </div>
                {owned !== null && !noneOwned && pickable(kind).length === 0 && <p className="send-fine">You own none of these yet.</p>}
              </section>
            ))}
            <section className="look-row">
              <h3>
                Background <small>Behind the whole profile</small>
              </h3>
              <div className="choices look-bg-kind" role="radiogroup" aria-label="Background">
                {(
                  [
                    ["default", "Plain"],
                    ["gradient", "Two colours"],
                    ["texture", "Picture"],
                  ] as const
                ).map(([value, label]) => (
                  <button key={value} role="radio" aria-checked={look.backgroundType === value} className={look.backgroundType === value ? "on" : ""} onClick={() => set({ backgroundType: value })}>
                    {label}
                  </button>
                ))}
              </div>
              {look.backgroundType === "inventory" && <p className="send-fine">Your profile uses a background from VRChat's shop. Pick Plain, Two colours or Picture to replace it, or change it in VRChat.</p>}
              {look.backgroundType === "gradient" && (
                <div className="look-colours">
                  <label>
                    Top
                    <input type="color" value={`#${look.top}`} onChange={(e) => set({ top: e.target.value.slice(1) })} />
                  </label>
                  <label>
                    Bottom
                    <input type="color" value={`#${look.bottom}`} onChange={(e) => set({ bottom: e.target.value.slice(1) })} />
                  </label>
                </div>
              )}
              {look.backgroundType === "texture" && (
                <div className="look-textures">
                  {textures.map(([id]) => (
                    <button key={id} className={look.texture === id ? "on" : ""} aria-pressed={look.texture === id} title={textureName(id)} onClick={() => set({ texture: id })}>
                      <TextureThumb id={id} />
                      <small>{textureName(id)}</small>
                    </button>
                  ))}
                </div>
              )}
              {look.backgroundType === "texture" && <p className="send-fine">Some pictures need VRChat+. If VRChat refuses one, its reason shows below.</p>}
            </section>
          </fieldset>
        )}

        {problem && (
          <p className="banner" role="alert">
            {problem}
          </p>
        )}
        {bad && part === "look" && (
          <p className="rule-warn" role="alert">
            A link on About me needs fixing before you can save.{" "}
            <button className="linkish" onClick={() => setPart("about")}>
              Show it
            </button>
          </p>
        )}
        {leaving ? (
          <div className="dialog-actions edit-leave" role="group" aria-label="Leave without saving?">
            <span>You have changes that are not saved. Leave without saving?</span>
            <button ref={keepButton} onClick={() => setLeaving("")}>
              Keep editing
            </button>
            <button className="go" onClick={leaving === "back" && onBack ? onBack : onClose}>
              Leave
            </button>
          </div>
        ) : (
          <div className="dialog-actions">
            <button onClick={tryClose}>Cancel</button>
            <button className="go" disabled={busy || !loaded || !!bad} title={bad ? "Fix the link on About me first" : ""} onClick={() => void save()}>
              {busy ? "Saving" : "Save"}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

/**
 * One background picture in the picker. VRChat's files are full size, so a picture is only fetched
 * once its place in the scrolling grid comes into sight.
 */
function TextureThumb({ id }: { id: string }) {
  const ref = useRef<HTMLImageElement>(null);
  const [seen, setSeen] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el || seen) return;
    const watch = new IntersectionObserver(
      (seenNow) => {
        if (!seenNow.some((e) => e.isIntersecting)) return;
        setSeen(true);
        watch.disconnect();
      },
      { root: el.closest(".look-textures"), rootMargin: "60px 0px" },
    );
    watch.observe(el);
    return () => watch.disconnect();
  }, [seen]);
  return <img ref={ref} className="look-thumb" src={seen ? textureUrl(id) : undefined} alt="" decoding="async" />;
}
