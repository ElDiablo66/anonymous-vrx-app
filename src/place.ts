import type { Place } from "./backend";

/** VRChat's own page for joining an instance. Empty for anything that is not an instance. */
export function joinLink(location: string): string {
  const cut = location.indexOf(":");
  if (cut < 0 || !location.startsWith("wrld_")) return "";
  return `https://vrchat.com/home/launch?worldId=${location.slice(0, cut)}&instanceId=${encodeURIComponent(location.slice(cut + 1))}`;
}

const nowhere: Place = { kind: "none", worldId: null, instance: null, access: null, region: null, groupId: null };

/** Reads a location string such as `wrld_x:123~hidden(usr_y)~region(eu)`. Same rules as the backend. */
export function parsePlace(location: string): Place {
  if (location === "private") return { ...nowhere, kind: "private" };
  if (location === "traveling") return { ...nowhere, kind: "traveling" };
  if (!location.startsWith("wrld_") && !location.startsWith("wld_")) return nowhere;
  const [worldId, rest] = location.split(/:(.*)/s);
  const place: Place = { ...nowhere, kind: "world", worldId };
  if (!rest) return place;
  const parts = rest.split("~");
  place.instance = parts.shift() ?? null;
  let access = "public";
  let canRequest = false;
  let groupAccess = "";
  for (const part of parts) {
    const [key, value = ""] = part.replace(/\)$/, "").split("(");
    if (key === "hidden") access = "friendsPlus";
    else if (key === "friends") access = "friends";
    else if (key === "private") access = "invite";
    else if (key === "canRequestInvite") canRequest = true;
    else if (key === "group") {
      access = "group";
      place.groupId = value || null;
    } else if (key === "groupAccessType") groupAccess = value;
    else if (key === "region") place.region = value || null;
  }
  if (access === "invite" && canRequest) access = "invitePlus";
  if (access === "group" && groupAccess === "plus") access = "groupPlus";
  if (access === "group" && groupAccess === "public") access = "groupPublic";
  place.access = access;
  return place;
}
