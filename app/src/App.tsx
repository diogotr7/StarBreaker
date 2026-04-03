import { useEffect } from "react";
import { useAppStore } from "./stores/app-store";
import { Sidebar } from "./components/sidebar";
import { StartupScreen } from "./components/startup-dialog";
import { UpdateBanner } from "./components/update-banner";
import { P4kBrowser } from "./views/p4k-browser";
import { DataCoreBrowser } from "./views/datacore-browser";
import { ExportView } from "./views/export-view";
import { AudioView } from "./views/audio-view";
import { getSystemTheme, onSystemThemeChanged } from "./lib/commands";
import { applySystemTheme } from "./lib/theme";

function App() {
  const mode = useAppStore((s) => s.mode);
  const hasData = useAppStore((s) => s.hasData);

  // Apply OS system theme on mount and react to changes.
  useEffect(() => {
    getSystemTheme().then(applySystemTheme);
    const unlisten = onSystemThemeChanged(applySystemTheme);
    return () => { unlisten.then((f) => f()); };
  }, []);

  if (!hasData) {
    return <StartupScreen />;
  }

  return (
    <div className="flex flex-col w-full h-full">
      <UpdateBanner />
      <div className="flex flex-1 overflow-hidden">
        <Sidebar />
        <main className="flex-1 flex flex-col overflow-hidden">
          {mode === "p4k" && <P4kBrowser />}
          {mode === "datacore" && <DataCoreBrowser />}
          {mode === "export" && <ExportView />}
          {mode === "audio" && <AudioView />}
        </main>
      </div>
    </div>
  );
}

export default App;
