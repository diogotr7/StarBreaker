import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import {
  debug as logDebug,
  error as logError,
  info as logInfo,
  trace as logTrace,
  warn as logWarn,
} from "@tauri-apps/plugin-log";
import App from "./App";
import "./globals.css";

// Forward browser console.* into the Rust log (and on to app.log + stdout).
// We deliberately do NOT call attachConsole(): it would route Rust logs back
// into console, which forwardConsole would re-capture, creating an infinite
// loop. View Rust logs via the terminal (`tauri dev` stdout) or the log
// file at %LOCALAPPDATA%\app.starbreaker\logs\StarBreaker.log.
// Mirror to the original console so DevTools keeps working unchanged.
type LogFn = (msg: string) => Promise<void>;
function forwardConsole(fnName: "log" | "debug" | "info" | "warn" | "error", logger: LogFn) {
  const original = console[fnName].bind(console);
  console[fnName] = (...args: unknown[]) => {
    original(...args);
    const msg = args
      .map((a) => (typeof a === "string" ? a : (() => { try { return JSON.stringify(a); } catch { return String(a); } })()))
      .join(" ");
    void logger(msg).catch(() => {});
  };
}
forwardConsole("log", logTrace);
forwardConsole("debug", logDebug);
forwardConsole("info", logInfo);
forwardConsole("warn", logWarn);
forwardConsole("error", logError);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
