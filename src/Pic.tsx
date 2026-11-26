import { useEffect, useState, type ImgHTMLAttributes } from "react";
import { backend } from "./backend";

// Some of VRChat's pictures only load for a signed-in account. The window itself is not
// signed in, so when a picture fails the app fetches it and hands it over as data.
const fetched = new Map<string, Promise<string>>();

export function signedPicture(url: string): Promise<string> {
  let waiting = fetched.get(url);
  if (!waiting) {
    waiting = backend.loadPicture(url);
    fetched.set(url, waiting);
    // A fetch that failed is forgotten, so the next look at the picture tries again.
    waiting.catch(() => fetched.delete(url));
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
