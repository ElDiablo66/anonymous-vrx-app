import { accessName, regionName } from "./bits";
import type { DiscordPresence, HereNow, World } from "./backend";
import { joinLink } from "./place";
import type { Settings } from "./settings";

/** Instances only invited people can enter. Their world is not named unless I say so. */
const closed = new Set(["invite", "invitePlus"]);
/** Instances a Discord friend could walk into from a link. */
const open = new Set(["public", "friendsPlus", "groupPublic"]);

const counted = ["", "", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"];

/** How many are there, said plainly or with some neon. */
function crowd(count: number, night: boolean): string {
  if (count <= 0) return "";
  if (!night) return count === 1 ? "alone" : `${count} people`;
  if (count === 1) return "riding solo";
  return `${counted[count] ?? count} deep`;
}

/**
 * What Discord should show for where I am now, from my own choices in Settings.
 * Nothing when the feature is off, no application id is set, or I am not in a world.
 */
export function discordPresence(settings: Settings, here: HereNow | null, worlds: Map<string, World>): DiscordPresence | null {
  if (!settings.discordOn || !/^\d{17,20}$/.test(settings.discordAppId) || !here) return null;
  const { visit, people } = here;
  const night = settings.discordStyle === "night";
  const access = visit.place.access ?? "";
  const mode = closed.has(access) ? settings.discordPrivateMode : "show";
  // In a private world with "show nothing", Discord gets no activity at all.
  if (mode === "none") return null;
  const hush = mode === "hush";
  const world = visit.place.worldId ? worlds.get(visit.place.worldId) : undefined;
  const name = visit.worldName || world?.name || "a world";

  const presence: DiscordPresence = {
    clientId: settings.discordAppId,
    details: night ? "Out in VRChat" : "In VRChat",
    state: "",
    startedAt: Math.floor(new Date(visit.joinedAt).getTime() / 1000) || null,
    image: "",
    imageText: "",
    buttonLabel: "",
    buttonUrl: "",
    button2Label: "",
    button2Url: "",
    smallImage: "logo",
    smallText: "Anonymous VRX",
  };
  if (hush) {
    presence.details = night ? "Off the grid" : "In a private world";
    presence.state = night ? "Invite only tonight" : "";
    return presence;
  }
  if (settings.discordWorld) {
    presence.details = night ? `Out in ${name}` : name;
    if (world?.image.startsWith("https://")) {
      presence.image = world.image;
      // Hovering the picture names the world and where its servers are.
      presence.imageText = [name, regionName(visit.place)].filter(Boolean).join(", ");
    }
  }
  const parts: string[] = [];
  if (settings.discordAccess && accessName(visit.place)) parts.push(accessName(visit.place));
  if (settings.discordPeople) {
    const words = crowd(people.filter((p) => !p.leftAt).length, night);
    if (words) parts.push(words);
  }
  presence.state = parts.join(", ");

  const buttons: Array<[string, string]> = [];
  if (settings.discordJoin && settings.discordWorld && open.has(access)) {
    buttons.push([night ? "Pull up" : "Join", joinLink(visit.location)]);
  }
  if (settings.discordWorldPage && settings.discordWorld && visit.place.worldId) {
    buttons.push([night ? "See the spot" : "World page", `https://vrchat.com/home/world/${visit.place.worldId}`]);
  }
  [presence.buttonLabel, presence.buttonUrl] = buttons[0] ?? ["", ""];
  [presence.button2Label, presence.button2Url] = buttons[1] ?? ["", ""];
  return presence;
}
