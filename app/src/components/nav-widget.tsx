// On-screen nav widget. Mirrors every keyboard-bound flight-cam
// control as a tappable button so a mobile / remote session without
// keyboard or wheel can still drive the camera fully.
//
// Movement-style buttons (translate / rotate / slew / dolly) hold the
// underlying keypress for as long as the user keeps the button
// pressed. They synthesize `keydown` on pointer-down and `keyup` on
// pointer-up / cancel / leave, so the existing `flight-camera.ts`
// per-frame loop (which reads `heldKeys`) doesn't need to know the
// nav widget exists.
//
// One-shot buttons (R reframe, H pivot orb, P projection, F9 capture,
// Numpad presets) synthesize a press-release pair on tap; the
// `dispatchViewerHotkey` path fires once.
//
// Speed +/- and FoV +/- can't reuse the synthetic-events path because
// the wheel listener lives on the renderer's canvas (not window). The
// flight cam exposes `nudgeMoveSpeed(direction)` and
// `nudgeFov(direction)` for those.

import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ArrowDown,
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  Camera,
  Eye,
  Focus,
  Gamepad2,
  Minus,
  Plus,
} from "lucide-react";
import type { FlightCamHandle } from "../lib/flight-camera";

interface NavWidgetProps {
  flightCamHandle: FlightCamHandle | null;
  /** When false, the widget renders only the toggle pill so the user
   *  can opt in. Default false. */
  initiallyOpen?: boolean;
}

/** Synthesize a window-level keyboard event so the existing flight-cam
 *  listeners pick it up. The flight cam stores `e.code` in `heldKeys`
 *  and the per-frame loop reads it; matches the behaviour of a real
 *  keyboard press. `isTrusted` will be false on the synthetic event
 *  but neither the flight-cam handler nor `dispatchViewerHotkey`
 *  gates on that. */
function fireKey(code: string, type: "keydown" | "keyup"): void {
  const event = new KeyboardEvent(type, {
    code,
    bubbles: true,
    cancelable: true,
  });
  window.dispatchEvent(event);
}

/** Press-and-hold: down on pointer-down, up on pointer-up / cancel /
 *  leave. The leave handler is critical for touch -- if a finger
 *  slides off the button, the browser fires pointercancel only on
 *  some platforms. We bind both to be safe. */
function holdHandlers(code: string) {
  return {
    onPointerDown: (e: React.PointerEvent) => {
      e.preventDefault();
      // capture so a sliding finger keeps generating events targeted
      // here, and we get a clean release notification.
      e.currentTarget.setPointerCapture(e.pointerId);
      fireKey(code, "keydown");
    },
    onPointerUp: (e: React.PointerEvent) => {
      try {
        e.currentTarget.releasePointerCapture(e.pointerId);
      } catch {
        // already released; not a problem.
      }
      fireKey(code, "keyup");
    },
    onPointerCancel: (e: React.PointerEvent) => {
      try {
        e.currentTarget.releasePointerCapture(e.pointerId);
      } catch {
        // ignore
      }
      fireKey(code, "keyup");
    },
    onContextMenu: (e: React.MouseEvent) => {
      // Long-press on touch devices fires a context menu by default;
      // suppress so the user can hold buttons without surprise menus.
      e.preventDefault();
    },
  };
}

/** One-shot tap: down + up, no held state. Suitable for hotkeys that
 *  the dispatcher fires on keydown (R, H, P, F9, Numpad presets). */
function tapHandlers(code: string) {
  return {
    onPointerDown: (e: React.PointerEvent) => {
      e.preventDefault();
    },
    onPointerUp: () => {
      fireKey(code, "keydown");
      fireKey(code, "keyup");
    },
    onContextMenu: (e: React.MouseEvent) => {
      e.preventDefault();
    },
  };
}

/** Visual style for a nav button. Two variants: `key` (light, default)
 *  and `accent` (action). */
function btnClass(variant: "key" | "accent" = "key"): string {
  const base =
    "flex items-center justify-center gap-1 select-none touch-none cursor-pointer transition-colors font-mono " +
    "active:bg-accent/30 active:text-text";
  if (variant === "accent") {
    return `${base} px-2 py-1 rounded border border-accent/40 bg-accent/15 text-text text-[11px]`;
  }
  return `${base} w-9 h-9 rounded border border-border bg-bg/60 text-text-sub hover:bg-bg/80 hover:text-text text-[11px]`;
}

