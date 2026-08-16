// Diffs store (#130): pins the external-divergence contract — the
// "changed on disk" flag survives until resolved, a successful save is a
// deliberate overwrite (clears dirty AND external), a failed save keeps
// both, and dropTab forgets everything.
import { describe, it, expect, vi, beforeEach } from "vitest";

const writeWorkspaceFile = vi.fn();
vi.mock("../lib/tauri", () => ({
  writeWorkspaceFile: (...a: unknown[]) => writeWorkspaceFile(...a),
  gitFileDiff: vi.fn(),
  getViewState: vi.fn().mockResolvedValue(null),
  setViewState: vi.fn().mockResolvedValue(undefined),
  onFartcodeEvent: vi.fn(),
}));

import { registerDiffView } from "../lib/diff-views";
import { useDiffs, type DiffParams } from "./diffs";
import type { EditorView } from "@codemirror/view";

const fakeView = (doc: string) =>
  ({ state: { doc: { toString: () => doc } } }) as unknown as EditorView;

const TAB = "diff:ws1:unstaged:src/a.ts";
const PARAMS: DiffParams = {
  workspaceId: "ws1",
  path: "src/a.ts",
  origPath: null,
  side: "unstaged",
};

beforeEach(() => {
  writeWorkspaceFile.mockReset();
  useDiffs.setState({
    paramsByTab: {},
    previewTabs: {},
    byTab: {},
    dirtyByTab: {},
    saveErrorByTab: {},
    externalByTab: {},
    selectionByTab: {},
  });
});

describe("external divergence flag (#130)", () => {
  it("marks once and clears", () => {
    useDiffs.getState().markExternal(TAB);
    const first = useDiffs.getState().externalByTab;
    useDiffs.getState().markExternal(TAB);
    expect(useDiffs.getState().externalByTab).toBe(first); // idempotent set
    expect(useDiffs.getState().externalByTab[TAB]).toBe(true);
    useDiffs.getState().clearExternal(TAB);
    expect(useDiffs.getState().externalByTab[TAB]).toBeUndefined();
  });

  it("successful save is a deliberate overwrite: clears dirty and external", async () => {
    writeWorkspaceFile.mockResolvedValue(undefined);
    useDiffs.getState().setParams(TAB, PARAMS);
    registerDiffView(TAB, fakeView("mine\n"));
    useDiffs.getState().markDirty(TAB);
    useDiffs.getState().markExternal(TAB);

    await useDiffs.getState().save(TAB);
    expect(writeWorkspaceFile).toHaveBeenCalledWith("ws1", "src/a.ts", "mine\n");
    expect(useDiffs.getState().dirtyByTab[TAB]).toBeUndefined();
    expect(useDiffs.getState().externalByTab[TAB]).toBeUndefined();
  });

  it("failed save keeps dirty AND the divergence badge", async () => {
    writeWorkspaceFile.mockRejectedValue("disk full");
    useDiffs.getState().setParams(TAB, PARAMS);
    registerDiffView(TAB, fakeView("mine\n"));
    useDiffs.getState().markDirty(TAB);
    useDiffs.getState().markExternal(TAB);

    await useDiffs.getState().save(TAB);
    expect(useDiffs.getState().dirtyByTab[TAB]).toBe(true);
    expect(useDiffs.getState().externalByTab[TAB]).toBe(true);
    expect(useDiffs.getState().saveErrorByTab[TAB]).toContain("disk full");
  });

  it("dropTab forgets the external flag", () => {
    useDiffs.getState().markExternal(TAB);
    useDiffs.getState().dropTab(TAB);
    expect(useDiffs.getState().externalByTab).toEqual({});
  });
});
