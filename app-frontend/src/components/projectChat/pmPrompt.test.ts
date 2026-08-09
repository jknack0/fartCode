import { describe, it, expect } from "vitest";
import { PM_PROMPT, PM_PROMPT_VERSION, buildPmPrompt } from "./pmPrompt";
import type { BoardColumnDto } from "../../lib/tauri";

/** A column with seed-ish defaults; every test overrides only what it means. */
function col(over: Partial<BoardColumnDto> & { name: string; position: number }): BoardColumnDto {
  return {
    id: `col_${over.name.toLowerCase().replace(/\s+/g, "_")}`,
    projectId: "p1",
    kind: "shelf",
    countsAsDone: false,
    isLanding: false,
    onEnter: "queue",
    onSettle: "hold",
    advanceTo: null,
    stepPrompt: null,
    stepProvider: null,
    stepModel: null,
    stepEffort: null,
    stepTools: null,
    seedLane: null,
    createdAt: null,
    updatedAt: null,
    ...over,
  };
}

/** Mirrors fartcode-core SEED_COLUMNS (issues/columns.rs). Note Quick is an
 * agent_step that sits BEFORE In Progress by position but carries no
 * seed_lane — the prose must still name In Progress. */
function seededColumns(): BoardColumnDto[] {
  return [
    col({ name: "Backlog", position: 0, isLanding: true, seedLane: "backlog" }),
    col({ name: "Ready", position: 1, seedLane: "ready" }),
    col({ name: "Quick", position: 2, kind: "agent_step", onEnter: "run", onSettle: "advance" }),
    col({
      name: "In Progress",
      position: 3,
      kind: "agent_step",
      onEnter: "run",
      onSettle: "advance",
      seedLane: "in_progress",
    }),
    col({ name: "In Review", position: 4, kind: "human_gate", seedLane: "in_review" }),
    col({ name: "Done", position: 5, countsAsDone: true, seedLane: "done" }),
  ];
}

describe("PM_PROMPT_VERSION", () => {
  it("is 2 — bump only alongside the Rust parser (ADR-0032)", () => {
    expect(PM_PROMPT_VERSION).toBe(2);
  });
});

describe("board prose", () => {
  it("names the seeded landing column and In Progress, not the earlier Quick step", () => {
    const prompt = buildPmPrompt(seededColumns());
    expect(prompt).toContain(
      "After the owner approves, the issues appear on the board in the Backlog column. " +
        "Work proceeds when they drag cards to In Progress.",
    );
    // The regression this guards: "first agent_step by position" would pick Quick.
    expect(prompt).not.toContain("drag cards to Quick");
  });

  it("follows the landing flag when it moves to another column", () => {
    const columns = seededColumns().map((c) =>
      c.name === "Backlog"
        ? { ...c, isLanding: false }
        : c.name === "Ready"
          ? { ...c, isLanding: true }
          : c,
    );
    expect(buildPmPrompt(columns)).toContain(
      "the issues appear on the board in the Ready column",
    );
    expect(buildPmPrompt(columns)).not.toContain("in the Backlog column");
  });

  it("falls back to the first agent_step by position when none mirrors in_progress", () => {
    const columns = [
      col({ name: "Inbox", position: 0, isLanding: true }),
      col({ name: "Second Pass", position: 2, kind: "agent_step" }),
      col({ name: "First Pass", position: 1, kind: "agent_step" }),
    ];
    // Array order deliberately disagrees with position order.
    expect(buildPmPrompt(columns)).toContain("drag cards to First Pass");
  });

  it("prefers the in_progress mirror even when it sits last by position", () => {
    const columns = [
      col({ name: "Inbox", position: 0, isLanding: true }),
      col({ name: "Triage", position: 1, kind: "agent_step" }),
      col({ name: "Build", position: 9, kind: "agent_step", seedLane: "in_progress" }),
    ];
    expect(buildPmPrompt(columns)).toContain("drag cards to Build");
  });

  it("says 'an agent column' when the board has no agent step", () => {
    const columns = [
      col({ name: "Inbox", position: 0, isLanding: true }),
      col({ name: "Shipped", position: 1, countsAsDone: true }),
    ];
    const prompt = buildPmPrompt(columns);
    expect(prompt).toContain(
      "After the owner approves, the issues appear on the board in the Inbox column. " +
        "Work proceeds when they drag cards to an agent column.",
    );
  });

  it("drops the column clause entirely when nothing is flagged as landing", () => {
    const columns = seededColumns().map((c) => ({ ...c, isLanding: false }));
    const prompt = buildPmPrompt(columns);
    expect(prompt).toContain(
      "After the owner approves, the issues appear on the board. " +
        "Work proceeds when they drag cards to In Progress.",
    );
    expect(prompt).not.toContain("in the Backlog column");
  });

  it("uses fully generic wording when there are no columns at all", () => {
    expect(buildPmPrompt([])).toContain(
      "After the owner approves, the issues appear on the board. " +
        "Work proceeds when they drag cards to an agent column.",
    );
  });

  it("PM_PROMPT is the board-agnostic build", () => {
    expect(PM_PROMPT).toBe(buildPmPrompt([]));
  });

  it("emits the board line as the last rules bullet", () => {
    const prompt = buildPmPrompt(seededColumns());
    expect(prompt).toContain("\n- After the owner approves,");
    expect(prompt.trimEnd().endsWith("Work proceeds when they drag cards to In Progress.")).toBe(
      true,
    );
  });
});

// The prompt is one half of a contract with fartcode-core/src/issue_proposal.rs
// (serde camelCase). If a field name here drifts, the parser silently drops it
// or rejects the block — assert the schema text verbatim.
describe("fartCode-proposal fence contract", () => {
  const prompt = buildPmPrompt(seededColumns());

  it("instructs a fence tagged fartCode-proposal holding only JSON", () => {
    expect(prompt).toContain("emitting exactly ONE fenced fartCode-proposal block per breakdown");
    expect(prompt).toContain(
      "The block MUST be a fenced code block tagged fartCode-proposal containing ONLY valid JSON matching this schema (no comments, no trailing commas):",
    );
  });

  it("carries the proposal schema byte-for-byte", () => {
    expect(prompt).toContain(
      `{
  "prd": { "path": "docs/prds/<slug>.md", "title": "<PRD title>" },
  "issues": [
    {
      "title": "<short imperative title, unique within the proposal>",
      "body": "<what + why, 1-3 sentences>",
      "acceptance": ["<observable criterion>", "..."],
      "blockedBy": ["<title of an issue that must land first>"],
      "provider": null,
      "model": null
    }
  ]
}`,
    );
  });

  it("carries the ticket-edit schema byte-for-byte", () => {
    expect(prompt).toContain("exactly ONE fenced code block tagged fartCode-ticket-edit");
    expect(prompt).toContain(
      `{
  "issueId": "<the issueId from the request>",
  "title": null,
  "body": null,
  "acceptance": null
}`,
    );
  });

  it("keeps the rules the parser and board rely on", () => {
    // Titles are the blockedBy join key; the parser rejects duplicates.
    expect(prompt).toContain("unique within the proposal");
    expect(prompt).toContain("blockedBy resolution is by exact title");
    expect(prompt).toContain("2-8 issues");
  });

  it("names the PRD path convention the apply step records", () => {
    expect(prompt).toContain("docs/prds/<slug>.md");
  });
});
