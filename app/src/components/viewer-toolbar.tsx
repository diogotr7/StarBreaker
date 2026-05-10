// Top-right strip of controls overlaid on the 3D viewer pane. Both the
// Ships and the Maps surfaces render this same strip so future controls
// land in one place. The Ships surface plugs its livery dropdown into
// `leadingSlot`; everything else is shared.

import { type ReactNode } from "react";
import { ProjectionModePicker } from "./projection-mode-picker";
import {
  SettingsPanel,
  type ViewerSettings,
  type ViewerSurfaceKind,
} from "./settings-panel";
import { HelpPanel } from "./help-panel";
import {
  RENDER_STYLES,
  type RenderStyle,
} from "../lib/render-styles";
import type { FlightCamHandle } from "../lib/flight-camera";

export interface ViewerToolbarProps {
  flightCamHandle: FlightCamHandle | null;
  renderStyle: RenderStyle;
  onRenderStyleChange: (style: RenderStyle) => void;
  settings: ViewerSettings;
  onSettingsChange: (patch: Partial<ViewerSettings>) => void;
  kind: ViewerSurfaceKind;
  /** Optional element rendered between the projection picker and the
   *  style dropdown. The Ships surface uses this slot for the per-ship
   *  livery picker (which only renders when paint variants exist). */
  leadingSlot?: ReactNode;
}

export function ViewerToolbar({
  flightCamHandle,
  renderStyle,
  onRenderStyleChange,
  settings,
  onSettingsChange,
  kind,
  leadingSlot,
}: ViewerToolbarProps) {
  return (
    <div className="absolute top-3 right-3 z-10 flex items-center gap-2">
      <ProjectionModePicker handle={flightCamHandle} embedded />
      {leadingSlot}
      <label className="flex items-center gap-2 px-2.5 py-1.5 rounded-md bg-bg-alt/90 border border-border text-xs text-text-sub shadow">
        <span className="text-text-faint">Style</span>
        <select
          value={renderStyle}
          // Blur after selection so arrow keys go back to orbiting
          // the camera instead of cycling the dropdown options.
          onChange={(e) => {
            onRenderStyleChange(e.target.value as RenderStyle);
            e.currentTarget.blur();
          }}
          className="bg-transparent outline-none text-text cursor-pointer"
        >
          {RENDER_STYLES.map((opt) => (
            <option
              key={opt.value}
              value={opt.value}
              className="bg-bg-alt text-text"
            >
              {opt.label}
            </option>
          ))}
        </select>
      </label>
      <SettingsPanel
        settings={settings}
        onChange={onSettingsChange}
        kind={kind}
      />
      <HelpPanel kind={kind} />
    </div>
  );
}
