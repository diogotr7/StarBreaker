// Floating, collapsible Settings panel anchored to the top-right of the
// viewer pane. Used by both the Ships viewer and the Maps (SOC) viewer.
//
// The same `ViewerSettings` shape feeds both surfaces so user-tuned
// values persist as the user switches tabs. A few rows are Ships-only
// because they map to data the SOC GLB does not carry (per-shader-
// family metalness override, MeshPhysicalMaterial clearcoat, the
// camera-attached headlight); those rows are gated by the `kind` prop.

import {
  useCallback,
  useState,
  type ReactNode,
} from "react";
import { ChevronDown, ChevronUp, Settings } from "lucide-react";
import {
  DEFAULT_DIAGNOSTIC_SETTINGS,
  type DiagnosticSettings,
} from "./scene-viewer";

/** Surface this panel is rendering against -- gates Ships-only rows
 *  whose underlying material/light targets do not exist on SOC scenes. */
export type ViewerSurfaceKind = "ships" | "soc";

/** User-tunable viewer settings. Flat object so adding a new field is
 *  one line per setting; the panel renders rows declaratively from
 *  this shape so growing the surface is a localised edit. */
export interface ViewerSettings {
  showGroundPlane: boolean;
  showGrid: boolean;
  /** Show the orange sphere marking the orbit pivot. Off by default
   *  -- the orb is a debug-style affordance most users do not want
   *  visible all the time. Toggling this calls
   *  `flightCamHandle.setPivotOrbVisible(...)` from the parent. */
  showPivotOrb: boolean;
  groundPlaneColor: [number, number, number];
  diagnostics: DiagnosticSettings;
}

export const DEFAULT_VIEWER_SETTINGS: ViewerSettings = {
  showGroundPlane: false,
  showGrid: false,
  showPivotOrb: false,
  groundPlaneColor: [128, 128, 128],
  diagnostics: { ...DEFAULT_DIAGNOSTIC_SETTINGS },
};

interface SettingsPanelProps {
  settings: ViewerSettings;
  onChange: (patch: Partial<ViewerSettings>) => void;
  kind: ViewerSurfaceKind;
}

/**
 * Floating, collapsible settings overlay anchored to the top-right of
 * the viewer pane. Starts collapsed (button only). Clicking the button
 * toggles the body open/closed.
 */
