import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";
import { disableTextFixing } from "./lib/textInput";
import { markPlatform } from "./lib/platform";

markPlatform();
disableTextFixing();

if (import.meta.env.VITE_DEMO) {
  (await import("./demo/mock")).install();
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
