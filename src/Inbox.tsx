import { useEffect, useState } from "react";
import { backend, errorText, type Friend, type Handled, type Notice } from "./backend";
import { Avatar, accessName, clock, dayLabel, tidy } from "./bits";
import { joinLink, parsePlace } from "./place";
import { launched } from "./History";
import { savedLaunchMode } from "./settings";

export function noticeLine(n: Notice, meId = ""): { title: string; detail: string } {
  const who = n.senderName || "Someone";
  const mine = meId !== "" && n.senderId === meId;
  const where = n.worldName ? `${tidy(n.worldName)}${n.location ? `, ${accessName(parsePlace(n.location))}` : ""}` : "";
  switch (n.kind) {
    case "friendRequest":
      return { title: `${who} wants to be friends`, detail: "" };
    case "invite":
      return mine
        ? { title: `Your own invite${where ? ` to ${where}` : ""}`, detail: "Accept it inside VRChat to go there. It is safe to dismiss here." }
        : { title: `${who} invited you${where ? ` to ${where}` : ""}`, detail: tidy(n.message) };
    case "requestInvite":
      return { title: `${who} asks for an invite`, detail: tidy(n.message) };
    case "inviteResponse":
      return { title: `${who} answered your invite`, detail: n.message };
    case "requestInviteResponse":
      return { title: `${who} answered your invite request`, detail: n.message };
    case "boop":
      return { title: `${who} booped you`, detail: "" };
    case "votetokick":
      return { title: "Vote to kick", detail: n.message };
    default:
      return { title: n.title || (n.senderName ? `From ${n.senderName}` : "Notification"), detail: n.message };
  }
}

const inboxGroups: Array<[string, string]> = [
  ["all", "Everything"],
  ["boops", "Boops"],
  ["invites", "Invites"],
  ["requests", "Invite requests"],
  ["friends", "Friend requests"],
  ["groups", "Groups and other"],
];

function groupOf(n: Notice): string {
  if (n.kind === "boop") return "boops";
  if (n.kind === "invite" || n.kind === "inviteResponse" || n.kind === "requestInviteResponse") return "invites";
  if (n.kind === "requestInvite") return "requests";
  if (n.kind === "friendRequest") return "friends";
  return "groups";
}

function outcomeWords(h: Handled): string {
  if (h.action === "accept") return "You accepted";
  if (h.action === "inviteBack") return "You sent an invite";
  if (h.action === "boopBack") return "You booped back";
  if (h.action === "elsewhere") return "Seen in VRChat";
  if (h.action === "reply") return `You chose: ${h.replyText || "a reply"}`;
  return h.notice.kind === "friendRequest" ? "You declined" : "You dismissed it";
}

function outcomeTone(h: Handled): string {
  if (h.action === "accept" || h.action === "inviteBack" || h.action === "boopBack") return "yes";
  if (h.action === "reply") return /accept|join/i.test(h.replyText) ? "yes" : /decline|block|unsubscribe/i.test(h.replyText) ? "no" : "plain";
  return h.notice.kind === "friendRequest" ? "no" : "plain";
}

function doneText(n: Notice, action: string, replyText: string): string {
  const who = n.senderName || n.title || "them";
  if (action === "accept") return `Accepted the friend request from ${who}.`;
  if (action === "inviteBack") return `Invite sent to ${who}.`;
  if (action === "boopBack") return `Booped ${who} back.`;
  if (action === "reply") return `Done: ${replyText}${n.title ? ` (${n.title})` : ""}.`;
  if (n.kind === "friendRequest") return `Declined the friend request from ${who}.`;
  return "Dismissed.";
}