export function NavWidget({
  flightCamHandle,
  initiallyOpen = false,
}: NavWidgetProps) {
  const [open, setOpen] = useState(initiallyOpen);

  // Speed and FoV use the FlightCamHandle methods directly rather than
  // synthetic wheel/keydown events. The wheel listener lives on the
  // renderer's canvas; window-level synthetic wheel events miss it.
  const onSpeedFaster = useCallback(() => {
    flightCamHandle?.nudgeMoveSpeed(1);
  }, [flightCamHandle]);
  const onSpeedSlower = useCallback(() => {
    flightCamHandle?.nudgeMoveSpeed(-1);
  }, [flightCamHandle]);
  const onFovWider = useCallback(() => {
    flightCamHandle?.nudgeFov(1);
  }, [flightCamHandle]);
  const onFovNarrower = useCallback(() => {
    flightCamHandle?.nudgeFov(-1);
  }, [flightCamHandle]);

  // For "tap-style" speed/FoV buttons, build the same shape as
  // `tapHandlers` so the hover styling matches.
  const speedFasterHandlers = useMemo(
    () => ({
      onPointerUp: onSpeedFaster,
      onContextMenu: (e: React.MouseEvent) => e.preventDefault(),
    }),
    [onSpeedFaster],
  );
  const speedSlowerHandlers = useMemo(
    () => ({
      onPointerUp: onSpeedSlower,
      onContextMenu: (e: React.MouseEvent) => e.preventDefault(),
    }),
    [onSpeedSlower],
  );
  const fovWiderHandlers = useMemo(
    () => ({
      onPointerUp: onFovWider,
      onContextMenu: (e: React.MouseEvent) => e.preventDefault(),
    }),
    [onFovWider],
  );
  const fovNarrowerHandlers = useMemo(
    () => ({
      onPointerUp: onFovNarrower,
      onContextMenu: (e: React.MouseEvent) => e.preventDefault(),
    }),
    [onFovNarrower],
  );

  // Defensive cleanup: if the widget unmounts while a button is held,
  // the captured pointer would otherwise stay captured and the keyup
  // would never fire. Releasing all currently-held keys on unmount is
  // a belt-and-braces guard.
  useEffect(() => {
    return () => {
      // This is a soft guard; the more common case is the per-frame
      // loop reads `heldKeys` and computes a non-zero delta forever
      // until the user clicks something. Sending keyups for the full
      // set is cheap.
      for (const code of [
        "KeyW",
        "KeyA",
        "KeyS",
        "KeyD",
        "KeyQ",
        "KeyE",
        "KeyI",
        "KeyJ",
        "KeyK",
        "KeyL",
        "KeyU",
        "KeyO",
        "ArrowUp",
        "ArrowDown",
        "ArrowLeft",
        "ArrowRight",
        "BracketLeft",
        "BracketRight",
      ]) {
        fireKey(code, "keyup");
      }
    };
  }, []);

  if (!open) {
    return (
      <button
        type="button"
        onClick={() => setOpen(true)}
        title="Show on-screen nav widget"
        aria-label="Show on-screen nav widget"
        className="flex items-center gap-1.5 px-2 py-1 rounded border border-border bg-bg-alt/90 text-text-sub hover:bg-bg-alt hover:text-text text-[10px] font-mono shadow"
      >
        <Gamepad2 size={12} strokeWidth={1.75} />
        nav
      </button>
    );
  }

  return (
    <div className="bg-bg-alt/95 border border-border rounded-md shadow p-2 flex flex-col gap-2 backdrop-blur-sm select-none">
      {/* Header with hide button. */}
      <div className="flex items-center justify-between gap-2">
        <span className="text-[10px] uppercase tracking-wider text-text-faint font-medium flex items-center gap-1.5">
          <Gamepad2 size={11} strokeWidth={1.75} />
          On-screen nav
        </span>
        <button
          type="button"
          onClick={() => setOpen(false)}
          title="Hide on-screen nav widget"
          aria-label="Hide on-screen nav widget"
          className="text-[10px] px-1.5 py-0.5 rounded border border-border bg-bg/50 text-text-sub hover:bg-bg/80 hover:text-text font-mono shrink-0"
        >
          hide
        </button>
      </div>

      {/* Translate (3 linear DOF) -- 3x2 grid of WASDQE. Layout puts
          the up axis on top so the spatial mapping is intuitive. */}
      <Section label="Translate">
        <div className="grid grid-cols-3 gap-1">
          <button {...holdHandlers("KeyQ")} title="Translate ventral (Q)" className={btnClass()}>Q</button>
          <button {...holdHandlers("KeyW")} title="Translate fore (W)" className={btnClass()}>W</button>
          <button {...holdHandlers("KeyE")} title="Translate dorsal (E)" className={btnClass()}>E</button>
          <button {...holdHandlers("KeyA")} title="Translate port (A)" className={btnClass()}>A</button>
          <button {...holdHandlers("KeyS")} title="Translate aft (S)" className={btnClass()}>S</button>
          <button {...holdHandlers("KeyD")} title="Translate starboard (D)" className={btnClass()}>D</button>
        </div>
        <div className="grid grid-cols-2 gap-1 mt-1">
          <button {...holdHandlers("BracketLeft")} title="Dolly aft ([)" className={btnClass()}>[</button>
          <button {...holdHandlers("BracketRight")} title="Dolly fore (])" className={btnClass()}>]</button>
        </div>
      </Section>

      {/* Rotate (3 angular DOF) -- IJKL/UO. */}
      <Section label="Rotate">
        <div className="grid grid-cols-3 gap-1">
          <button {...holdHandlers("KeyU")} title="Roll port (U)" className={btnClass()}>U</button>
          <button {...holdHandlers("KeyI")} title="Pitch up (I)" className={btnClass()}>I</button>
          <button {...holdHandlers("KeyO")} title="Roll starboard (O)" className={btnClass()}>O</button>
          <button {...holdHandlers("KeyJ")} title="Yaw port (J)" className={btnClass()}>J</button>
          <button {...holdHandlers("KeyK")} title="Pitch down (K)" className={btnClass()}>K</button>
          <button {...holdHandlers("KeyL")} title="Yaw starboard (L)" className={btnClass()}>L</button>
        </div>
      </Section>

      {/* Slew (2 angular DOF) -- arrow keys. Cross layout so the spatial
          mapping is obvious. */}
      <Section label="Slew">
        <div className="grid grid-cols-3 gap-1 w-fit mx-auto">
          <span />
          <button {...holdHandlers("ArrowUp")} title="Slew pitch up" className={btnClass()}>
            <ArrowUp size={14} strokeWidth={2} />
          </button>
          <span />
          <button {...holdHandlers("ArrowLeft")} title="Slew yaw port" className={btnClass()}>
            <ArrowLeft size={14} strokeWidth={2} />
          </button>
          <button {...holdHandlers("ArrowDown")} title="Slew pitch down" className={btnClass()}>
            <ArrowDown size={14} strokeWidth={2} />
          </button>
          <button {...holdHandlers("ArrowRight")} title="Slew yaw starboard" className={btnClass()}>
            <ArrowRight size={14} strokeWidth={2} />
          </button>
        </div>
      </Section>

      {/* Speed and FoV. Each tap = one wheel tick / Numpad press. */}
      <Section label="Speed / FoV">
        <div className="flex items-center gap-1 text-[10px] text-text-sub">
          <span className="w-9 shrink-0">Speed</span>
          <button {...speedSlowerHandlers} title="Slower" className={btnClass()}>
            <Minus size={14} strokeWidth={2} />
          </button>
          <button {...speedFasterHandlers} title="Faster" className={btnClass()}>
            <Plus size={14} strokeWidth={2} />
          </button>
        </div>
        <div className="flex items-center gap-1 text-[10px] text-text-sub mt-1">
          <span className="w-9 shrink-0">FoV</span>
          <button {...fovNarrowerHandlers} title="Narrower FoV" className={btnClass()}>
            <Minus size={14} strokeWidth={2} />
          </button>
          <button {...fovWiderHandlers} title="Wider FoV" className={btnClass()}>
            <Plus size={14} strokeWidth={2} />
          </button>
        </div>
      </Section>

      {/* View presets -- Numpad 0-5. */}
      <Section label="View presets">
        <div className="grid grid-cols-6 gap-1">
          <button {...tapHandlers("Numpad0")} title="Overhead (Numpad 0)" className={btnClass()}>0</button>
          <button {...tapHandlers("Numpad1")} title="Perspective 2 (Numpad 1)" className={btnClass()}>1</button>
          <button {...tapHandlers("Numpad2")} title="Side (Numpad 2)" className={btnClass()}>2</button>
          <button {...tapHandlers("Numpad3")} title="Fore (Numpad 3)" className={btnClass()}>3</button>
          <button {...tapHandlers("Numpad4")} title="Aft (Numpad 4)" className={btnClass()}>4</button>
          <button {...tapHandlers("Numpad5")} title="Perspective (Numpad 5)" className={btnClass()}>5</button>
        </div>
      </Section>

      {/* Action row -- R reframe, H pivot orb, P projection cycle, F9
          screenshot. Wider buttons so the icon + label fit. */}
      <Section label="Actions">
        <div className="grid grid-cols-2 gap-1">
          <button {...tapHandlers("KeyR")} title="Reframe (R)" className={btnClass("accent")}>
            <Focus size={12} strokeWidth={2} />
            R reset
          </button>
          <button {...tapHandlers("KeyH")} title="Toggle pivot orb (H)" className={btnClass("accent")}>
            <Eye size={12} strokeWidth={2} />
            H orb
          </button>
          <button {...tapHandlers("KeyP")} title="Cycle projection (P)" className={btnClass("accent")}>
            P proj
          </button>
          <button {...tapHandlers("F9")} title="Screenshot (F9)" className={btnClass("accent")}>
            <Camera size={12} strokeWidth={2} />
            F9
          </button>
        </div>
      </Section>
    </div>
  );
}

function Section({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-0.5">
      <div className="text-[9px] uppercase tracking-wider text-text-faint font-medium">
        {label}
      </div>
      {children}
    </div>
  );
}
