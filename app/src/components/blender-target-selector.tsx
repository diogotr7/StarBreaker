import { useEffect, useState } from "react";
import {
  listBlenderAddonTargets,
  type BlenderAddonTarget,
  type BlenderAddonTargets,
} from "../lib/commands";

interface BlenderTargetSelectorProps {
  onTargetSelected: (addonsPath: string) => void;
  onUninstallRequested: (addonsPath: string) => void;
}

export function BlenderTargetSelector({
  onTargetSelected,
  onUninstallRequested,
}: BlenderTargetSelectorProps) {
  const [data, setData] = useState<BlenderAddonTargets | null>(null);
  const [scanning, setScanning] = useState(true);
  const [scanError, setScanError] = useState<string | null>(null);

  useEffect(() => {
    listBlenderAddonTargets()
      .then((result) => {
        setData(result);
        setScanError(null);
      })
      .catch((err) => setScanError(String(err)))
      .finally(() => setScanning(false));
  }, []);

  const handleBrowseCustom = async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({
      title: "Select the Blender 'scripts/addons' directory",
      directory: true,
      multiple: false,
    });
    if (typeof picked === "string") {
      onTargetSelected(picked);
    }
  };

  const getStatusBadge = (target: BlenderAddonTarget) => {
    switch (target.state) {
      case "install":
        return (
          <span className="px-2.5 py-1 rounded-md text-[11px] font-medium bg-accent text-on-accent">
            Install
          </span>
        );
      case "upgrade":
        return (
          <span className="px-2.5 py-1 rounded-md text-[11px] font-medium bg-accent text-on-accent">
            Update
          </span>
        );
      case "installed":
        return (
          <span className="px-2.5 py-1 rounded-md text-[11px] font-medium border border-success/40 text-success">
            ✓ Current
          </span>
        );
    }
  };

  if (scanning) {
    return (
      <div className="p-4 text-center">
        <p className="text-text-dim text-sm">Scanning for Blender installations...</p>
      </div>
    );
  }

  if (scanError) {
    return (
      <div className="p-4">
        <p className="text-danger text-sm mb-3">Error scanning for Blender: {scanError}</p>
        <button
          onClick={handleBrowseCustom}
          className="w-full px-3 py-2 bg-accent text-on-accent rounded-md text-sm hover:brightness-110 transition-colors"
        >
          Browse for addons folder...
        </button>
      </div>
    );
  }

  const targets = data?.targets ?? [];
  const running = data?.blender_running ?? false;
  const hasIncompatible = data?.incompatible_blender_found ?? false;

  return (
    <div className="flex flex-col gap-3">
      {running && (
        <div className="rounded-md border border-warning/30 bg-warning/10 px-3 py-2">
          <p className="text-[11px] text-warning leading-relaxed">
            ⚠ Blender appears to be running. Restart Blender after install to load the new version.
          </p>
        </div>
      )}

      {targets.length > 0 && (
        <>
          <p className="text-xs text-text-sub font-medium">Detected Installations</p>
          {targets.map((target, idx) => {
            const isInstalled =
              target.state === "installed" || target.state === "upgrade";
            return (
              <div
                key={idx}
                className="flex items-stretch gap-2 rounded-md border border-border hover:border-accent/40 transition-colors bg-surface overflow-hidden"
              >
                <button
                  onClick={() => onTargetSelected(target.addons_path)}
                  className="flex items-center justify-between gap-3 flex-1 min-w-0 p-3 hover:bg-surface-hi transition-colors text-left cursor-pointer"
                >
                  <div className="min-w-0 flex-1">
                    <p className="text-sm text-text font-medium">
                      Blender {target.blender_version}
                    </p>
                    <p className="text-xs text-text-dim truncate mt-0.5">
                      {target.addons_path}
                    </p>
                    {target.installed_version && (
                      <p className="text-xs text-text-faint mt-1">
                        Addon: v{target.installed_version}
                      </p>
                    )}
                  </div>
                  <div className="shrink-0">{getStatusBadge(target)}</div>
                </button>
                {isInstalled && (
                  <button
                    onClick={() => onUninstallRequested(target.addons_path)}
                    title="Uninstall addon from this Blender"
                    aria-label="Uninstall addon"
                    className="shrink-0 px-3 flex items-center text-text-dim hover:text-danger hover:bg-danger/10 transition-colors cursor-pointer border-l border-border"
                  >
                    <svg
                      width="14"
                      height="14"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    >
                      <path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                      <line x1="10" y1="11" x2="10" y2="17" />
                      <line x1="14" y1="11" x2="14" y2="17" />
                    </svg>
                  </button>
                )}
              </div>
            );
          })}
        </>
      )}

      {targets.length === 0 && (
        <p className="text-sm text-text-dim text-center py-3">
          {hasIncompatible
            ? "Blender found but requires 5.0 or newer."
            : "No Blender installations detected"}
        </p>
      )}

      <button
        onClick={handleBrowseCustom}
        className="px-3 py-2 border border-dashed border-surface-hi rounded-md text-sm text-text-sub hover:text-text hover:border-accent/50 hover:bg-accent/5 transition-colors cursor-pointer"
      >
        Browse for addons folder...
      </button>
    </div>
  );
}
