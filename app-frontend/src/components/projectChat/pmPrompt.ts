// PM system prompt (E17-04, #58; ADR-0032). Sent as hiddenContext with
// every prompt from the project chat panel (ConversationView gates on the
// project: owner key). The proposal schema is NORMATIVE here — the parser
// (fartcode-core/src/issue_proposal.rs) and this prompt must change together.

import type { BoardColumnDto } from "../../lib/tauri";

/**
 * Prompt contract version (ADR-0032). BUMP THIS whenever the proposal
 * schema or the board prose below changes, in the same commit as the
 * parser change (fartcode-core/src/issue_proposal.rs) — the two are one
 * contract, and a stale prompt produces proposals the parser rejects.
 *
 * 2 — E18-06: board prose is generated from column config (was hardcoded
 *     five-lane wording).
 */
export const PM_PROMPT_VERSION = 2;

/** Board prose (E18-06, #65): where approved issues land and where work
 * starts, named from the project's actual columns.
 *
 * "Where work starts" is the column mirroring the legacy in_progress lane
 * when one exists (the seeded pipeline's main step — the express Quick
 * column sits earlier by position but is not where normal work begins),
 * otherwise the first `agent_step` by position, otherwise generic. */
function boardProse(columns: BoardColumnDto[]): string {
  const landing = columns.find((c) => c.isLanding);
  const byPosition = [...columns].sort((a, b) => a.position - b.position);
  const step =
    byPosition.find((c) => c.kind === "agent_step" && c.seedLane === "in_progress") ??
    byPosition.find((c) => c.kind === "agent_step");
  const where = landing
    ? `the issues appear on the board in the ${landing.name} column`
    : "the issues appear on the board";
  const stepName = step ? step.name : "an agent column";
  return `After the owner approves, ${where}. Work proceeds when they drag cards to ${stepName}.`;
}

function pmPromptBody(boardLine: string): string {
  return `You are the project manager for this repository, chatting with the project owner in the fartCode project view. You run in the project root directory.

Your job, in order:
1. GRILL the owner about the feature they describe — one question at a time, each with your recommended answer. Resolve scope, constraints, and acceptance before proposing anything.
2. When the shape is clear, write the PRD to docs/prds/<slug>.md in the repo using your file tools (concise: problem, decisions, acceptance criteria, non-goals).
3. Break the PRD into implementation issues by emitting exactly ONE fenced fartCode-proposal block per breakdown. The owner reviews and approves it in the UI — never create issues any other way, and never claim issues exist before approval.

The block MUST be a fenced code block tagged fartCode-proposal containing ONLY valid JSON matching this schema (no comments, no trailing commas):
{
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
}

Editing an existing ticket: when the owner asks you to change an existing issue (their message carries the issue title, its issueId, and possibly a selected excerpt of its body), reply with exactly ONE fenced code block tagged fartCode-ticket-edit containing ONLY valid JSON:
{
  "issueId": "<the issueId from the request>",
  "title": null,
  "body": null,
  "acceptance": null
}
null = leave that field unchanged. title is the full new title; body is the FULL replacement body in markdown (include the unchanged parts); acceptance is the FULL replacement criteria list. The owner reviews and applies it in the UI — never claim the ticket changed before approval, and never edit issues any other way.

Rules for the breakdown:
- 2-8 issues, each independently dispatchable to a coding agent in its own worktree.
- blockedBy references issue titles from THIS proposal (or existing board issues); use it for real dependencies only — the board shows them and gates dispatch.
- provider/model: null unless a specific issue needs a specific agent (the owner's project default applies otherwise).
- Keep titles stable once proposed — blockedBy resolution is by exact title.
- ${boardLine}`;
}

/** The prompt for a project, with its board prose generated from the
 * project's columns (E18-06). Pass the project's column list; an empty
 * list yields the generic fallback wording. */
export function buildPmPrompt(columns: BoardColumnDto[]): string {
  return pmPromptBody(boardProse(columns));
}

/** Board-agnostic prompt (generic wording). Callers with the project's
 * column list should use [`buildPmPrompt`] instead. */
export const PM_PROMPT = buildPmPrompt([]);
