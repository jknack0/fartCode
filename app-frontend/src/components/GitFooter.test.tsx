// GitFooter sync segment (#133): the footer hint line finally renders the
// upstream/ahead/behind the DTO has carried since E4-08. The pins below
// (null upstream, missing entry, add-remote form) freeze today's behavior
// before the segment lands; the segment tests assert the exact `↑n ↓n
// <upstream> · ` prefix the dossier's acceptance criteria name.
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen } from "@testing-library/react";

vi.mock("../lib/tauri", () => ({
  // commit-state.ts imports
  gitAddRemote: vi.fn(),
  gitCommit: vi.fn(),
  gitCommitState: vi.fn(),
  gitFetch: vi.fn(),
  gitPublish: vi.fn(),
  gitPull: vi.fn(),
  gitPush: vi.fn(),
  // changes.ts imports
  gitDiscard: vi.fn(),
  gitStage: vi.fn(),
  gitStageAll: vi.fn(),
  gitStatus: vi.fn(),
  gitUnstage: vi.fn(),
  onFartcodeEvent: vi.fn(() => Promise.resolve(() => {})),
}));

// The hint chord comes from the bindings store; a constant keeps the
// exact-text assertions deterministic (dossier plan, risk #2).
vi.mock("../lib/useCommands", () => ({
  hint: vi.fn(() => "⌘K"),
}));

import GitFooter from "./GitFooter";
import { hint } from "../lib/useCommands";
import { useCommitState } from "../store/commit-state";
import type { GitCommitStateDto } from "../lib/tauri";

const BARE_HINT = "d discards after a confirm · fetch / pull / push in ⌘K";

const STATE: GitCommitStateDto = {
  branch: "feat-x",
  remote: "origin",
  hasRemote: true,
  published: true,
  prOpen: false,
  canCreatePr: false,
  upstream: "origin/main",
  ahead: 0,
  behind: 0,
  remotes: ["origin"],
};

function seed(state: Partial<GitCommitStateDto>) {
  useCommitState.setState({
    byWorkspace: { w1: { state: { ...STATE, ...state }, error: null } },
  });
}

function hintText() {
  return document.querySelector(".fc-footer-hint")?.textContent;
}

beforeEach(() => {
  useCommitState.setState({ byWorkspace: {} });
});

describe("GitFooter", () => {
  it("prefixes the hint line with ↑n ↓n upstream when the branch tracks one", () => {
    seed({ upstream: "origin/main", ahead: 3, behind: 2 });
    render(<GitFooter workspaceId="w1" />);
    expect(hintText()).toBe(`↑3 ↓2 origin/main · ${BARE_HINT}`);
  });

  it("shows ↑0 ↓0 rather than hiding zero counts when synced", () => {
    seed({ upstream: "origin/main", ahead: 0, behind: 0 });
    render(<GitFooter workspaceId="w1" />);
    expect(hintText()).toBe(`↑0 ↓0 origin/main · ${BARE_HINT}`);
  });

  it("renders the bare hint line when upstream is null", () => {
    seed({ upstream: null, ahead: 0, behind: 0 });
    render(<GitFooter workspaceId="w1" />);
    expect(hintText()).toBe(BARE_HINT);
  });

  it("renders the bare hint line without throwing when no state entry exists", () => {
    render(<GitFooter workspaceId="w1" />);
    expect(hintText()).toBe(BARE_HINT);
  });

  // Adversarial finding 1, human-blessed: a local-tracking branch
  // (branch.X.remote = ".") yields upstream without any remote — both the
  // add-remote form and the sync segment render, and both are truthful.
  it("renders the sync segment alongside the add-remote form for a local-tracking upstream", () => {
    seed({ upstream: "main", ahead: 1, behind: 4, remotes: [] });
    render(<GitFooter workspaceId="w1" />);
    expect(hintText()).toBe(`↑1 ↓4 main · ${BARE_HINT}`);
    expect(screen.getByLabelText("Remote name")).toBeInTheDocument();
  });

  // Adversarial finding 3: the ⌘K fallback was dead under a truthy mock.
  it("falls back to ⌘K when no palette binding is configured", () => {
    vi.mocked(hint).mockReturnValue("");
    seed({ upstream: null });
    render(<GitFooter workspaceId="w1" />);
    expect(hintText()).toBe(BARE_HINT);
  });

  it("still shows the add-remote form when remotes is empty", () => {
    seed({ upstream: null, remotes: [] });
    render(<GitFooter workspaceId="w1" />);
    expect(screen.getByLabelText("Remote name")).toBeInTheDocument();
    expect(screen.getByLabelText("Remote URL")).toBeInTheDocument();
  });
});
