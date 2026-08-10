// Command palette (E1-09): ⌘K finds anything — every registered command
// (live from the keybinding registry, E14-01, so remaps show up here) plus
// FTS results over projects/tasks. Empty query = the full command list in
// registration order (global → project → task); typing fuzzy-filters it.
import { useEffect, useRef, useState } from "react";
import { bindings } from "../lib/useCommands";
import {
  FeatureRowDto,
  SearchResultDto,
  dossierFeatureRows,
  getResourceMonitorEnabled,
  restoreTask,
  search as apiSearch,
  setResourceMonitorEnabled,
} from "../lib/tauri";
import { CommandId } from "../lib/registry";
import { issueRefParts } from "./board/runState";
import { useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";

/** Subsequence match with ranking: prefix > substring > scattered letters.
 * Returns null when the query doesn't match at all. */
function fuzzyScore(query: string, text: string): number | null {
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  if (t.startsWith(q)) return 100;
  const idx = t.indexOf(q);
  if (idx >= 0) return 80 - idx;
  let qi = 0;
  let streaks = 0;
  let prev = -2;
  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t[ti] === q[qi]) {
      if (prev === ti - 1) streaks++;
      prev = ti;
      qi++;
    }
  }
  return qi === q.length ? 40 + streaks : null;
}

interface PaletteEntry {
  key: string;
  title: string;
  /** Chord shown as a key cap; plain text (item type) renders unbordered. */
  hint: string;
  hintIsChord: boolean;
  /** §8h's row style: 12.5 sans title, mono 10.5 `--meta` right. */
  feature?: boolean;
  run: () => void;
}

/** `fartcode_core::dossier_index::ITEM_TYPE` — one row per dossier section
 * (ADR-0038 item 4). #75 removed the backend filter that used to hold these
 * back, so they arrive like any other hit. */
const FEATURE_ITEM_TYPE = "feature";

/** §8h renders a hit as `<Column> — <feature title>`. The COLUMN comes from
 * the indexed section heading (`<Column> — <date>`) — the feature's own
 * title replaces the date, since the date is already the timeline's job. An
 * agent-written heading with no separator is used whole rather than
 * guessed at. */
function columnOfHeading(heading: string): string {
  const at = heading.indexOf(" — ");
  return at > 0 ? heading.slice(0, at) : heading;
}

/** Commands the palette never lists: itself, and modal-scope keys (Esc). */
const HIDDEN: ReadonlySet<CommandId> = new Set(["open-command-palette", "close-modal"]);

