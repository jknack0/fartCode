// emdash world icon set: drawn SVG, one consistent 1.5px stroke,
// currentColor, sized to sit inside 26px header keys and 18px row actions.
interface IconProps {
  size?: number;
}

function base(size: number | undefined) {
  return {
    width: size ?? 12,
    height: size ?? 12,
    viewBox: "0 0 12 12",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 1.5,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
    "aria-hidden": true,
  };
}

export function IconPlus({ size }: IconProps) {
  return (
    <svg {...base(size)}>
      <path d="M6 2v8M2 6h8" />
    </svg>
  );
}

export function IconClose({ size }: IconProps) {
  return (
    <svg {...base(size)}>
      <path d="M3 3l6 6M9 3l-6 6" />
    </svg>
  );
}

export function IconChevron({ size }: IconProps) {
  // Points right at rest; the collapse control rotates it 90° when open.
  return (
    <svg {...base(size)}>
      <path d="M4.5 2.5L8 6l-3.5 3.5" />
    </svg>
  );
}

export function IconGear({ size }: IconProps) {
  return (
    <svg {...base(size)}>
      <circle cx="6" cy="6" r="2.1" />
      <path d="M6 1v1.4M6 9.6V11M1 6h1.4M9.6 6H11M2.46 2.46l.99.99M8.55 8.55l.99.99M9.54 2.46l-.99.99M3.45 8.55l-.99.99" />
    </svg>
  );
}

export function IconPin({ size }: IconProps) {
  return (
    <svg {...base(size)}>
      <circle cx="6" cy="4" r="2.2" />
      <path d="M6 6.2V11" />
    </svg>
  );
}

export function IconBranch({ size }: IconProps) {
  // git-branch: two nodes on a main line, one forked.
  return (
    <svg {...base(size)}>
      <circle cx="3.5" cy="2.5" r="1.4" />
      <circle cx="3.5" cy="9.5" r="1.4" />
      <circle cx="8.5" cy="4" r="1.4" />
      <path d="M3.5 3.9v4.2M8.5 5.4c0 1.6-2 2.1-3.4 2.4" />
    </svg>
  );
}

export function IconMinus({ size }: IconProps) {
  return (
    <svg {...base(size)}>
      <path d="M2 6h8" />
    </svg>
  );
}

export function IconDiscard({ size }: IconProps) {
  // Undo arrow: reverting to the last recorded state.
  return (
    <svg {...base(size)}>
      <path d="M2.5 4.5h4a3 3 0 0 1 0 6H5" />
      <path d="M4.5 2.5L2.5 4.5l2 2" />
    </svg>
  );
}

export function IconPull({ size }: IconProps) {
  // Down arrow onto a baseline: bring remote commits down (project pull).
  return (
    <svg {...base(size)}>
      <path d="M6 1.5v6" />
      <path d="M3.5 5L6 7.5L8.5 5" />
      <path d="M2 10.5h8" />
    </svg>
  );
}

export function IconGitHub({ size }: IconProps) {
  // Brand mark (filled glyph — the stroke convention doesn't apply to
  // brand shapes); opens the project's GitHub remote in the browser.
  return (
    <svg
      width={size ?? 12}
      height={size ?? 12}
      viewBox="0 0 24 24"
      fill="currentColor"
      stroke="none"
      aria-hidden
    >
      <path d="M12 .5C5.65.5.5 5.65.5 12c0 5.08 3.29 9.39 7.86 10.91.58.11.79-.25.79-.56 0-.27-.01-1.17-.02-2.12-3.2.7-3.87-1.36-3.87-1.36-.52-1.33-1.28-1.68-1.28-1.68-1.04-.71.08-.7.08-.7 1.15.08 1.76 1.19 1.76 1.19 1.03 1.76 2.69 1.25 3.35.96.1-.75.4-1.25.72-1.54-2.55-.29-5.23-1.28-5.23-5.68 0-1.26.45-2.28 1.19-3.09-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.18 1.18a11.1 11.1 0 0 1 5.78 0c2.21-1.49 3.18-1.18 3.18-1.18.63 1.59.23 2.76.11 3.05.74.81 1.19 1.83 1.19 3.09 0 4.41-2.69 5.38-5.25 5.67.41.35.77 1.05.77 2.12 0 1.53-.01 2.76-.01 3.14 0 .31.21.68.8.56A10.52 10.52 0 0 0 23.5 12C23.5 5.65 18.35.5 12 .5z" />
    </svg>
  );
}

export function IconChat({ size }: IconProps) {
  // Speech bubble: the project chat panel toggle.
  return (
    <svg {...base(size)}>
      <path d="M2.4 2h7.2a1.9 1.9 0 0 1 1.9 1.9v4.2a1.9 1.9 0 0 1-1.9 1.9H5L2.4 11.3V2.4A.4.4 0 0 1 2.4 2z" />
    </svg>
  );
}

