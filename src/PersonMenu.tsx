import { openSend } from "./Send";
import { useEffect, useState } from "react";
import { useEscape } from "./escape";
import { backend, errorText } from "./backend";

interface Target {
  id: string;
  name: string;
  x: number;
  y: number;
}

/**
 * The right-click menu on a friend, wherever they are listed. Anything on the page that carries
 * `data-person` (a user id) and `data-person-name` gets it.
 */
export function PersonMenu({
  isFriend,
  inWorld,
  openProfile,
  onDone,
}: {
  isFriend: (userId: string) => boolean;
  inWorld: boolean;
  openProfile: (userId: string) => void;
  onDone: (message: string) => void;
}) {
  const [target, setTarget] = useState<Target | null>(null);

  useEffect(() => {
    const open = (e: MouseEvent) => {
      const el = (e.target as HTMLElement | null)?.closest<HTMLElement>("[data-person]");
      const id = el?.dataset.person;
      if (!el || !id) return;
      e.preventDefault();
      // The page can be enlarged (Settings, Size). The pointer is measured in real pixels and
      // the menu is placed in page pixels, so the place is divided by that size.
      const size = parseFloat(document.documentElement.style.zoom) || 1;
      // Kept inside the window: the menu is about 230 by 190.
      const x = Math.max(4, Math.min(e.clientX / size, window.innerWidth / size - 240));
      const y = Math.max(4, Math.min(e.clientY / size, window.innerHeight / size - 200));
      setTarget({ id, name: el.dataset.personName || "them", x, y });
    };
    const close = () => setTarget(null);
    document.addEventListener("contextmenu", open);
    document.addEventListener("click", close);
    document.addEventListener("scroll", close, true);
    window.addEventListener("blur", close);
    return () => {
      document.removeEventListener("contextmenu", open);
      document.removeEventListener("click", close);
      document.removeEventListener("scroll", close, true);
      window.removeEventListener("blur", close);
    };
  }, []);
  useEscape(() => setTarget(null), target !== null);

  if (!target) return null;
  const { id, name } = target;
  const friend = isFriend(id);

  const run = (job: () => Promise<unknown>, done: string) => {
    setTarget(null);
    job().then(
      () => onDone(done),
      (err) => onDone(`That did not work: ${errorText(err)}`),
    );
  };

  return (
    <ul className="person-menu" role="menu" style={{ left: target.x, top: target.y }} onClick={(e) => e.stopPropagation()}>
      <li role="none">
        <button
          role="menuitem"
          onClick={() => {
            setTarget(null);
            openProfile(id);
          }}
        >
          Open profile
        </button>
      </li>
      <li role="none">
        <button
          role="menuitem"
          disabled={!friend}
          title={friend ? "Asks them to invite you to where they are. Their answer arrives in your Inbox" : "Only friends can be asked"}
          onClick={() => run(() => backend.requestInvite(id), `Asked ${name} for an invite. Their answer arrives in your Inbox.`)}
        >
          Ask for an invite
        </button>
      </li>
      <li role="none">
        <button
          role="menuitem"
          disabled={!inWorld}
          title={inWorld ? "Sends them an invite to the world you are in" : "You are not in a world right now"}
          onClick={() => run(() => backend.inviteHere(id), `Invited ${name} to where you are.`)}
        >
          Invite to my world
        </button>
      </li>
      <li role="none">
        <button
          role="menuitem"
          disabled={!friend}
          title={friend ? "Sends them a boop, plain or with an emoji" : "Only friends can be booped"}
          onClick={() => {
            setTarget(null);
            openSend({ kind: "boop", userId: id, name });
          }}
        >
          Boop
        </button>
      </li>
      <li role="none">
        <button
          role="menuitem"
          disabled={!friend}
          title={friend ? "An invite, or asking for one, with one of your saved lines" : "Only friends can be sent these"}
          onClick={() => {
            setTarget(null);
            openSend({ kind: "message", userId: id, name });
          }}
        >
          With a message
        </button>
      </li>
    </ul>
  );
}
