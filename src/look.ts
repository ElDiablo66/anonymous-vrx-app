import { useEffect, useState } from "react";
import { backend, type Cosmetic } from "./backend";

/** The frames, effects, plates and backgrounds VRChat offers, asked for once while the app runs. */
let dressKept: Promise<Record<string, Cosmetic>> | null = null;

/** The frames and effects by id, or nothing while they load or when they are switched off. */
export function useDress(on: boolean): Record<string, Cosmetic> {
  const [dress, setDress] = useState<Record<string, Cosmetic>>({});
  useEffect(() => {
    if (!on) return;
    dressKept ??= backend.cosmetics().catch(() => {
      // Asked again the next time a profile opens.
      dressKept = null;
      return {};
    });
    let alive = true;
    void dressKept.then((all) => alive && setDress(all));
    return () => {
      alive = false;
    };
  }, [on]);
  return on ? dress : {};
}

/** A colour as CSS can use it: "#aabbcc" from "#aabbcc" or "aabbcc"; empty for anything else. */
export function colourOf(value: unknown): string {
  const text = typeof value === "string" ? value.trim().replace(/^#/, "") : "";
  return /^[0-9a-f]{6}$/i.test(text) ? `#${text}` : "";
}

const base = "https://assets.vrchat.com/www/profile_decorations/profile_backgrounds/";

/** VRChat's picture backgrounds for profiles: the id a profile names, and the file on VRChat's server. */
export const textures: Array<[string, string]> = [
  ["grid", "BG_Grid.png"], ["filigree", "BG_Cascade.png"], ["bit-mountain", "BG_GRID_BITS.png"], ["approach", "BG_Approach.png"],
  ["planet-fall", "BG_PlanetFall.png"], ["jungle", "BG_Jungle.png"], ["light-streams", "BG_LightStream.png"], ["koi", "Koi_Layer.png"],
  ["moonlight", "BG_Cloud.png"], ["machine", "TechWallLayer.png"], ["topology", "TOPOLOGY.png"], ["circuit", "BG_Circuit.png"],
  ["kawaii-cupcakes", "BG_Kawaii_cupcakes.png"], ["bokeh", "BG_Bokeh.png"], ["hologram", "BG_Shatter.png"], ["flow", "BG_Flow_Horizontal.png"],
  ["jelly", "BG_Jelly.png"], ["cheese", "BG_Cheese.png"], ["alchemy", "BG_Alchemy.png"], ["sunset-mountain", "BG_MountainSun.png"],
  ["magic-rocks", "BG_MagicRocks.png"], ["city-stillness", "BG_FutureCityStillness.png"], ["music", "BG_Music.png"], ["wolf-running", "BG_WolfRunning.png"],
  ["dragon-smoke", "BG_DragonSmoke.png"], ["night-rain", "BG_NightRain.png"], ["butterfly-zero", "BG_ButterFlyZERO.png"], ["i-am-speed", "void.png"],
  ["ronin", "BG_Ronin.png"], ["abduction", "BG_Abduction.png"], ["cat-dream", "BG_CatDream.png"], ["virtual-waves", "BG_VirtualWaves.png"],
  ["bolt-and-snail", "BG_BoltNSnail.png"], ["escape", "BG_Escape.png"],
];

export function textureUrl(id: string): string {
  const file = textures.find(([key]) => key === id)?.[1];
  return file ? base + file : "";
}

/** A texture id written as words: "city-stillness" becomes "City stillness". */
export function textureName(id: string): string {
  const words = id.replace(/-/g, " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/**
 * The background a person chose for their profile, as CSS: a gradient of two colours, one of
 * VRChat's pictures, or a background from the shop. Empty when they kept the plain one.
 */
export function profileBackground(raw: Record<string, unknown>, dress: Record<string, Cosmetic>): { image: string; picture: string } {
  const kind = typeof raw.backgroundType === "string" ? raw.backgroundType : "";
  if (kind === "gradient") {
    const top = colourOf(raw.backgroundGradientTop);
    const bottom = colourOf(raw.backgroundGradientBottom);
    if (top && bottom) return { image: `linear-gradient(180deg, ${top}, ${bottom})`, picture: "" };
  }
  if (kind === "texture") {
    const url = textureUrl(typeof raw.backgroundTextureId === "string" ? raw.backgroundTextureId : "");
    if (url) return { image: "", picture: url };
  }
  if (kind === "inventory") {
    const id = typeof raw.backgroundTemplateId === "string" ? raw.backgroundTemplateId : "";
    const item = dress[id];
    if (item?.still || item?.picture) return { image: "", picture: item.still || item.picture };
  }
  return { image: "", picture: "" };
}