export default function CommandPalette() {
  const open = useUi((s) => s.paletteOpen);
  useUi((s) => s.bindingsVersion);
  const setOpen = useUi((s) => s.setPaletteOpen);
  const setResourceOpen = useUi((s) => s.setResourceOpen);
  const selectProject = useSidebar((s) => s.selectProject);
  const selectTask = useSidebar((s) => s.selectTask);
  const selectedProjectId = useSidebar((s) => s.selectedProjectId);
  const selectedTaskId = useSidebar((s) => s.selectedTaskId);

  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResultDto[]>([]);
  /** Feature hits' cards, by item id (§8h) — the index carries the section
   * heading, not the feature's title, its ref, or whether it landed. */
  const [features, setFeatures] = useState<Record<string, FeatureRowDto>>({});
  const [selected, setSelected] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (open) {
      setQuery("");
      setResults([]);
      setFeatures({});
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

  // A feature hit needs its card before it can render §8h's row or open
  // anything. Until it resolves the row still shows (as its heading) —
  // a hit that flickers away would be worse than one that fills in.
  useEffect(() => {
    const ids = results
      .filter((r) => r.itemType === FEATURE_ITEM_TYPE)
      .map((r) => r.itemId);
    if (ids.length === 0) {
      setFeatures({});
      return;
    }
    let cancelled = false;
    dossierFeatureRows(ids)
      .then((rows) => {
        if (cancelled) return;
        setFeatures(Object.fromEntries(rows.map((row) => [row.itemId, row])));
      })
      .catch(() => !cancelled && setFeatures({}));
    return () => {
      cancelled = true;
    };
  }, [results]);

  if (!open) return null;

  // Every registry command valid in the current view context (the palette
  // itself is the open modal, so modal state is ignored here).
  const available = bindings().filter((b) => {
    if (HIDDEN.has(b.id)) return false;
    if (b.scope === "modal") return false;
    if (b.scope === "project-view") return selectedProjectId !== null;
    if (b.scope === "task-view") return selectedTaskId !== null;
    return true; // global / app-view / editor
  });

  const runCommand = (run: () => void) => {
    setOpen(false);
    run();
  };

  let commandEntries: PaletteEntry[] = available.map((b) => ({
    key: b.id,
    title: b.label,
    hint: b.hint,
    hintIsChord: b.hint !== "",
    run: () => runCommand(b.run),
  }));

  // Settings-backed special case the registry can't express: enabling the
  // resource monitor flips a persisted setting, not just a panel.
  commandEntries.push({
    key: "toggle-resource-monitor",
    title: "Toggle resource monitor (enable/disable)",
    hint: "",
    hintIsChord: false,
    run: () => {
      getResourceMonitorEnabled()
        .then((enabled) => {
          const next = !enabled;
          return setResourceMonitorEnabled(next).then(() => setResourceOpen(next));
        })
        .catch(() => {});
      setOpen(false);
    },
  });

  const q = query.trim();
  if (q) {
    commandEntries = commandEntries
      .map((e) => ({ e, score: fuzzyScore(q, e.title) }))
      .filter((x): x is { e: PaletteEntry; score: number } => x.score !== null)
      .sort((a, b) => b.score - a.score)
      .map((x) => x.e);
  }

  const entries: PaletteEntry[] = [
    ...commandEntries,
    ...results.map((r) => {
      // §8h feature hit: `<Column> — <feature title>` with a mono
      // `feature · #id[ · landed]` right meta, and ↵ opens the CARD
      // DETAIL — one destination whether the feature is live or landed,
      // since issue rows outlive their tasks.
      const feature =
        r.itemType === FEATURE_ITEM_TYPE ? features[r.itemId] : undefined;
      if (feature) {
        const { ref, title } = issueRefParts(feature.title, feature.externalRef);
        return {
          key: `res-${r.itemId}`,
          title: `${columnOfHeading(r.title)} — ${title}`,
          hint: `${FEATURE_ITEM_TYPE}${ref ? ` · ${ref}` : ""}${
            feature.landed ? " · landed" : ""
          }`,
          hintIsChord: false,
          feature: true,
          run: () => {
            setOpen(false);
            // The detail lives in the project view's one right-panel slot
            // (the same route `runOpenCardDetail` takes).
            const ui = useUi.getState();
            ui.setBoardDetailIssueId(feature.issueId);
            ui.setChangesOpen(true);
            if (r.projectId) selectProject(r.projectId);
          },
        };
      }
      // 7a: archived tasks stay findable here — opening one restores it
      // ("restore via ⌘K"); the task:restored event refreshes the stores.
      const archived =
        r.itemType === "task" &&
        Object.values(useSidebar.getState().tasksByProject)
          .flat()
          .some((t) => t.id === r.itemId && t.archivedAt);
      return {
        key: `res-${r.itemId}`,
        title: r.title,
        hint: archived ? `${r.itemType} · archived — ↵ restores` : r.itemType,
        hintIsChord: false,
        run: () => {
          setOpen(false);
          if (r.itemType === "project" && r.itemId) {
            selectProject(r.itemId);
          } else if (r.itemType === "task" && r.itemId) {
            if (archived && r.itemId) void restoreTask(r.itemId).catch(() => {});
            if (r.projectId) selectProject(r.projectId);
            selectTask(r.itemId);
          }
        },
      };
    }),
  ];

  const run = (i: number) => entries[i]?.run();

  return (
    <div className="modal-backdrop" onClick={() => setOpen(false)}>
      <div className="palette" onClick={(e) => e.stopPropagation()}>
        <input
          ref={inputRef}
          value={query}
          placeholder="Type a command or search projects and tasks…"
          aria-label="Command palette"
          onChange={(e) => {
            setQuery(e.target.value);
            setSelected(0);
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") {
              e.preventDefault();
              setSelected((s) => Math.min(s + 1, entries.length - 1));
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              setSelected((s) => Math.max(s - 1, 0));
            } else if (e.key === "Enter") {
              run(selected);
            }
          }}
        />
        <ul className="palette-results">
          {entries.map((entry, i) => (
            <li
              key={entry.key}
              className={`${entry.feature ? "palette-feature" : ""}${
                i === selected ? " selected" : ""
              }`.trim()}
              onMouseEnter={() => setSelected(i)}
              onClick={() => run(i)}
            >
              <span className="palette-title">{entry.title}</span>
              {entry.hint &&
                (entry.hintIsChord ? (
                  <span className="kbd-hint">{entry.hint}</span>
                ) : (
                  <span className="palette-hint">{entry.hint}</span>
                ))}
            </li>
          ))}
          {entries.length === 0 && <li className="empty">No matches</li>}
        </ul>
        <p className="palette-hint">
          ↑↓ move · ↵ run · esc close — every shortcut, live from your keymap
        </p>
      </div>
    </div>
  );
}
