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

export function IconGear({ size }: IconProps) {
  // Project settings: an actual gear — blocky 8-tooth outline + center hole.
  return (
    <svg {...base(size)}>
      <path d="M8.92 5.33L10.35 5.31L10.40 5.85L10.40 6.15L10.35 6.69L8.92 6.67L8.54 7.59L9.56 8.59L9.22 9.00L9.00 9.22L8.59 9.56L7.59 8.54L6.67 8.92L6.69 10.35L6.15 10.40L5.85 10.40L5.31 10.35L5.33 8.92L4.41 8.54L3.41 9.56L3.00 9.22L2.78 9.00L2.44 8.59L3.46 7.59L3.08 6.67L1.65 6.69L1.60 6.15L1.60 5.85L1.65 5.31L3.08 5.33L3.46 4.41L2.44 3.41L2.78 3.00L3.00 2.78L3.41 2.44L4.41 3.46L5.33 3.08L5.31 1.65L5.85 1.60L6.15 1.60L6.69 1.65L6.67 3.08L7.59 3.46L8.59 2.44L9.00 2.78L9.22 3.00L9.56 3.41L8.54 4.41Z" />
      <circle cx="6" cy="6" r="1.6" />
    </svg>
  );
}

export function IconSearch({ size }: IconProps) {
  // Magnifier: command palette / search.
  return (
    <svg {...base(size)}>
      <circle cx="5.2" cy="5.2" r="3.4" />
      <path d="M7.8 7.8L10.5 10.5" />
    </svg>
  );
}

export function IconSliders({ size }: IconProps) {
  // Adjustment sliders: app settings (distinct from the project gear).
  return (
    <svg {...base(size)}>
      <path d="M1.5 3.5h9M1.5 8.5h9" />
      <circle cx="4.2" cy="3.5" r="1.4" />
      <circle cx="7.8" cy="8.5" r="1.4" />
    </svg>
  );
}

export function IconColumns({ size }: IconProps) {
  // Arrow entering a column: move this card to another column.
  return (
    <svg {...base(size)}>
      <path d="M1 6h5.5" />
      <path d="M4.5 4L6.5 6l-2 2" />
      <path d="M9.5 2.5v7" />
    </svg>
  );
}

export function IconCard({ size }: IconProps) {
  // Ticket card with a title line: the card detail.
  return (
    <svg {...base(size)}>
      <rect x="1.5" y="2.5" width="9" height="7" rx="1" />
      <path d="M3.5 5h5M3.5 7h3" />
    </svg>
  );
}

export function IconFolder({ size }: IconProps) {
  // Folder: the worktree file tree.
  return (
    <svg {...base(size)}>
      <path d="M1.5 9V3.5a1 1 0 0 1 1-1h2l1 1.5h4a1 1 0 0 1 1 1V9a1 1 0 0 1-1 1h-7a1 1 0 0 1-1-1z" />
    </svg>
  );
}

export function IconTrash({ size }: IconProps) {
  return (
    <svg {...base(size)}>
      <path d="M2 3.5h8" />
      <path d="M4.5 3.5v-1h3v1" />
      <path d="M3 3.5l.4 5.6a1 1 0 0 0 1 .9h3.2a1 1 0 0 0 1-.9l.4-5.6" />
    </svg>
  );
}

export function IconChat({ size }: IconProps) {
  // Speech bubble: the project chat panel toggle.
  return (
    <svg {...base(size)}>
      <path d="M10.5 7.5a1 1 0 0 1-1 1H3.5l-2 2V2.5a1 1 0 0 1 1-1h7a1 1 0 0 1 1 1z" />
    </svg>
  );
}

