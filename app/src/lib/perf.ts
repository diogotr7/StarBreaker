/**
 * Load-phase performance instrumentation.
 *
 * Installs a PerformanceObserver that logs long tasks (>50ms) and slow
 * events (>200ms) to the console so they are captured by the existing
 * tauri-plugin-log / attachConsole pipeline and written to app.log.
 *
 * All output is prefixed with `[perf]` for easy grepping.
 *
 * Call `installPerfObserver()` once from main.tsx after `attachConsole()`.
 * Safe to call in any environment; unknown entryTypes are silently skipped.
 */
export function installPerfObserver(): void {
  // Chromium inside WebView2 supports "longtask"; Safari and older builds
  // may not. Wrap in try/catch so a missing entryType does not crash the app.
  try {
    const longTaskObserver = new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        const dur = entry.duration.toFixed(0);
        // Only log tasks that are long enough to be a real bottleneck.
        if (entry.duration >= 50) {
          console.warn(`[perf] longtask ${dur}ms name=${entry.name}`);
        }
      }
    });
    longTaskObserver.observe({ entryTypes: ["longtask"] });
  } catch {
    // entryType not supported in this WebView build -- skip silently.
  }

  try {
    const eventObserver = new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        if (entry.duration >= 200) {
          const dur = entry.duration.toFixed(0);
          console.warn(`[perf] slow-event ${dur}ms name=${entry.name}`);
        }
      }
    });
    eventObserver.observe({ entryTypes: ["event"] });
  } catch {
    // entryType not supported in this WebView build -- skip silently.
  }
}
