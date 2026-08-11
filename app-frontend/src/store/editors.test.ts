// Editors store (E5-02): pins the save contract — ⌘S writes the LIVE
// editor document (not the loaded snapshot), clears the dirty dot, and a
// failed write surfaces per-tab without losing dirty; ⌘⇧S saves every
// dirty tab; tab ids round-trip through parse (colon-bearing paths
// included).
import { describe, it, expect, vi, beforeEach } from "vitest";

const readWorkspaceFile = vi.fn();
const writeWorkspaceFile = vi.fn();
vi.mock("../lib/tauri", () => ({
  readWorkspaceFile: (...a: unknown[]) => readWorkspaceFile(...a),
  writeWorkspaceFile: (...a: unknown[]) => writeWorkspaceFile(...a),
}));

import { editorTabId, parseEditorTabId } from "../lib/editor-tabs";
import { registerEditorView, useEditors } from "./editors";
import type { EditorView } from "@codemirror/view";

const fakeView = (doc: string) =>
  ({ state: { doc: { toString: () => doc } } }) as unknown as EditorView;

const TAB = editorTabId("ws1", "src/a.ts");

beforeEach(() => {
  readWorkspaceFile.mockReset();
  writeWorkspaceFile.mockReset();
  useEditors.setState({ byTab: {}, previewTabs: {}, dirtyByTab: {}, saveErrorByTab: {} });
});

describe("editor tab ids", () => {
  it("round-trips, colons in paths included", () => {
    expect(parseEditorTabId(TAB)).toEqual({ workspaceId: "ws1", path: "src/a.ts" });
    const weird = editorTabId("ws1", "a:b/c.txt");
    expect(parseEditorTabId(weird)).toEqual({ workspaceId: "ws1", path: "a:b/c.txt" });
    expect(parseEditorTabId("nope")).toBeNull();
  });
});

describe("useEditors", () => {
  it("ensure loads content once", async () => {
    readWorkspaceFile.mockResolvedValue("disk\n");
    await useEditors.getState().ensure(TAB);
    expect(useEditors.getState().byTab[TAB]?.content).toBe("disk\n");
    await useEditors.getState().ensure(TAB);
    expect(readWorkspaceFile).toHaveBeenCalledTimes(1);
  });

  it("save writes the live document and clears dirty", async () => {
    writeWorkspaceFile.mockResolvedValue(undefined);
    registerEditorView(TAB, fakeView("edited\n"));
    useEditors.getState().markDirty(TAB);

    await useEditors.getState().save(TAB);
    expect(writeWorkspaceFile).toHaveBeenCalledWith("ws1", "src/a.ts", "edited\n");
    expect(useEditors.getState().dirtyByTab[TAB]).toBeUndefined();
    expect(useEditors.getState().byTab[TAB]?.content).toBe("edited\n");
  });

  it("failed save keeps dirty and surfaces the error", async () => {
    writeWorkspaceFile.mockRejectedValue("disk full");
    registerEditorView(TAB, fakeView("x"));
    useEditors.getState().markDirty(TAB);

    await useEditors.getState().save(TAB);
    expect(useEditors.getState().dirtyByTab[TAB]).toBe(true);
    expect(useEditors.getState().saveErrorByTab[TAB]).toContain("disk full");
  });

  it("saveAll saves every dirty tab, only dirty tabs", async () => {
    writeWorkspaceFile.mockResolvedValue(undefined);
    const other = editorTabId("ws1", "b.ts");
    const clean = editorTabId("ws1", "c.ts");
    registerEditorView(TAB, fakeView("1"));
    registerEditorView(other, fakeView("2"));
    registerEditorView(clean, fakeView("3"));
    useEditors.getState().markDirty(TAB);
    useEditors.getState().markDirty(other);

    await useEditors.getState().saveAll();
    expect(writeWorkspaceFile).toHaveBeenCalledTimes(2);
    expect(useEditors.getState().dirtyByTab).toEqual({});
  });
});
