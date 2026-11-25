import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@fontsource/yellowtail";
import "@fontsource/barlow-condensed/600-italic.css";
import "@fontsource/barlow-condensed/700-italic.css";
import "@fontsource-variable/instrument-sans";
import "./styles.css";
import { App } from "./App";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
