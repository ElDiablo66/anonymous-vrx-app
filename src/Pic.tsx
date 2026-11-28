import { useEffect, useState, type ImgHTMLAttributes } from "react";
import { backend } from "./backend";

// Some of VRChat's pictures only load for a signed-in account. The window itself is not
// signed in, so when a picture fails the app fetches it and hands it over as data.
const fetched = new Map<string, Promise<string>>();

/** Pictures whose fetch failed in a way asking again will not change, such as a picture that
 *  is gone or not allowed, with the error it gave. Kept for this session, so a long list that
 *  is opened again does not ask for each of them again. */
const refused = new Map<string, unknown>();

/** Whether a failed fetch will fail the same way when asked again. A slowdown, a network
 *  problem or a sign-in that is not done yet can pass, so those are asked again later; the
 *  app holds its own requests back during a slowdown. */
export function lastingFailure(err: unknown): boolean {
  if (!err || typeof err !== "object" || (err as { kind?: unknown }).kind !== "cannotDo") return false;
  const message = String((err as { message?: unknown }).message ?? "");
  if (message.includes("not a picture address") || message.includes("did not answer with a picture") || message.includes("too large")) return true;
  // "The picture could not be fetched (404)." A server that is busy or asks for a pause is not
  // lasting, and neither is a 401 or 403, which usually means the sign-in ran out.
  const status = Number(/\((\d{3})\)/.exec(message)?.[1]);
  return status >= 400 && status < 500 && ![401, 403, 408, 429].includes(status);
}

export function signedPicture(url: string): Promise<string> {
  if (refused.has(url)) return Promise.reject(refused.get(url));
  let waiting = fetched.get(url);
  if (!waiting) {
    waiting = backend.loadPicture(url);
    fetched.set(url, waiting);
    // A fetch that failed is forgotten, so the next look at the picture tries again, unless
    // asking again cannot help.
    waiting.catch((err: unknown) => {
      fetched.delete(url);
      if (!lastingFailure(err)) return;
      refused.set(url, err);
      if (refused.size > 1000) refused.delete(refused.keys().next().value as string);
    });
    if (fetched.size > 300) fetched.delete(fetched.keys().next().value as string);
  }
  return waiting;
}

/** An address that works for a background picture: the plain one, or the fetched one if that fails. */
export function usePic(url: string): string {
  const [shown, setShown] = useState(url);
  useEffect(() => {
    setShown(url);
    if (!url || url.startsWith("data:")) return;
    let alive = true;
    const probe = new Image();
    probe.referrerPolicy = "no-referrer";
    probe.onerror = () =>
      void signedPicture(url)
        .then((data) => alive && setShown(data))
        .catch(() => {});
    probe.src = url;
    return () => {
      alive = false;
    };
  }, [url]);
  return shown;
}

/** An `img` that falls back to the fetched picture when the plain address fails. */
export function SafeImg({ src, alt = "", onGiveUp, ...rest }: { src: string; onGiveUp?: () => void } & ImgHTMLAttributes<HTMLImageElement>) {
  const [shown, setShown] = useState(src);
  const [tried, setTried] = useState(false);
  useEffect(() => {
    setShown(src);
    setTried(false);
  }, [src]);
  return (
    <img
      {...rest}
      src={shown}
      alt={alt}
      referrerPolicy="no-referrer"
      onError={() => {
        // The fetched picture failing too means there is nothing to show.
        if (tried) return onGiveUp?.();
        setTried(true);
        void signedPicture(src)
          .then(setShown)
          .catch(() => onGiveUp?.());
      }}
    />
  );
}
