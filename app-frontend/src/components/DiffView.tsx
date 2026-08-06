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
import { Compartment, EditorState, RangeSet, type Extension, type Range } from "@codemirror/state";
import { EditorView, GutterMarker, gutter, keymap } from "@codemirror/view";
import { MergeView, unifiedMergeView } from "@codemirror/merge";
import { LanguageDescription } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { oneDark } from "@codemirror/theme-one-dark";
import { basicSetup } from "codemirror";
import DiffSelectionPopover from "./DiffSelectionPopover";
import CommentThread from "./CommentThread";
import { parseDiffTabId } from "../lib/diff-tabs";
import {
  diffViewDocs,
  registerDiffView,
  unregisterDiffView,
} from "../lib/diff-views";
import { useDiffs, type DiffParams } from "../store/diffs";
import { useLineComments } from "../store/line-comments";
import type { LineCommentDto } from "../lib/tauri";

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

// -- E4-10 comment markers (§14 gutter) ----------------------------------------

class CommentMarker extends GutterMarker {
  constructor(
    private resolved: boolean,
    private openThread: () => void,
  ) {
    super();
  }
  toDOM() {
    const span = document.createElement("span");
    span.className = this.resolved ? "comment-marker resolved" : "comment-marker";
    span.textContent = this.resolved ? "✓" : "💬";
    span.title = this.resolved ? "Resolved comment" : "Comment";
    span.addEventListener("click", (e) => {
      e.stopPropagation();
      this.openThread();
    });
    return span;
  }
}

/** Gutter extension showing one marker per comment's anchor line on the
 * given side. Reconfigured via its Compartment when comments change. */
function commentGutter(
  comments: LineCommentDto[],
  side: "before" | "after",
  openThread: () => void,
): Extension {
  return gutter({
    class: "cm-comment-gutter",
    markers: (view) => {
      const ranges: Range<GutterMarker>[] = [];
      for (const c of comments) {
        if (c.sourceSide !== side) continue;
        if (c.lineNumber < 1 || c.lineNumber > view.state.doc.lines) continue;
        ranges.push(
          new CommentMarker(c.resolved, openThread).range(
            view.state.doc.line(c.lineNumber).from,
          ),
        );
      }
      return RangeSet.of(ranges);
    },
  });
}

/** One mounted editor + the compartment owning its comment markers. */
interface MarkerMount {
  editor: EditorView;
  compartment: Compartment;
  side: "before" | "after";
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
  const markerMountsRef = useRef<MarkerMount[]>([]);
  const [buildError, setBuildError] = useState<string | null>(null);
  const [thread, setThread] = useState<"before" | "after" | null>(null);

  // E4-10: load this task's comments (restart persistence — the gutter
  // rehydrates from SQLite every time a diff opens).
  const comments = useLineComments((s) => (taskId ? s.byTask[taskId] : undefined)) ?? [];
  useEffect(() => {
    void useLineComments.getState().ensure(taskId);
  }, [taskId]);

  // Reconfigure mounted gutters whenever the comment list changes.
  useEffect(() => {
    for (const mount of markerMountsRef.current) {
      mount.editor.dispatch({
        effects: mount.compartment.reconfigure(
          commentGutter(comments, mount.side, () => setThread(mount.side)),
        ),
      });
    }
  }, [comments]);

  // The popover's "Add Note" / "Create Task" flows raise the thread.
  useEffect(() => {
    const onShow = (e: Event) => {
      const detail = (e as CustomEvent).detail as
        | { taskId: string; filePath: string; side: "before" | "after" }
        | undefined;
      if (detail && detail.taskId === taskId && detail.filePath === params?.path) {
        setThread(detail.side);
      }
    };
    window.addEventListener("ade:show-comments", onShow);
    return () => window.removeEventListener("ade:show-comments", onShow);
  }, [taskId, params?.path]);

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
    // E4-10: each editor gets a comment-marker gutter in its own
    // compartment; the comments effect reconfigures on change.
    const aCompartment = new Compartment();
    const bCompartment = new Compartment();
    const singleCompartment = new Compartment();
    const openThreadFor = (side: "before" | "after") => () => setThread(side);

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
                aCompartment.of(commentGutter(comments, "before", openThreadFor("before"))),
              ],
            },
            b: {
              doc: newDoc,
              extensions: [
                ...baseExtensions(lang),
                ...(editable ? editableExtensions(tabId) : readOnlyExtensions()),
                ...selectionExtensions(tabId, "b"),
                bCompartment.of(commentGutter(comments, "after", openThreadFor("after"))),
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
              singleCompartment.of(commentGutter(comments, "after", openThreadFor("after"))),
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
              singleCompartment.of(commentGutter(comments, "after", openThreadFor("after"))),
            ],
            parent: container,
          });
        }

        previous?.destroy();
        if (previous) unregisterDiffView(tabId, previous);
        viewRef.current = view;
        registerDiffView(tabId, view);

        // Register comment-gutter mounts so the comments effect can
        // reconfigure them without a rebuild.
        if (view instanceof MergeView) {
          markerMountsRef.current = [
            { editor: view.a, compartment: aCompartment, side: "before" },
            { editor: view.b, compartment: bCompartment, side: "after" },
          ];
        } else {
          markerMountsRef.current = [
            { editor: view, compartment: singleCompartment, side: "after" },
          ];
        }

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
      markerMountsRef.current = [];
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
      markerMountsRef.current = [];
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
          {thread && params && (
            <CommentThread
              taskId={taskId}
              filePath={params.path}
              side={thread}
              onClose={() => setThread(null)}
            />
          )}
        </div>
      ) : (
        <p className="diff-notice">Loading diff…</p>
      )}
    </div>
  );
}
