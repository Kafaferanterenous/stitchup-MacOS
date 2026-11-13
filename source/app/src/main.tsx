import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./themes/schemes.css";
import { restoreTheme } from "./themes/theme";
import "./styles/global.css";

void restoreTheme();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);