// Diff view (E4-04 render, E4-05 edit): two-sided diff via
// @codemirror/merge. Split mode mounts a MergeView (baseline | worktree);
// unified mode mounts a single EditorView with the unifiedMergeView
// extension. Payloads come from the diffs store (git_file_diff); the tab id
// carries the params so restored tabs work without sidecar state.
//
// E4-05: the worktree side of an *unstaged* diff is editable — ⌘S (bound in
// the editor, so it only fires when the diff editor is focused) writes the
// document to disk via write_workspace_file, and the watcher's
// files:changed event drives the refresh. Refresh semantics:
//  - a refresh whose content matches the editor's current document (the
//    post-save echo) is skipped — the view, cursor, and scroll survive;
//  - a refresh while the tab is dirty is deferred (the in-progress edit
//    wins over a clobbering rebuild);
//  - a genuine external change rebuilds with scroll + selection preserved.
// Hidden tabs (display:none) never build: the view is only constructed
// while the tab is active.
import { useEffect, useRef, useState } from "react";
import { EditorState, type Extension } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { MergeView, unifiedMergeView } from "@codemirror/merge";
import { LanguageDescription } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { oneDark } from "@codemirror/theme-one-dark";
import { basicSetup } from "codemirror";
import DiffSelectionPopover from "./DiffSelectionPopover";
import { parseDiffTabId } from "../lib/diff-tabs";
import {
  diffViewDocs,
  registerDiffView,
  unregisterDiffView,
} from "../lib/diff-views";
import { useDiffs, type DiffParams } from "../store/diffs";

function formatSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${bytes} B`;
}

function baseExtensions(lang: Extension[]): Extension[] {
  return [basicSetup, oneDark, ...lang];
}

function readOnlyExtensions(): Extension[] {
  return [EditorState.readOnly.of(true), EditorView.editable.of(false)];
}

function editableExtensions(tabId: string): Extension[] {
  return [
    EditorView.updateListener.of((update) => {
      if (update.docChanged) useDiffs.getState().markDirty(tabId);
    }),
    keymap.of([
      {
        key: "Mod-s",
        run: () => {
          void useDiffs.getState().save(tabId);
          return true;
        },
      },
    ]),
  ];
}

/** Reports text selections in any diff editor to the diffs store (the
 * "Ask agent" popover reads them). Selection text is capped. */
const MAX_SELECTION_CHARS = 4000;

function selectionExtensions(tabId: string, side: "a" | "b" | "single"): Extension[] {
  return [
    EditorView.updateListener.of((update) => {
      if (!update.selectionSet && !update.docChanged) return;
      const sel = update.state.selection.main;
      if (sel.empty) {
        const prev = useDiffs.getState().selectionByTab[tabId];
        if (prev && prev.side === side) useDiffs.getState().setSelection(tabId, null);
        return;
      }
      useDiffs.getState().setSelection(tabId, {
        side,
        from: sel.from,
        to: sel.to,
        fromLine: update.state.doc.lineAt(sel.from).number,
        toLine: update.state.doc.lineAt(sel.to).number,
        text: update.state.sliceDoc(
          sel.from,
          Math.min(sel.to, sel.from + MAX_SELECTION_CHARS),
        ),
      });
    }),
  ];
}

export default function DiffView({
  tabId,
  title,
  taskId,
  active,
}: {
  tabId: string;
  title: string;
  taskId: string;
  active: boolean;
}) {
  const storeParams = useDiffs((s) => s.paramsByTab[tabId]);
  const parsed = parseDiffTabId(tabId);
  const params: DiffParams | null = storeParams ??
    (parsed ? { ...parsed, origPath: null } : null);
  const entry = useDiffs((s) => s.byTab[tabId]);
  const dirty = useDiffs((s) => !!s.dirtyByTab[tabId]);
  const saveError = useDiffs((s) => s.saveErrorByTab[tabId] ?? null);
  const mode = useDiffs((s) => s.mode);
  const setMode = useDiffs((s) => s.setMode);

  const containerRef = useRef<HTMLDivElement | null>(null);
  const viewRef = useRef<EditorView | MergeView | null>(null);
  const [buildError, setBuildError] = useState<string | null>(null);

  // Restored tabs: register id-parsed params so event refresh finds them.
  useEffect(() => {
    if (params && !useDiffs.getState().paramsByTab[tabId]) {
      useDiffs.getState().setParams(tabId, params);
    }
  }, [tabId, params]);

  useEffect(() => {
    if (!params) return;
    void useDiffs.getState().ensure(tabId, params);
    void useDiffs.getState().ensureMode();
  }, [tabId, params]);

  const payload = entry?.payload ?? null;
  const singleDoc = payload ? !payload.oldExists || !payload.newExists : false;
  const editable =
    params?.side === "unstaged" &&
    !!payload?.newExists &&
    !payload?.binary &&
    !payload?.tooLarge;

  // (Re)build the CodeMirror view. Keyed on payload identity + mode +
  // active; only builds while visible. Destruction happens inside the
  // winning build (never in the effect cleanup) so a skipped rebuild keeps
  // the live editor untouched.
  useEffect(() => {
    const container = containerRef.current;
    if (!container || !active || !payload || payload.binary || payload.tooLarge) return;

    // Post-save echo / no-op refresh: the editor already shows exactly
    // this content in the requested mode — keep the view (cursor, scroll,
    // undo history). A mode flip (unified↔split) is NOT a no-op: the view
    // kind must match too.
    if (viewRef.current) {
      const wantMerge = !!(payload.oldExists && payload.newExists && mode === "split");
      const haveMerge = viewRef.current instanceof MergeView;
      const current = diffViewDocs(viewRef.current);
      const baseline = payload.oldContent ?? "";
      const worktree = payload.newExists
        ? (payload.newContent ?? "")
        : (payload.oldContent ?? "");
      if (
        wantMerge === haveMerge &&
        current.b === worktree &&
        (current.a === null || current.a === baseline)
      ) {
        return;
      }
      // An in-progress edit wins over a clobbering rebuild.
      if (useDiffs.getState().dirtyByTab[tabId]) return;
    }

    let cancelled = false;

    const build = async () => {
      let lang: Extension[] = [];
      const match = LanguageDescription.matchFilename(languages, payload.path);
      if (match) {
        try {
          lang = [await match.load()];
        } catch {
          lang = []; // unknown grammar renders as plain text
        }
      }
      if (cancelled) return;

      // Preserve viewport + selection across the rebuild (external change).
      const previous = viewRef.current;
      const scrollTop = previous
        ? (previous instanceof MergeView ? previous.b : previous).scrollDOM.scrollTop
        : 0;
      const selHead = previous
        ? (previous instanceof MergeView ? previous.b : previous).state.selection.main.head
        : 0;

      const oldDoc = payload.oldContent ?? "";
      const newDoc = payload.newContent ?? "";
      try {
        let view: EditorView | MergeView;
        if (payload.oldExists && payload.newExists && mode === "split") {
          view = new MergeView({
            a: {
              doc: oldDoc,
              extensions: [
                ...baseExtensions(lang),
                ...readOnlyExtensions(),
                ...selectionExtensions(tabId, "a"),
              ],
            },
            b: {
              doc: newDoc,
              extensions: [
                ...baseExtensions(lang),
                ...(editable ? editableExtensions(tabId) : readOnlyExtensions()),
                ...selectionExtensions(tabId, "b"),
              ],
            },
            parent: container,
            gutter: true,
            highlightChanges: true,
            collapseUnchanged: { margin: 3, minSize: 4 },
          });
        } else if (payload.oldExists && payload.newExists) {
          view = new EditorView({
            doc: newDoc,
            extensions: [
              ...baseExtensions(lang),
              ...(editable ? editableExtensions(tabId) : readOnlyExtensions()),
              ...selectionExtensions(tabId, "single"),
              unifiedMergeView({
                original: oldDoc,
                highlightChanges: true,
                gutter: true,
                mergeControls: false,
              }),
            ],
            parent: container,
          });
        } else {
          // Added / deleted: single document, no merge chrome.
          view = new EditorView({
            doc: payload.newExists ? newDoc : oldDoc,
            extensions: [
              ...baseExtensions(lang),
              ...(editable ? editableExtensions(tabId) : readOnlyExtensions()),
              ...selectionExtensions(tabId, "single"),
            ],
            parent: container,
          });
        }

        previous?.destroy();
        if (previous) unregisterDiffView(tabId, previous);
        viewRef.current = view;
        registerDiffView(tabId, view);

        const editor = view instanceof MergeView ? view.b : view;
        editor.scrollDOM.scrollTop = scrollTop;
        const head = Math.min(selHead, editor.state.doc.length);
        if (head > 0) editor.dispatch({ selection: { anchor: head } });

        setBuildError(null);
      } catch (e) {
        setBuildError(String(e));
      }
    };

    void build();
    return () => {
      cancelled = true;
    };
  }, [tabId, active, payload, mode, editable]);

  // Tear the view down on unmount (the keyed effect above only destroys
  // when replacing, so hidden/unbuilt tabs are covered here).
  useEffect(
    () => () => {
      const view = viewRef.current;
      if (view) {
        unregisterDiffView(tabId, view);
        view.destroy();
        viewRef.current = null;
      }
    },
    [tabId],
  );

  return (
    <div className="diff-view">
      <div className="diff-header">
        <span className="diff-path" title={payload?.origPath ?? params?.path ?? title}>
          {payload?.origPath ? (
            <>
              <span className="diff-orig">{payload.origPath}</span>
              <span className="diff-arrow">→</span>
              {payload.path}
            </>
          ) : (
            (params?.path ?? title)
          )}
        </span>
        {params && (
          <span className={`diff-side-badge side-${params.side}`}>
            {params.side === "staged" ? "Staged" : "Unstaged"}
          </span>
        )}
        {payload && !payload.oldExists && <span className="diff-badge added">Added</span>}
        {payload && !payload.newExists && <span className="diff-badge deleted">Deleted</span>}
        {dirty && <span className="diff-badge dirty" title="Unsaved changes — ⌘S to save">●</span>}
        {saveError && (
          <span className="diff-save-error" title={saveError}>
            Save failed
          </span>
        )}
        {payload && !singleDoc && !payload.binary && !payload.tooLarge && (
          <div className="diff-mode-toggle" role="group" aria-label="Diff mode">
            <button
              className={mode === "unified" ? "active" : undefined}
              onClick={() => setMode("unified")}
            >
              Unified
            </button>
            <button
              className={mode === "split" ? "active" : undefined}
              onClick={() => setMode("split")}
            >
              Split
            </button>
          </div>
        )}
      </div>

      {!params ? (
        <p className="diff-notice">Unknown diff target.</p>
      ) : entry?.loading && !payload ? (
        <p className="diff-notice">Loading diff…</p>
      ) : entry?.error && !payload ? (
        <div className="diff-notice">
          <p className="error">{entry.error}</p>
          <button onClick={() => void useDiffs.getState().refresh(tabId)}>Retry</button>
        </div>
      ) : payload?.binary ? (
        <p className="diff-notice">Binary file — preview unavailable.</p>
      ) : payload?.tooLarge ? (
        <p className="diff-notice">
          Diff too large ({formatSize(Math.max(payload.oldSize, payload.newSize))}) — preview
          unavailable.
        </p>
      ) : buildError ? (
        <p className="diff-notice error">{buildError}</p>
      ) : payload ? (
        <div className="diff-body" ref={containerRef}>
          <DiffSelectionPopover
            tabId={tabId}
            params={params}
            taskId={taskId}
            containerRef={containerRef}
          />
        </div>
      ) : (
        <p className="diff-notice">Loading diff…</p>
      )}
    </div>
  );
}
