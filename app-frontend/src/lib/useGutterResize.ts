// Drag-to-resize for gutter panels (left sidebar / right sheet). Pointer
// capture keeps the drag alive outside the handle; no mouse-move listeners,
// no window cleanup. `dir` 1 = drag right edge (left panel), -1 = drag left
// edge (right panel). Widths are in-memory only.

import { useRef, useState } from "react";

export function useGutterResize(
  initial: number,
  min: number,
  max: number,
  dir: 1 | -1,
) {
  const [width, setWidth] = useState(initial);
  const startX = useRef(0);
  const startWidth = useRef(initial);

  return {
    width,
    bind: {
      onPointerDown: (e: React.PointerEvent<HTMLDivElement>) => {
        startX.current = e.clientX;
        startWidth.current = width;
        e.currentTarget.setPointerCapture(e.pointerId);
      },
      onPointerMove: (e: React.PointerEvent<HTMLDivElement>) => {
        if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
        const next = startWidth.current + dir * (e.clientX - startX.current);
        setWidth(Math.min(max, Math.max(min, next)));
      },
    },
  };
}
