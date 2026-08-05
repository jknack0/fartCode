// Command palette (E1-09): ⌘K finds anything — commands + FTS search results
// over projects/tasks. Selecting a result runs its action.
import { useEffect, useRef, useState } from "react";
import { hint } from "../lib/useCommands";
import {
  SearchResultDto,
  getResourceMonitorEnabled,
  search as apiSearch,
  setResourceMonitorEnabled,
} from "../lib/tauri";
import { useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";

interface PaletteCommand {
  id: string;
  title: string;
  hint?: string;
  run: () => void;
}

export default function CommandPalette() {
  const open = useUi((s) => s.paletteOpen);
  useUi((s) => s.bindingsVersion);
  const setOpen = useUi((s) => s.setPaletteOpen);
  const setCreateProjectOpen = useUi((s) => s.setCreateProjectOpen);
  const setResourceOpen = useUi((s) => s.setResourceOpen);
  const selectProject = useSidebar((s) => s.selectProject);
  const selectTask = useSidebar((s) => s.selectTask);

  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResultDto[]>([]);
  const [selected, setSelected] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (open) {
      setQuery("");
      setResults([]);
      setSelected(0);
      setTimeout(() => inputRef.current?.focus(), 0);
    }
  }, [open]);

  // Debounced FTS search.
  useEffect(() => {
    if (!open) return;
    const q = query.trim();
    if (!q) {
      setResults([]);
      return;
    }
    const t = setTimeout(() => {
      apiSearch(q, 8).then(setResults).catch(() => setResults([]));
    }, 150);
    return () => clearTimeout(t);
  }, [query, open]);

  const commands: PaletteCommand[] = [
    {
      id: "new-project",
      title: "New project…",
      hint: hint("new-project") || "⌘⇧N",
      run: () => {
        setOpen(false);
        setCreateProjectOpen(true);
      },
    },
    {
      id: "toggle-resource",
      title: "Toggle resource monitor",
      hint: "CPU / memory panel",
      run: () => {
        // Enable -> open the panel; disable -> close it. Opening on disable
        // would pop a panel that immediately renders nothing.
        getResourceMonitorEnabled()
          .then((enabled) => {
            const next = !enabled;
            return setResourceMonitorEnabled(next).then(() => setResourceOpen(next));
          })
          .catch(() => {});
        setOpen(false);
      },
    },
  ];

  const combined = [
    ...commands.map((c, i) => ({ kind: "command" as const, index: i, title: c.title, hint: c.hint })),
    ...results.map((r) => ({
      kind: "result" as const,
      title: r.title,
      hint: r.itemType,
      item: r,
    })),
  ];

  const run = (i: number) => {
    const entry = combined[i];
    if (!entry) return;
    if (entry.kind === "command") {
      commands[entry.index].run();
      return;
    }
    const item = entry.item!;
    setOpen(false);
    if (item.itemType === "project" && item.itemId) {
      selectProject(item.itemId);
    } else if (item.itemType === "task" && item.itemId) {
      if (item.projectId) selectProject(item.projectId);
      selectTask(item.itemId);
    }
  };

  if (!open) return null;

  return (
    <div className="modal-backdrop" onClick={() => setOpen(false)}>
      <div className="palette" onClick={(e) => e.stopPropagation()}>
        <input
          ref={inputRef}
          value={query}
          placeholder="Search projects, tasks, commands…"
          onChange={(e) => {
            setQuery(e.target.value);
            setSelected(0);
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") {
              e.preventDefault();
              setSelected((s) => Math.min(s + 1, combined.length - 1));
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              setSelected((s) => Math.max(s - 1, 0));
            } else if (e.key === "Enter") {
              run(selected);
            }
          }}
        />
        {query.trim() === "" ? (
          <p className="palette-hint">Type to search the index; commands are listed first.</p>
        ) : (
          <ul className="palette-results">
            {combined.map((c, i) => (
              <li
                key={c.kind === "command" ? `cmd-${c.index}` : `res-${c.item!.itemId}`}
                className={i === selected ? "selected" : ""}
                onMouseEnter={() => setSelected(i)}
                onClick={() => run(i)}
              >
                <span className="palette-title">{c.title}</span>
                <span className="palette-hint">{c.hint}</span>
              </li>
            ))}
            {combined.length === 0 && <li className="empty">No matches</li>}
          </ul>
        )}
      </div>
    </div>
  );
}
