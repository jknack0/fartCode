// File editor tab (E5-02): one CodeMirror 6 EditorView per tab, basicSetup
// only (E5-06 adds highlighting/find). The store owns disk content and
// dirty/preview/save bookkeeping; the view owns live text. ⌘S saves this
// tab, ⌘⇧S saves all dirty tabs; the first edit flips a preview tab
// persistent (diff-tab semantics). Disk changes arriving while the tab is
// CLEAN reset the document; a dirty tab never gets clobbered.
import { useEffect, useRef } from "react";
import { basicSetup } from "codemirror";
import { EditorState } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { onFartcodeEvent } from "../lib/tauri";
import { parseEditorTabId } from "../lib/editor-tabs";
import {
  registerEditorView,
  unregisterEditorView,
  useEditors,
} from "../store/editors";

interface Props {
  tabId: string;
  active: boolean;
}

export default function FileEditorView({ tabId, active }: Props) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const viewRef = useRef<EditorView | null>(null);
  const entry = useEditors((s) => s.byTab[tabId]);
  const dirty = useEditors((s) => s.dirtyByTab[tabId] ?? false);
  const saveError = useEditors((s) => s.saveErrorByTab[tabId] ?? null);
  const content = entry?.content ?? null;

  useEffect(() => {
    void useEditors.getState().ensure(tabId);
  }, [tabId]);

  // Refetch on the watcher's files:changed for this file — only while
  // clean (see header comment). The post-save echo is a no-op: content
  // already equals the doc.
  useEffect(() => {
    const params = parseEditorTabId(tabId);
    if (!params) return;
    const un = onFartcodeEvent((ev) => {
      if (
        ev.type === "files:changed" &&
        ev.workspaceId === params.workspaceId &&
        ev.paths.includes(params.path) &&
        !useEditors.getState().dirtyByTab[tabId]
      ) {
        const s = useEditors.getState();
        s.dropTab(tabId);
        void s.ensure(tabId);
      }
    });
    return () => {
      void un.then((f) => f());
    };
  }, [tabId]);

  // Mount once per loaded content generation; content flips only via
  // ensure/refetch (never per keystroke — the view owns live text).
  useEffect(() => {
    if (content === null || !hostRef.current) return;
    const state = EditorState.create({
      doc: content,
      extensions: [
        basicSetup,
        keymap.of([
          {
            key: "Mod-s",
            run: () => {
              void useEditors.getState().save(tabId);
              return true;
            },
          },
          {
            key: "Mod-Shift-s",
            run: () => {
              void useEditors.getState().saveAll();
              return true;
            },
          },
        ]),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) {
            const s = useEditors.getState();
            s.markDirty(tabId);
            if (s.previewTabs[tabId]) s.setPreview(tabId, false);
          }
        }),
      ],
    });
    const view = new EditorView({ state, parent: hostRef.current });
    viewRef.current = view;
    registerEditorView(tabId, view);
    return () => {
      unregisterEditorView(tabId);
      view.destroy();
      viewRef.current = null;
    };
  }, [tabId, content]);

  const params = parseEditorTabId(tabId);
  return (
    <div className="file-editor" data-active={active || undefined}>
      <div className="fe-header">
        <span className="fe-path">{params?.path ?? tabId}</span>
        {dirty && (
          <span className="fe-dirty" title="Unsaved changes — ⌘S to save">
            ●
          </span>
        )}
        {saveError && (
          <span className="fe-save-error" title={saveError}>
            save failed
          </span>
        )}
      </div>
      {entry?.error && <div className="fe-error">{entry.error}</div>}
      {entry?.loading && !content && <div className="fe-loading">loading…</div>}
      <div ref={hostRef} className="fe-host" />
    </div>
  );
}