export function SettingsPanel({ settings, onChange, kind }: SettingsPanelProps) {
  const [open, setOpen] = useState(false);

  // Guard: merge defaults so that missing fields (e.g. after a store
  // hydration against an older settings shape) never produce undefined
  // values reaching .toFixed() in the slider rows.
  const diag: DiagnosticSettings = {
    ...DEFAULT_DIAGNOSTIC_SETTINGS,
    ...settings.diagnostics,
  };
  const groundColor: [number, number, number] =
    settings.groundPlaneColor ?? DEFAULT_VIEWER_SETTINGS.groundPlaneColor;

  const patchDiag = useCallback(
    (patch: Partial<DiagnosticSettings>) => {
      onChange({
        diagnostics: {
          ...DEFAULT_DIAGNOSTIC_SETTINGS,
          ...settings.diagnostics,
          ...patch,
        },
      });
    },
    [onChange, settings.diagnostics],
  );

  const patchGroundColor = useCallback(
    (channel: 0 | 1 | 2, val: number) => {
      const base =
        settings.groundPlaneColor ?? DEFAULT_VIEWER_SETTINGS.groundPlaneColor;
      const next: [number, number, number] = [...base] as [
        number,
        number,
        number,
      ];
      next[channel] = val;
      onChange({ groundPlaneColor: next });
    },
    [onChange, settings.groundPlaneColor],
  );

  const resetAll = useCallback(() => {
    onChange({ diagnostics: { ...DEFAULT_DIAGNOSTIC_SETTINGS } });
  }, [onChange]);

  const showShipsOnlyRows = kind === "ships";

  return (
    <div className="relative z-10">
      <button
        onClick={() => setOpen((v) => !v)}
        className="flex items-center gap-2 px-2.5 py-1.5 rounded-md bg-bg-alt/90 border border-border text-xs text-text-sub hover:text-text hover:bg-bg-alt shadow transition-colors cursor-pointer select-none"
        title={open ? "Collapse settings" : "Expand settings"}
        aria-label={open ? "Collapse settings panel" : "Expand settings panel"}
      >
        <Settings size={14} strokeWidth={1.75} />
        <span>Settings</span>
        {open ? (
          <ChevronDown size={12} strokeWidth={1.75} />
        ) : (
          <ChevronUp size={12} strokeWidth={1.75} />
        )}
      </button>
      {open && (
        <div
          className="absolute top-full right-0 mt-1.5 w-[280px] max-h-[70vh] overflow-y-auto bg-bg-alt/95 border border-border rounded-md shadow-lg p-3 flex flex-col gap-3 backdrop-blur-sm"
          onClick={(e) => e.stopPropagation()}
        >
          <SettingsSection title="Display">
            <SettingsToggleRow
              label="Show ground plane"
              checked={
                settings.showGroundPlane ??
                DEFAULT_VIEWER_SETTINGS.showGroundPlane
              }
              onChange={(v) => onChange({ showGroundPlane: v })}
            />
            <SettingsToggleRow
              label="Show grid"
              checked={settings.showGrid ?? DEFAULT_VIEWER_SETTINGS.showGrid}
              onChange={(v) => onChange({ showGrid: v })}
            />
            <SettingsToggleRow
              label="Show pivot orb"
              checked={
                settings.showPivotOrb ?? DEFAULT_VIEWER_SETTINGS.showPivotOrb
              }
              onChange={(v) => onChange({ showPivotOrb: v })}
            />
          </SettingsSection>

          <SettingsSection title="Ground Plane Color">
            <SettingsSliderRow
              label="R"
              value={groundColor[0]}
              min={0}
              max={255}
              step={1}
              displayDecimals={0}
              onChange={(v) => patchGroundColor(0, v)}
            />
            <SettingsSliderRow
              label="G"
              value={groundColor[1]}
              min={0}
              max={255}
              step={1}
              displayDecimals={0}
              onChange={(v) => patchGroundColor(1, v)}
            />
            <SettingsSliderRow
              label="B"
              value={groundColor[2]}
              min={0}
              max={255}
              step={1}
              displayDecimals={0}
              onChange={(v) => patchGroundColor(2, v)}
            />
          </SettingsSection>

          <SettingsSection title="Render Tuning">
            <SettingsSliderRow
              label="envMapIntensity"
              value={diag.envMapIntensity}
              min={0}
              max={2}
              step={0.05}
              onChange={(v) => patchDiag({ envMapIntensity: v })}
            />
            <SettingsSliderRow
              label="Tone map exposure"
              value={diag.toneMappingExposure}
              min={0}
              max={3}
              step={0.05}
              onChange={(v) => patchDiag({ toneMappingExposure: v })}
            />
            {showShipsOnlyRows && (
              <SettingsSliderRow
                label="Metalness"
                value={diag.metalness}
                min={0}
                max={1}
                step={0.01}
                onChange={(v) => patchDiag({ metalness: v })}
              />
            )}
            {showShipsOnlyRows && (
              <SettingsCheckboxSliderRow
                label="Roughness"
                value={diag.roughness}
                min={0}
                max={1}
                step={0.01}
                enabled={diag.roughnessOverrideEnabled}
                onEnabledChange={(v) =>
                  patchDiag({ roughnessOverrideEnabled: v })
                }
                onChange={(v) => patchDiag({ roughness: v })}
              />
            )}
            {showShipsOnlyRows && (
              <SettingsSliderRow
                label="Clearcoat"
                value={diag.clearcoat}
                min={0}
                max={1}
                step={0.05}
                onChange={(v) => patchDiag({ clearcoat: v })}
              />
            )}
          </SettingsSection>

          <SettingsSection title="Scene Lights">
            <SettingsSliderRow
              label="Ambient intensity"
              value={diag.ambientIntensity}
              min={0}
              max={2}
              step={0.05}
              onChange={(v) => patchDiag({ ambientIntensity: v })}
            />
            <SettingsSliderRow
              label="Directional intensity"
              value={diag.directionalIntensity}
              min={0}
              max={5}
              step={0.1}
              onChange={(v) => patchDiag({ directionalIntensity: v })}
            />
            {showShipsOnlyRows && (
              <SettingsSliderRow
                label="Headlight intensity"
                value={diag.headlightIntensity}
                min={0}
                max={5}
                step={0.1}
                onChange={(v) => patchDiag({ headlightIntensity: v })}
              />
            )}
          </SettingsSection>

          <SettingsSection title="Color Path">
            <SettingsSliderRow
              label="Color saturation"
              value={diag.colorSaturation}
              min={0}
              max={2}
              step={0.05}
              onChange={(v) => patchDiag({ colorSaturation: v })}
            />
          </SettingsSection>

          <button
            onClick={resetAll}
            className="mt-1 w-full py-1.5 rounded-md text-xs font-medium bg-surface hover:bg-surface-hi text-text-sub hover:text-text transition-colors cursor-pointer"
            title="Reset all sliders to defaults"
          >
            Reset all
          </button>
        </div>
      )}
    </div>
  );
}

function SettingsSection({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <div className="text-[10px] uppercase tracking-wider text-text-faint font-medium">
        {title}
      </div>
      <div className="flex flex-col gap-1">{children}</div>
    </div>
  );
}

function SettingsToggleRow({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="flex items-center justify-between gap-3 px-1.5 py-1 rounded text-xs text-text-sub hover:bg-surface/40 cursor-pointer select-none">
      <span>{label}</span>
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="accent-accent cursor-pointer"
      />
    </label>
  );
}

function SettingsSliderRow({
  label,
  value,
  min,
  max,
  step,
  displayDecimals = 2,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  displayDecimals?: number;
  onChange: (v: number) => void;
}) {
  return (
    <div className="flex flex-col gap-0.5 px-1.5 py-1">
      <div className="flex items-center justify-between text-xs text-text-sub select-none">
        <span>{label}</span>
        <span className="tabular-nums text-text-faint">
          {value.toFixed(displayDecimals)}
        </span>
      </div>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(parseFloat(e.target.value))}
        className="w-full accent-accent cursor-pointer"
      />
    </div>
  );
}

function SettingsCheckboxSliderRow({
  label,
  value,
  min,
  max,
  step,
  enabled,
  onEnabledChange,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  enabled: boolean;
  onEnabledChange: (v: boolean) => void;
  onChange: (v: number) => void;
}) {
  return (
    <div
      className={`flex flex-col gap-0.5 px-1.5 py-1 ${enabled ? "" : "opacity-60"}`}
    >
      <div className="flex items-center justify-between text-xs text-text-sub select-none">
        <label className="flex items-center gap-1.5 cursor-pointer">
          <input
            type="checkbox"
            checked={enabled}
            onChange={(e) => onEnabledChange(e.target.checked)}
            className="accent-accent cursor-pointer"
          />
          <span>{label}</span>
        </label>
        <span className="tabular-nums text-text-faint">
          {value.toFixed(2)}
        </span>
      </div>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={!enabled}
        onChange={(e) => onChange(parseFloat(e.target.value))}
        className="w-full accent-accent cursor-pointer disabled:cursor-not-allowed"
      />
    </div>
  );
}