export function Inbox({
  notices,
  friends,
  inWorld,
  meId,
  onDone,
}: {
  notices: Notice[];
  friends: Map<string, Friend>;
  inWorld: boolean;
  meId: string;
  onDone: (message: string) => void;
}) {
  const [busy, setBusy] = useState("");
  const [problem, setProblem] = useState("");
  const [tab, setTab] = useState<"waiting" | "done">("waiting");
  const [handled, setHandled] = useState<Handled[]>([]);
  const [kind, setKind] = useState("all");

  // The Done list is re-read whenever the waiting list changes, which is right after every action.
  useEffect(() => {
    let alive = true;
    void backend
      .handled(200)
      .then((list) => alive && setHandled(list))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [notices]);

  async function act(n: Notice, action: string, replyKind?: string, replyData?: string, replyText = "") {
    setBusy(n.id);
    setProblem("");
    try {
      await backend.noticeAction(n.id, action, replyKind, replyData);
      onDone(doneText(n, action, replyText));
    } catch (err) {
      setProblem(errorText(err));
    } finally {
      setBusy("");
    }
  }

  /** Goes to the instance an invite points at. The invite stays in the list until it is dismissed. */
  async function join(n: Notice) {
    setBusy(n.id);
    setProblem("");
    try {
      onDone(launched[await backend.launchInstance(n.location, savedLaunchMode())]);
    } catch (err) {
      setProblem(errorText(err));
    } finally {
      setBusy("");
    }
  }

  const tabs = (
    <div className="tabs" role="tablist" aria-label="Inbox">
      <button role="tab" aria-selected={tab === "waiting"} className={tab === "waiting" ? "on" : ""} onClick={() => setTab("waiting")}>
        Waiting{notices.length > 0 ? `, ${notices.length}` : ""}
      </button>
      <button role="tab" aria-selected={tab === "done"} className={tab === "done" ? "on" : ""} onClick={() => setTab("done")}>
        Done{handled.length > 0 ? `, ${handled.length}` : ""}
      </button>
    </div>
  );

  // Kind filters show only when there is more than one kind to tell apart.
  const source = tab === "done" ? handled.map((h) => h.notice) : notices;
  const counts = new Map<string, number>();
  for (const n of source) counts.set(groupOf(n), (counts.get(groupOf(n)) ?? 0) + 1);
  const active = kind !== "all" && !counts.has(kind) ? "all" : kind;
  const chips =
    counts.size > 1 ? (
      <div className="filters filters-flat" role="group" aria-label="Kinds of notification">
        {inboxGroups
          .filter(([id]) => id === "all" || counts.has(id))
          .map(([id, label]) => (
            <button key={id} className={id === active ? "on" : ""} aria-pressed={id === active} onClick={() => setKind(id)}>
              {label}
              {id !== "all" && <span className="chip-count">{counts.get(id)}</span>}
            </button>
          ))}
      </div>
    ) : null;
  const shownNotices = notices.filter((n) => active === "all" || groupOf(n) === active);
  const shownHandled = handled.filter((h) => active === "all" || groupOf(h.notice) === active);

  if (tab === "done") {
    let lastDoneDay = "";
    return (
      <div className="table-wrap">
        {tabs}
        {chips}
        {handled.length === 0 ? (
          <p className="empty">
            Nothing yet. Whatever you accept, decline or dismiss in Waiting is listed here afterwards, with what
            you chose and when.
          </p>
        ) : (
          <ol className="inbox">
            {shownHandled.map((h) => {
              const day = dayLabel(h.at);
              const header = day !== lastDoneDay ? <li className="day" key={`day-${h.id}`}>{day}</li> : null;
              lastDoneDay = day;
              const { title, detail } = noticeLine(h.notice, meId);
              return [
                header,
                <li className="notice notice-done" key={h.id}>
                  <Avatar name={h.notice.senderName || title} image={friends.get(h.notice.senderId)?.image || h.notice.image} />
                  <div className="notice-text">
                    <p className="notice-title">{title}</p>
                    {detail && <p className="notice-detail">{detail}</p>}
                    <time dateTime={h.at}>{clock(h.at)}</time>
                  </div>
                  <span className={`outcome outcome-${outcomeTone(h)}`}>{outcomeWords(h)}</span>
                </li>,
              ];
            })}
          </ol>
        )}
      </div>
    );
  }

  if (notices.length === 0) {
    return (
      <div className="table-wrap">
        {tabs}
        <p className="empty">
          Nothing waiting. Friend requests, invites, invite requests and group notifications arrive here as soon as
          VRChat sends them.
        </p>
      </div>
    );
  }

  let lastDay = "";
  return (
    <div className="table-wrap">
      {tabs}
      {chips}
      {problem && (
        <p className="banner" role="alert">
          {problem} <button onClick={() => setProblem("")}>Dismiss</button>
        </p>
      )}
      <ol className="inbox">
        {shownNotices.map((n) => {
          const day = n.at ? dayLabel(n.at) : "";
          const header = day && day !== lastDay ? <li className="day" key={`day-${n.id}`}>{day}</li> : null;
          if (day) lastDay = day;
          const { title, detail } = noticeLine(n, meId);
          const off = busy === n.id;
          return [
            header,
            <li className={`notice${n.seen ? "" : " notice-new"}`} key={n.id}>
              <Avatar name={n.senderName || title} image={friends.get(n.senderId)?.image || n.image} />
              <div className="notice-text">
                <p className="notice-title">{title}</p>
                {detail && <p className="notice-detail">{detail}</p>}
                {n.at && <time dateTime={n.at}>{clock(n.at)}</time>}
              </div>
              <div className="notice-actions">
                {n.kind === "friendRequest" && (
                  <>
                    <button className="go" disabled={off} onClick={() => void act(n, "accept")}>Accept</button>
                    <button disabled={off} onClick={() => void act(n, "dismiss")}>Decline</button>
                  </>
                )}
                {n.kind === "requestInvite" && (
                  <>
                    <button
                      className="go"
                      disabled={off || !inWorld}
                      title={inWorld ? "Sends them an invite to the world you are in" : "You are not in a world right now"}
                      onClick={() => void act(n, "inviteBack")}
                    >
                      Invite them here
                    </button>
                    <button disabled={off} onClick={() => void act(n, "dismiss")}>Dismiss</button>
                  </>
                )}
                {n.kind === "invite" && joinLink(n.location) && (
                  <button
                    className="go"
                    disabled={off}
                    title="Starts VRChat in the instance you were invited to. If the game is already open, it moves you there."
                    onClick={() => void join(n)}
                  >
                    Join in VRChat
                  </button>
                )}
                {n.kind === "boop" && n.senderId && (
                  <button className="go" disabled={off} onClick={() => void act(n, "boopBack")}>
                    Boop back
                  </button>
                )}
                {n.version === 2 &&
                  n.kind !== "boop" &&
                  n.replies.map((r) => (
                    <button key={r.kind + r.data} className={r.kind === "accept" ? "go" : ""} disabled={off} onClick={() => void act(n, "reply", r.kind, r.data, r.text || r.kind)}>
                      {r.text || r.kind}
                    </button>
                  ))}
                {n.kind !== "friendRequest" && n.kind !== "requestInvite" && (n.kind === "boop" || !(n.version === 2 && n.replies.length > 0)) && (
                  <button disabled={off} onClick={() => void act(n, "dismiss")}>Dismiss</button>
                )}
              </div>
            </li>,
          ];
        })}
      </ol>
    </div>
  );
}
