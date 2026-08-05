// Diff view (E4-04): read-only two-sided diff rendered with
// @codemirror/merge. Split mode mounts a MergeView (original | modified);
// unified mode mounts a single EditorView with the unifiedMergeView
// extension. Payloads come from the diffs store (git_file_diff); the tab id
// carries the params so restored tabs work without sidecar state.
//
// Hidden tabs (display:none) must never be measured into a zero-size view:
// the MergeView is only constructed while the tab is active, and a re-show
// triggers requestMeasure so CodeMirror recovers its layout.
import { useEffect, useRef, useState } from "react";
import { EditorState, type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { MergeView, unifiedMergeView } from "@codemirror/merge";
import { LanguageDescription } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { oneDark } from "@codemirror/theme-one-dark";
import { basicSetup } from "codemirror";
import { parseDiffTabId } from "../lib/diff-tabs";
import { useDiffs, type DiffParams } from "../store/diffs";

function formatSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${bytes} B`;
}

function singleDocExtensions(lang: Extension[]) {
  return [
    basicSetup,
    EditorState.readOnly.of(true),
    EditorView.editable.of(false),
    oneDark,
    ...lang,
  ];
}

export default function DiffView({
  tabId,
  title,
  active,
}: {
  tabId: string;
  title: string;
  active: boolean;
}) {
  const storeParams = useDiffs((s) => s.paramsByTab[tabId]);
  const parsed = parseDiffTabId(tabId);
  const params: DiffParams | null = storeParams ??
    (parsed ? { ...parsed, origPath: null } : null);
  const entry = useDiffs((s) => s.byTab[tabId]);
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

  // (Re)build the CodeMirror view. Keyed on payload identity + mode +
  // active; only builds while visible so hidden mounts never measure zero.
  useEffect(() => {
    const container = containerRef.current;
    if (!container || !active || !payload || payload.binary || payload.tooLarge) return;

    let cancelled = false;
    const destroyView = () => {
      viewRef.current?.destroy();
      viewRef.current = null;
    };

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
      destroyView();
      try {
        const oldDoc = payload.oldContent ?? "";
        const newDoc = payload.newContent ?? "";
        if (payload.oldExists && payload.newExists && mode === "split") {
          viewRef.current = new MergeView({
            a: { doc: oldDoc, extensions: singleDocExtensions(lang) },
            b: { doc: newDoc, extensions: singleDocExtensions(lang) },
            parent: container,
            gutter: true,
            highlightChanges: true,
            collapseUnchanged: { margin: 3, minSize: 4 },
          });
        } else if (payload.oldExists && payload.newExists) {
          viewRef.current = new EditorView({
            doc: newDoc,
            extensions: [
              ...singleDocExtensions(lang),
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
          viewRef.current = new EditorView({
            doc: payload.newExists ? newDoc : oldDoc,
            extensions: singleDocExtensions(lang),
            parent: container,
          });
        }
        setBuildError(null);
      } catch (e) {
        setBuildError(String(e));
      }
    };

    void build();
    return () => {
      cancelled = true;
      destroyView();
    };
  }, [tabId, active, payload, mode]);

  // Tear the view down on unmount (the keyed effect above skips hidden
  // builds, so its cleanup doesn't run when the tab never activated).
  useEffect(
    () => () => {
      viewRef.current?.destroy();
      viewRef.current = null;
    },
    [],
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
        <div className="diff-body" ref={containerRef} />
      ) : (
        <p className="diff-notice">Loading diff…</p>
      )}
    </div>
  );
}
