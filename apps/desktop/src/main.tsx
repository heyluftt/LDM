import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import { applyAppTheme } from "./shared/appearancePreferences";
import "./styles.css";

applyAppTheme();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
