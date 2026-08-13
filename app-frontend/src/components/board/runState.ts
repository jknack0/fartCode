// Board run-state vocabulary (design_handoff_v2 frame 4a + handoff v3
// system rules): the card's dot + meta treatment.
//
// It derives from the LIVE SESSION, never from the card's lane or column
// (ADR-0037 item 11 / the TaskHeader-dot finding): `task.status` is frozen
// at `in_progress` from birth — `update_status` has no production callers —
// so keying "running" on it paints every linked card amber forever. The
// truth is a live agent terminal, which the scripts store tracks
// (`agentByTask`, kept fresh by hydrate / noteAgentSpawn / the exit event),
// exactly as the task header's dot does. Status still carries the states
// the terminal cannot: needs-you (review) and failed.
//
// Step-done (accent-filled dot) and queued (dashed ring) are engine states,
// fed in from the step:settled / step:queued events. All of it is derived
// at render time; nothing is stored.

export interface RunState {
  kind:
    | "running"
    | "starting"
    | "needs-you"
    | "failed"
    | "step-done"
    | "stopped"
    | "queued"
    | "neutral";
  /** Modifier class for the canonical .status-dot (styles.css ~556). */
  dot: string;
  /** Meta-line label; null renders no run segment (idle / step-done — the
   * step-done dot and its hint line say it without a word). */
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

/** Everything the card knows about its own run, all of it live. */
export interface RunStateInput {
  /** The linked task's status (needs-you / failed / cancelled only). */
  status: string | undefined;
  /** The task's agent terminal state from the scripts store; `undefined`
   * means the task has not been hydrated yet. */
  agent: { running: boolean } | undefined;
  /** step:settled landed for the card's CURRENT column and that column
   * holds — awaiting a human drag. */
  stepDone: boolean;
  /** step:queued parked a step here; the confirm has not fired yet. */
  queued: boolean;
  /** A dispatch is in flight for this card (drag → prompt pasted). */
  launching?: boolean;
}

/** Is an agent live for this card? The scripts store is the authority once
 * the task is hydrated; before that we fall back to the legacy status test
 * so a cold board reads exactly as it did rather than flickering to idle. */
export function agentLive(
  agent: { running: boolean } | undefined,
  status: string | undefined,
): boolean {
  return agent ? agent.running : status === "in_progress";
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

export function runStateFor(input: RunStateInput): RunState {
  // A dispatch in flight reads "starting" whatever the card carried — the
  // old state is being superseded by the launch.
  if (input.launching) {
    return { ...NEUTRAL, kind: "starting", dot: "run-starting", label: "starting" };
  }
  // A failed run outranks everything — it is the one state with a
  // follow-up action ("↵ read").
  if (input.status === "failed") {
    return {
      ...NEUTRAL,
      kind: "failed",
      dot: "status-failed",
      label: "failed",
      bad: true,
      actionable: true,
    };
  }
  if (agentLive(input.agent, input.status)) {
    return { ...NEUTRAL, kind: "running", dot: "status-in_progress", label: "running" };
  }
  // The agent stopped to ask something: hollow amber, and it stays the
  // card's state even after the terminal exits.
  if (input.status === "review") {
    return { ...NEUTRAL, kind: "needs-you", dot: "status-needs-you", label: "needs you" };
  }
  if (input.queued) {
    return {
      ...NEUTRAL,
      kind: "queued",
      dot: "run-queued",
      label: "queued",
      dimTitle: true,
      dimRow: true,
    };
  }
  // Step settled, column holds: the same accent dot as a passed check.
  if (input.stepDone) {
    return { ...NEUTRAL, kind: "step-done", dot: "run-step-done" };
  }
  if (input.status === "cancelled") {
    return { ...NEUTRAL, kind: "stopped", dot: "run-stopped", label: "stopped", dimTitle: true };
  }
  if (input.status === "todo") {
    return {
      ...NEUTRAL,
      kind: "queued",
      dot: "run-queued",
      label: "queued",
      dimTitle: true,
      dimRow: true,
    };
  }
  return NEUTRAL;
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
