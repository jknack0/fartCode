// File tree (E5-01): pins the three behaviors the ticket promises — lazy
// listing renders, expanding a dir loads its children, changed paths (and
// their ancestor dirs) carry the highlight class, and a files:changed
// event refetches loaded dirs without a poll.
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, act, fireEvent, waitFor } from "@testing-library/react";
import type { DirEntryDto, FartcodeEvent } from "../lib/tauri";

let emit: (e: FartcodeEvent) => void = () => {};
const listWorkspaceDir = vi.fn();

vi.mock("../lib/tauri", () => ({
  listWorkspaceDir: (...a: unknown[]) => listWorkspaceDir(...a),
  onFartcodeEvent: vi.fn((cb: (e: FartcodeEvent) => void) => {
    emit = cb;
    return Promise.resolve(() => {});
  }),
}));

vi.mock("../store/changes", () => {
  const state = {
    byWorkspace: {
      ws1: {
        snapshot: {
          staged: [],
          unstaged: [{ path: "src/deep/changed.ts" }],
          stagedAdditions: 0,
          stagedDeletions: 0,
          truncated: false,
        },
        loading: false,
        error: null,
      },
    },
    ensure: vi.fn(() => Promise.resolve()),
  };
  const useChanges = (sel: (s: typeof state) => unknown) => sel(state);
  useChanges.getState = () => state;
  return { useChanges };
});

import FileTreeView from "./FileTreeView";

const root: DirEntryDto[] = [
  { name: "src", isDir: true },
  { name: "README.md", isDir: false },
];
const srcDir: DirEntryDto[] = [{ name: "deep", isDir: true }];

beforeEach(() => {
  listWorkspaceDir.mockReset();
  listWorkspaceDir.mockImplementation((_ws: string, dir: string) =>
    Promise.resolve(dir === "" ? root : dir === "src" ? srcDir : []),
  );
});

describe("FileTreeView", () => {
  it("renders the root listing and lazy-expands a dir on click", async () => {
    render(<FileTreeView taskId="t1" workspaceId="ws1" active />);
    await screen.findByText("README.md");
    expect(listWorkspaceDir).toHaveBeenCalledWith("ws1", "");
    expect(screen.queryByText("deep")).toBeNull();

    fireEvent.click(screen.getByText("src"));
    await screen.findByText("deep");
    expect(listWorkspaceDir).toHaveBeenCalledWith("ws1", "src");
  });

  it("tints changed ancestor dirs", async () => {
    render(<FileTreeView taskId="t1" workspaceId="ws1" active />);
    const src = await screen.findByText("src");
    expect(src.className).toContain("ft-changed");
    const readme = screen.getByText("README.md");
    expect(readme.className).not.toContain("ft-changed");
  });

  it("refetches loaded dirs on a files:changed event for its workspace", async () => {
    render(<FileTreeView taskId="t1" workspaceId="ws1" active />);
    await screen.findByText("README.md");
    listWorkspaceDir.mockClear();

    await act(async () => {
      emit({ type: "files:changed", workspaceId: "other", paths: [] });
    });
    expect(listWorkspaceDir).not.toHaveBeenCalled();

    await act(async () => {
      emit({ type: "files:changed", workspaceId: "ws1", paths: ["x"] });
    });
    await waitFor(() => expect(listWorkspaceDir).toHaveBeenCalledWith("ws1", ""));
  });
});
