interface BlenderConfirmationDialogProps {
  mode: "install" | "uninstall";
  addonsPath: string;
  onConfirm: () => void;
  onCancel: () => void;
  busy: boolean;
}

export function BlenderConfirmationDialog({
  mode,
  addonsPath,
  onConfirm,
  onCancel,
  busy,
}: BlenderConfirmationDialogProps) {
  const sep = addonsPath.includes("\\") ? "\\" : "/";
  const trimmed = addonsPath.endsWith(sep) ? addonsPath.slice(0, -1) : addonsPath;
  const targetPath = `${trimmed}${sep}starbreaker_addon${sep}`;

  const isUninstall = mode === "uninstall";

  const title = isUninstall
    ? "Confirm Blender Addon Removal"
    : "Confirm Blender Addon Installation";
  const description = isUninstall
    ? "The StarBreaker addon will be removed from:"
    : "The StarBreaker addon will be installed to:";
  const busyLabel = isUninstall ? "Removing addon..." : "Installing addon...";
  const confirmLabel = isUninstall ? "Remove Addon" : "Confirm Installation";
  const confirmClass = isUninstall
    ? "bg-danger text-on-accent hover:brightness-110"
    : "bg-accent text-on-accent hover:brightness-110";

  return (
    <div className="fixed inset-0 bg-bg/80 backdrop-blur-sm flex items-center justify-center z-50">
      <div className="bg-bg-alt border border-border rounded-lg p-6 w-[480px] max-w-[90vw] shadow-lg">
        <h3 className="text-lg font-semibold text-text mb-4">{title}</h3>

        <div className="mb-6">
          <p className="text-sm text-text-sub mb-3">{description}</p>

          <div className="bg-surface rounded-md p-3 border border-border">
            <p className="text-xs font-mono text-text break-all">{targetPath}</p>
          </div>
        </div>

        {busy ? (
          <div className="flex justify-center">
            <div className="flex items-center gap-2">
              <div className="w-4 h-4 border-2 border-accent border-t-transparent rounded-full animate-spin"></div>
              <span className="text-sm text-text-dim">{busyLabel}</span>
            </div>
          </div>
        ) : (
          <div className="flex justify-end gap-3">
            <button
              onClick={onCancel}
              className="px-4 py-2 rounded-md text-sm font-medium bg-surface text-text-sub hover:bg-surface-hi transition-colors cursor-pointer"
            >
              Cancel
            </button>
            <button
              onClick={onConfirm}
              className={`px-4 py-2 rounded-md text-sm font-medium transition-colors cursor-pointer ${confirmClass}`}
            >
              {confirmLabel}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
