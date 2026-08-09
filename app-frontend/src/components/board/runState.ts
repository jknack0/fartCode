// Board run-state vocabulary (design_handoff_v2 frame 4a): maps the
// linked task's status onto the card's dot + meta treatment. The task
// status set is todo|in_progress|review|done|cancelled (ADR-0005) — the
// frame's failed/conflict/queue-ordinal states have no data-model
// counterpart yet; "failed" is mapped defensively should the backend
// ever emit it. Derived at render time, never stored.

export interface RunState {
  kind: "running" | "needs-you" | "failed" | "stopped" | "queued" | "neutral";
  /** Modifier class for the canonical .status-dot (styles.css ~823). */
  dot: string;
  /** Meta-line label; null renders no run segment (done/idle). */
  label: string | null;
  /** Meta text renders in var(--fc-bad-text). */
  bad: boolean;
  /** Title drops to #a4a4ab (stopped/queued vocabulary). */
  dimTitle: boolean;
  /** Whole row at .55 (queued). */
  dimRow: boolean;
  /** Second mono action line ("↵ read") is worth showing. */
  actionable: boolean;
}

const NEUTRAL: RunState = {
  kind: "neutral",
  dot: "",
  label: null,
  bad: false,
  dimTitle: false,
  dimRow: false,
  actionable: false,
};

export function runStateFor(status: string | undefined): RunState {
  switch (status) {
    case "in_progress":
      return { ...NEUTRAL, kind: "running", dot: "status-in_progress", label: "running" };
    case "review":
      return { ...NEUTRAL, kind: "needs-you", dot: "status-needs-you", label: "needs you" };
    case "failed":
      return {
        ...NEUTRAL,
        kind: "failed",
        dot: "status-failed",
        label: "failed",
        bad: true,
        actionable: true,
      };
    case "cancelled":
      return { ...NEUTRAL, kind: "stopped", dot: "run-stopped", label: "stopped", dimTitle: true };
    case "todo":
      return {
        ...NEUTRAL,
        kind: "queued",
        dot: "run-queued",
        label: "queued",
        dimTitle: true,
        dimRow: true,
      };
    default:
      return NEUTRAL;
  }
}

/** GitHub-imported issues carry their number in the title ("#12 Fix …",
 * fartcode-git/src/issues.rs); local issues have no short id — the ref
 * is display-derived, never stored. */
export function issueRefParts(
  title: string,
  externalRef: string | null,
): { ref: string | null; title: string } {
  const m = title.match(/^#(\d+)\s+(.*\S.*)$/);
  if (m) return { ref: `#${m[1]}`, title: m[2] };
  const g = externalRef?.match(/\/issues\/(\d+)\b/);
  if (g) return { ref: `#${g[1]}`, title };
  return { ref: null, title };
}

/** Blocker display label: its number when derivable, else a short title. */
export function blockerLabel(title: string): string {
  const { ref, title: rest } = issueRefParts(title, null);
  if (ref) return ref;
  return rest.length > 24 ? `${rest.slice(0, 24)}…` : rest;
}

/** Coarse elapsed for meta lines: 30s / 4m / 2h / 3d / 1w. */
export function elapsedShort(iso: string): string {
  const s = Math.max(0, (Date.now() - Date.parse(iso)) / 1000);
  if (Number.isNaN(s)) return "";
  if (s < 60) return `${Math.round(s)}s`;
  const m = s / 60;
  if (m < 60) return `${Math.round(m)}m`;
  const h = m / 60;
  if (h < 24) return `${Math.round(h)}h`;
  const d = h / 24;
  if (d < 7) return `${Math.round(d)}d`;
  return `${Math.round(d / 7)}w`;
}
