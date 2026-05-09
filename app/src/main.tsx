import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { attachConsole } from "@tauri-apps/plugin-log";
import App from "./App";
import "./globals.css";

// Route Rust log::info!/warn!/error! to browser devtools console.
attachConsole();

// Suppress the WebView's built-in context menu globally; our app uses its own
// <ContextMenu> for right-click actions. Components can still attach
// onContextMenu handlers to render the custom menu — preventing the default
// here only stops the native menu, not React event dispatch.
window.addEventListener("contextmenu", (e) => e.preventDefault());

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
