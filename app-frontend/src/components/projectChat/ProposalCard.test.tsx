// ProposalCard copy comes from column config (ADR-0037 item 7, #68):
// "approve N → <landing column>" and "added to <landing column>" name the
// board's is_landing column, never a hardcoded "Backlog". Until the
// columns load, the copy stays generic — it must never name a wrong
// column. The backend already lands on the is_landing column, so this is
// display truth only.

import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

vi.mock("../../lib/tauri", () => ({
  issueParseProposal: vi.fn(),
  issueApplyProposal: vi.fn(),
  columnList: vi.fn(() => Promise.resolve([])),
  onFartcodeEvent: vi.fn(() => Promise.resolve(() => {})),
}));

import ProposalCard from "./ProposalCard";
import { columnList, issueParseProposal } from "../../lib/tauri";
import type { BoardColumnDto, ProposalDto } from "../../lib/tauri";
import { useColumns } from "../../store/columns";

function column(
  over: Partial<BoardColumnDto> & { id: string; name: string; position: number },
): BoardColumnDto {
  return {
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

const PROPOSAL: ProposalDto = {
  prd: null,
  issues: [
    { title: "one", body: null, acceptance: [], blockedBy: [], provider: null, model: null },
    { title: "two", body: null, acceptance: [], blockedBy: [], provider: null, model: null },
  ],
};

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(issueParseProposal).mockResolvedValue(PROPOSAL);
  vi.mocked(columnList).mockResolvedValue([]);
  useColumns.setState({ byProject: {}, loading: {}, loaded: {}, error: null });
});

describe("approve copy names the landing column", () => {
  it("uses the is_landing column's name from config", async () => {
    vi.mocked(columnList).mockResolvedValue([
      column({ id: "c-inbox", name: "Inbox", position: 0, isLanding: true }),
      column({ id: "c-done", name: "Done", position: 1, countsAsDone: true }),
    ]);

    render(<ProposalCard raw="whatever" projectId="p1" />);

    await waitFor(() =>
      expect(screen.getByRole("button", { name: /approve 2 → Inbox/ })).toBeInTheDocument(),
    );
  });

  it("stays generic while columns are not loaded, never naming a wrong column", async () => {
    // columnList never resolves — the card renders before any column truth.
    vi.mocked(columnList).mockReturnValue(new Promise(() => {}));

    render(<ProposalCard raw="whatever" projectId="p1" />);

    const button = await screen.findByRole("button", { name: /approve 2/ });
    expect(button.textContent).not.toContain("Backlog");
    expect(button.textContent).not.toContain("→");
  });
});
