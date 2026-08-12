// File tree panel (E5-01): lazy directory tree for a task's workspace.
// Listing comes from `list_workspace_dir` (containment + hidden-dir filter
// backend-side); git-changed highlight borrows the changes store's status
// snapshot (E4-03) — never its own git call — and loaded dirs refetch on
// `files:changed` / `git:changed` bus events, never on a poll. Tree state
// (expanded dirs, loaded children) lives in the component: tabs stay
// mounted across switches, so the tree survives a tab flip for free.
import { useEffect, useMemo, useRef, useState } from "react";
import {
  listWorkspaceDir,
  onFartcodeEvent,
  type DirEntryDto,
} from "../lib/tauri";
import { emitOpenFile } from "../lib/open-file";
import { useChanges } from "../store/changes";

interface Props {
  taskId: string;
  workspaceId: string;
  active: boolean;
}

/** children keyed by dir path ("" = root); missing key = not loaded yet. */
type Children = Record<string, DirEntryDto[] | undefined>;

export default function FileTreeView({ taskId, workspaceId, active }: Props) {
  const [children, setChildren] = useState<Children>({});
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set([""]));
  const [error, setError] = useState<string | null>(null);
  // Single click selects (highlight only); double click opens the file in
  // the main content area (editor tab via the open-file intent).
  const [selected, setSelected] = useState<string | null>(null);

  const snapshot = useChanges((s) => s.byWorkspace[workspaceId]?.snapshot ?? null);
  // Changed paths + every ancestor dir, so a change deep in a collapsed
  // branch still tints the visible dir row.
  const changed = useMemo(() => {
    const set = new Set<string>();
    for (const c of [...(snapshot?.staged ?? []), ...(snapshot?.unstaged ?? [])]) {
      set.add(c.path);
      const parts = c.path.split("/");
      for (let i = 1; i < parts.length; i++) set.add(parts.slice(0, i).join("/"));
    }
    return set;
  }, [snapshot]);

  const load = (dir: string) => {
    listWorkspaceDir(workspaceId, dir)
      .then((rows) => {
        setChildren((c) => ({ ...c, [dir]: rows }));
        setError(null);
      })
      .catch((e) => setError(String(e)));
  };

  useEffect(() => {
    load("");
    void useChanges.getState().ensure(workspaceId);
    // Live refresh: refetch every LOADED dir on this workspace's events.
    // Cheap (loaded set is what's on screen) and self-healing — no path
    // diffing against the event payload needed.
    let loaded = () => new Set(Object.keys(childrenRef.current));
    const un = onFartcodeEvent((ev) => {
      if (
        (ev.type === "files:changed" || ev.type === "git:changed") &&
        ev.workspaceId === workspaceId
      ) {
        for (const dir of loaded()) load(dir);
      }
    });
    return () => {
      void un.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId]);

  // Ref mirror so the event handler sees current loaded dirs without
  // resubscribing per load.
  const childrenRef = useRef(children);
  childrenRef.current = children;

  const toggle = (dir: string) => {
    setExpanded((s) => {
      const next = new Set(s);
      if (next.has(dir)) next.delete(dir);
      else {
        next.add(dir);
        if (!children[dir]) load(dir);
      }
      return next;
    });
  };

  const renderDir = (dir: string, depth: number): JSX.Element[] => {
    const rows = children[dir];
    if (!rows) return [];
    return rows.flatMap((e) => {
      const path = dir ? `${dir}/${e.name}` : e.name;
      const isChanged = changed.has(path);
      const row = (
        <button
          key={path}
          type="button"
          className={`ft-row${e.isDir ? " ft-dir" : ""}${isChanged ? " ft-changed" : ""}${selected === path ? " ft-selected" : ""}`}
          style={{ paddingLeft: `${8 + depth * 14}px` }}
          onClick={() => (e.isDir ? toggle(path) : setSelected(path))}
          onDoubleClick={() => {
            if (!e.isDir) emitOpenFile({ taskId, workspaceId, path });
          }}
        >
          <span className="ft-glyph">
            {e.isDir ? (expanded.has(path) ? "▾" : "▸") : "·"}
          </span>
          {e.name}
        </button>
      );
      return e.isDir && expanded.has(path)
        ? [row, ...renderDir(path, depth + 1)]
        : [row];
    });
  };

  return (
    <div className="file-tree" data-active={active || undefined}>
      {error && <div className="ft-error">{error}</div>}
      {children[""] && children[""].length === 0 && !error && (
        <div className="ft-empty">empty worktree</div>
      )}
      {renderDir("", 0)}
    </div>
  );
}
