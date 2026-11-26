import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@fontsource/yellowtail";
import "@fontsource/barlow-condensed/600-italic.css";
import "@fontsource/barlow-condensed/700-italic.css";
import "@fontsource-variable/instrument-sans";
// Fonts offered in Settings. A browser only loads the files of a font that is in use.
import "@fontsource/rajdhani/latin-500.css";
import "@fontsource/rajdhani/latin-600.css";
import "@fontsource/rajdhani/latin-700.css";
import "@fontsource-variable/orbitron";
import "@fontsource-variable/outfit";
import "@fontsource-variable/space-grotesk";
import "./styles.css";
import { App } from "./App";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
