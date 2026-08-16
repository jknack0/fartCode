// #125: "acceptance": [] is a full replacement that clears every existing
// criterion. The card must label that case explicitly ("clears all N
// criteria", never "Acceptance (0)") and require a second apply to confirm.

import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";

vi.mock("../../lib/tauri", () => ({
  issueList: vi.fn(),
  issueUpdate: vi.fn(() => Promise.resolve()),
}));

import TicketEditCard from "./TicketEditCard";
import { issueList, issueUpdate, type IssueDto } from "../../lib/tauri";

function issue(over: Partial<IssueDto> & { id: string }): IssueDto {
  return {
    projectId: "p1",
    title: "the ticket",
    body: null,
    acceptance: [],
    lane: "backlog" as IssueDto["lane"],
    position: 0,
    provider: null,
    model: null,
    prdPath: null,
    prdSection: null,
    dossierPath: null,
    linkedTaskId: null,
    externalRef: null,
    columnId: null,
    blocked: false,
    blockers: [],
    createdAt: null,
    updatedAt: null,
    ...over,
  };
}

const CLEAR_EDIT = JSON.stringify({ issueId: "i1", title: null, body: null, acceptance: [] });

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(issueUpdate).mockResolvedValue(issue({ id: "i1" }));
});

describe("empty acceptance edit (#125)", () => {
  it("labels the empty case as clearing all N criteria, not (0)", async () => {
    vi.mocked(issueList).mockResolvedValue([
      issue({ id: "i1", acceptance: ["a", "b", "c"] }),
    ]);

    render(<TicketEditCard raw={CLEAR_EDIT} projectId="p1" />);

    expect(await screen.findByText("Acceptance — clears all 3 criteria")).toBeInTheDocument();
    expect(screen.queryByText("Acceptance (0)")).toBeNull();
  });

  it("requires a second apply to confirm the clear", async () => {
    vi.mocked(issueList).mockResolvedValue([
      issue({ id: "i1", acceptance: ["a", "b", "c"] }),
    ]);

    render(<TicketEditCard raw={CLEAR_EDIT} projectId="p1" />);

    const apply = await screen.findByRole("button", { name: /apply/ });
    fireEvent.click(apply);

    // First click arms the confirm — nothing applied yet.
    expect(issueUpdate).not.toHaveBeenCalled();
    expect(screen.getByText(/apply again to confirm/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /confirm clear/ }));
    expect(issueUpdate).toHaveBeenCalledWith("i1", { acceptance: [] });
  });

  it("applies a non-empty acceptance edit on the first click", async () => {
    vi.mocked(issueList).mockResolvedValue([
      issue({ id: "i1", acceptance: ["a"] }),
    ]);
    const raw = JSON.stringify({ issueId: "i1", title: null, body: null, acceptance: ["x", "y"] });

    render(<TicketEditCard raw={raw} projectId="p1" />);

    expect(await screen.findByText("Acceptance (2)")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /apply/ }));
    expect(issueUpdate).toHaveBeenCalledWith("i1", { acceptance: ["x", "y"] });
  });

  it("does not demand a confirm when the issue already has no criteria", async () => {
    vi.mocked(issueList).mockResolvedValue([issue({ id: "i1", acceptance: [] })]);

    render(<TicketEditCard raw={CLEAR_EDIT} projectId="p1" />);

    expect(await screen.findByText("Acceptance — empty")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /apply/ }));
    expect(issueUpdate).toHaveBeenCalledWith("i1", { acceptance: [] });
  });
});
