import { useAppStore } from "./stores/app-store";
import { Sidebar } from "./components/sidebar";
import { StartupScreen } from "./components/startup-dialog";
import { UpdateBanner } from "./components/update-banner";
import { P4kBrowser } from "./views/p4k-browser";
import { DataCoreBrowser } from "./views/datacore-browser";
import { ExportView } from "./views/export-view";
import { AudioView } from "./views/audio-view";

function App() {
  const mode = useAppStore((s) => s.mode);
  const hasData = useAppStore((s) => s.hasData);

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
