// Drag-to-resize for gutter panels (left sidebar / right sheet). Pointer
// capture keeps the drag alive outside the handle; no mouse-move listeners,
// no window cleanup. `dir` 1 = drag right edge (left panel), -1 = drag left
// edge (right panel). Widths are in-memory only.
//
// The handle is a focusable separator (WAI-ARIA window-splitter pattern):
// ←/→ resize when focused, Home resets, double-click resets. The title
// documents the range so the affordance isn't hover-stumble-only.

import { useRef, useState } from "react";

const KEY_STEP = 16;

export function useGutterResize(
  initial: number,
  min: number,
  max: number,
  dir: 1 | -1,
) {
  const [width, setWidth] = useState(initial);
  const startX = useRef(0);
  const startWidth = useRef(initial);
  const clamp = (n: number) => Math.min(max, Math.max(min, n));

  return {
    width,
    bind: {
      role: "separator" as const,
      tabIndex: 0,
      "aria-orientation": "vertical" as const,
      "aria-label": "Resize panel",
      "aria-valuenow": width,
      "aria-valuemin": min,
      "aria-valuemax": max,
      title: `Drag to resize (${min}–${max}px) · double-click to reset · arrow keys when focused`,
      onPointerDown: (e: React.PointerEvent<HTMLDivElement>) => {
        startX.current = e.clientX;
        startWidth.current = width;
        e.currentTarget.setPointerCapture(e.pointerId);
      },
      onPointerMove: (e: React.PointerEvent<HTMLDivElement>) => {
        if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
        setWidth(clamp(startWidth.current + dir * (e.clientX - startX.current)));
      },
      onDoubleClick: () => setWidth(initial),
      onKeyDown: (e: React.KeyboardEvent<HTMLDivElement>) => {
        // Divider moves with the arrow: for dir=1 (left panel) → widens;
        // for dir=-1 (right sheet) → shrinks, matching the drag direction.
        if (e.key === "ArrowRight") {
          e.preventDefault();
          setWidth((w) => clamp(w + dir * KEY_STEP));
        } else if (e.key === "ArrowLeft") {
          e.preventDefault();
          setWidth((w) => clamp(w - dir * KEY_STEP));
        } else if (e.key === "Home") {
          e.preventDefault();
          setWidth(initial);
        }
      },
    },
  };
}
