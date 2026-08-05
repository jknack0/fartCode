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
