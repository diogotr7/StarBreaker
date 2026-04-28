// `?` button in the top-right toolbar strip. Click to drop a panel
// listing every keyboard / mouse control the flight cam responds to.
// Matches the SettingsPanel button-with-dropdown pattern so the two
// read as a pair.
//
// The keymap is the source of truth; if `flight-camera.ts` adds or
// changes a binding, update this file to match. Bindings live in:
//   - flight-camera.ts: WASDQE movement, IJKL/UO look, Arrow orbit,
//     [/] cart, Numpad +/- FoV, Numpad 0-5 view presets, P projection
//     cycle, R reframe, H HUD toggle, mouse buttons, wheel speed.
//   - scene-viewer.tsx (Ships): F9 screenshot capture.

import { useState } from "react";
import { ChevronDown, ChevronUp, HelpCircle } from "lucide-react";
import type { ViewerSurfaceKind } from "./settings-panel";

interface HelpPanelProps {
  /** Surface controls vary slightly between Ships and Maps. F9 capture
   *  is wired only on the Ships viewer today (SOC has a no-op stub),
   *  so we annotate it instead of hiding it. */
  kind: ViewerSurfaceKind;
}

interface ControlGroup {
  title: string;
  items: Array<{ keys: string; description: string; note?: string }>;
}

function buildGroups(kind: ViewerSurfaceKind): ControlGroup[] {
  return [
    {
      title: "Move",
      items: [
        { keys: "W A S D", description: "Forward / left / back / right" },
        { keys: "Q E", description: "Down / up" },
        { keys: "[ ]", description: "Cart out / in (along look direction)" },
      ],
    },
    {
      title: "Look",
      items: [
        { keys: "I K", description: "Pitch up / down" },
        { keys: "J L", description: "Yaw left / right" },
        { keys: "U O", description: "Roll left / right" },
      ],
    },
    {
      title: "Orbit",
      items: [
        { keys: "Arrows", description: "Swing around the pivot point" },
      ],
    },
    {
      title: "Mouse",
      items: [
        { keys: "Left drag", description: "Pan in screen space" },
        { keys: "Right drag", description: "Orbit around the pivot" },
        { keys: "Middle drag", description: "Look without moving the pivot" },
        { keys: "Wheel", description: "Adjust movement speed (not zoom)" },
      ],
    },
    {
      title: "View",
      items: [
        { keys: "R", description: "Reframe to default view" },
        {
          keys: "Numpad 0-5",
          description:
            "Preset views: 0 overhead / 1 perspective / 2 side / 3 fore / 4 aft / 5 perspective",
        },
        { keys: "P", description: "Cycle projection: perspective / ortho / oblique" },
        { keys: "Numpad + -", description: "FoV up / down" },
      ],
    },
    {
      title: "UI",
      items: [
        { keys: "H", description: "Toggle the orange pivot orb" },
        {
          keys: "F9",
          description: "Capture high-resolution screenshot",
          note: kind === "soc" ? "Ships viewer only today" : undefined,
        },
      ],
    },
  ];
}

export function HelpPanel({ kind }: HelpPanelProps) {
  const [open, setOpen] = useState(false);
  const groups = buildGroups(kind);

  return (
    <div className="relative z-10">
      <button
        onClick={() => setOpen((v) => !v)}
        className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-md bg-bg-alt/90 border border-border text-xs text-text-sub hover:text-text hover:bg-bg-alt shadow transition-colors cursor-pointer select-none"
        title={open ? "Hide keyboard controls" : "Show keyboard controls"}
        aria-label={open ? "Hide keyboard controls" : "Show keyboard controls"}
      >
        <HelpCircle size={14} strokeWidth={1.75} />
        <span className="font-mono">?</span>
        {open ? (
          <ChevronDown size={12} strokeWidth={1.75} />
        ) : (
          <ChevronUp size={12} strokeWidth={1.75} />
        )}
      </button>
      {open && (
        <div
          className="absolute top-full right-0 mt-1.5 w-[340px] max-h-[70vh] overflow-y-auto bg-bg-alt/95 border border-border rounded-md shadow-lg p-3 flex flex-col gap-3 backdrop-blur-sm"
          onClick={(e) => e.stopPropagation()}
        >
          {groups.map((group) => (
            <div key={group.title} className="flex flex-col gap-1">
              <div className="text-[10px] uppercase tracking-wider text-text-faint font-medium">
                {group.title}
              </div>
              <div className="flex flex-col gap-0.5">
                {group.items.map((item) => (
                  <div
                    key={item.keys}
                    className="flex items-baseline gap-2 px-1 py-0.5 text-xs"
                  >
                    <span className="font-mono text-text shrink-0 w-[88px]">
                      {item.keys}
                    </span>
                    <span className="text-text-sub flex-1">
                      {item.description}
                      {item.note && (
                        <span className="text-text-faint italic">
                          {" "}
                          ({item.note})
                        </span>
                      )}
                    </span>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
