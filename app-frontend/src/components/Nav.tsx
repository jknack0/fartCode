// Left nav redesign (design_handoff_left_nav): a 56px icon rail plus a
// 244px project flyout, replacing the old projects/tasks tree sidebar.
// The flyout shows in-flight work first (in_progress = running, review =
// needs you), then a short Recent list of everything else — ad-hoc tasks
// have no board card, so without Recent they'd be unreachable outside ⌘K.
// ⌘B / ⌘\ toggles the flyout (the "toggle-sidebar" command); the rail is
// always on so project state is never hidden by collapsing.
import { useEffect, useState } from "react";
import { useUi } from "../store/ui";
import { useSidebar } from "../store/sidebar";
import { useScripts } from "../store/scripts";
import { useSteps } from "../store/steps";
import { TaskDto } from "../lib/tauri";
import { hint } from "../lib/useCommands";

export default function Nav() {
  return (
    <div className="shell-nav">
      <LeftRail />
      <ProjectFlyout />
    </div>
  );
}

/** Worst-of the project's runs: running beats needs-you. */
function agentState(tasks: TaskDto[]): "running" | "needs-you" | null {
  let needsYou = false;
  for (const t of tasks) {
    if (t.archivedAt) continue;
    if (t.status === "in_progress") return "running";
    if (t.status === "review") needsYou = true;
  }
  return needsYou ? "needs-you" : null;
}

function LeftRail() {
  const projects = useSidebar((s) => s.projects);
  const tasksByProject = useSidebar((s) => s.tasksByProject);
  const selectedProjectId = useSidebar((s) => s.selectedProjectId);
  const selectProject = useSidebar((s) => s.selectProject);
  const setCreateProjectOpen = useUi((s) => s.setCreateProjectOpen);
  const settingsOpen = useUi((s) => s.settingsOpen);
  const setSettingsOpen = useUi((s) => s.setSettingsOpen);
  const setDeleteProjectTarget = useUi((s) => s.setDeleteProjectTarget);
  const setSidebarVisible = useUi((s) => s.setSidebarVisible);
  // Re-render hint text when keybindings change.
  useUi((s) => s.bindingsVersion);

  return (
    <nav className="rail" data-tauri-drag-region="deep">
      {/* fC mark (design_handoff_v2 frame 6d): green tile, dark mono glyphs.
          Inline SVG so the app's loaded JetBrains Mono Variable renders the
          letterforms; .rail-mark keeps the rail's sizing/margin. */}
      <svg
        className="rail-mark"
        width={18}
        height={18}
        viewBox="0 0 64 64"
        role="img"
        aria-label="fartCode"
      >
        <rect width="64" height="64" rx="16" fill="#45d68a" />
        <text
          x="9"
          y="45"
          fontSize="34"
          fontWeight="600"
          fill="#0d0d10"
          fontFamily="var(--font-mono)"
        >
          f
        </text>
        <text
          x="31"
          y="45"
          fontSize="34"
          fontWeight="600"
          fill="#0d0d10"
          fontFamily="var(--font-mono)"
        >
          C
        </text>
      </svg>

      <div className="rail-scroll">
        {projects.map((p) => {
          const agent = agentState(tasksByProject[p.id] ?? []);
          return (
            <button
              key={p.id}
              type="button"
              className={`rail-tile${p.id === selectedProjectId ? " active" : ""}`}
              title={`${p.name} — right-click to delete`}
              aria-label={p.name}
              onClick={() => {
                selectProject(p.id);
                // Clicking a tile is also the mouse path back from a collapsed
                // flyout — ⌘\ is otherwise the only way to reopen it.
                setSidebarVisible(true);
              }}
              onContextMenu={(e) => {
                e.preventDefault();
                setDeleteProjectTarget(p.id);
              }}
            >
              {p.name[0] ?? "?"}
              {agent && (
                <span className="tile-dot">
                  <span className={`status-dot status-${agent === "running" ? "in_progress" : "needs-you"}`} />
                </span>
              )}
            </button>
          );
        })}
      </div>

      <button
        type="button"
        className={`rail-tile glyph${projects.length === 0 ? " dashed" : ""}`}
        title={`Add project (${hint("new-project") || "⌘⇧N"})`}
        aria-label="Add project"
        onClick={() => setCreateProjectOpen(true)}
      >
        +
      </button>

      <div className="rail-spacer" />
      <button
        type="button"
        className={`rail-tile mono${settingsOpen ? " active" : ""}`}
        title={`Settings (${hint("open-settings") || "⌘,"})`}
        aria-label="Settings"
        onClick={() => setSettingsOpen(true)}
      >
        ⌘
      </button>
    </nav>
  );
}

function ProjectFlyout() {
  const projects = useSidebar((s) => s.projects);
  const selectedProjectId = useSidebar((s) => s.selectedProjectId);
  const tasksByProject = useSidebar((s) => s.tasksByProject);
  const selectTask = useSidebar((s) => s.selectTask);
  const pendingTitle = useSidebar((s) => s.pendingTitle);
  const setCreateTaskTarget = useUi((s) => s.setCreateTaskTarget);
  const visible = useUi((s) => s.sidebarVisible);
  const toggleVisible = useUi((s) => s.toggleSidebarVisible);
  useUi((s) => s.bindingsVersion);
  const agentByTask = useScripts((s) => s.agentByTask);
  const launchingByIssue = useSteps((s) => s.launchingByIssue);

  // Elapsed times are derived from statusChangedAt, never stored — refresh
  // on a slow tick (the display is minute-coarse).
  const [, setTick] = useState(0);
  useEffect(() => {
    const t = setInterval(() => setTick((n) => n + 1), 30_000);
    return () => clearInterval(t);
  }, []);

  // The dot + Running group key on the LIVE agent terminal, never frozen
  // task.status (TaskHeader's doctrine — TaskStatus has no production
  // writer, so status is born `in_progress` and never moves). Hydrate each
  // task's terminal snapshot once so a backend-launched agent (the create
  // path) reads as running before its task view ever mounts.
  useEffect(() => {
    if (!selectedProjectId) return;
    for (const t of tasksByProject[selectedProjectId] ?? []) {
      if (useScripts.getState().agentByTask[t.id]) continue;
      void useScripts.getState().hydrate(t.id).catch(() => {});
    }
  }, [selectedProjectId, tasksByProject]);

  const project = projects.find((p) => p.id === selectedProjectId);
  if (!project || !visible) return null;

  // Pasted-prompt tasks stay hidden until their LLM title lands
  // (task:renamed) or the store's 10s cap gives up and reveals them.
  const projectTasks = (tasksByProject[project.id] ?? []).filter((t) => !pendingTitle[t.id]);
  const active = projectTasks.filter((t) => !t.archivedAt);
  // In-flight work is the LIVE agent terminal, never `task.status` (no
  // production writer — status is frozen at `in_progress` from birth; the
  // same audit drives TaskHeader's dot). `review` stays status-keyed for the
  // future needs-you state (no writer today, so it cannot fire).
  const needsYou = active.filter((t) => t.status === "review");
  // A dispatch in flight counts as running before its terminal is open —
  // the task is already provisioned and the prompt is on its way.
  const launchingTaskIds = new Set(
    Object.values(launchingByIssue)
      .map((l) => l.taskId)
      .filter((id): id is string => Boolean(id)),
  );
  const running = active.filter(
    (t) => Boolean(agentByTask[t.id]?.running) || launchingTaskIds.has(t.id),
  );
  // Everything at rest — stopped agents and never-run ad-hoc tasks — has no
  // board card, so list the most recent ones for a path back to them.
  const recent = active
    .filter(
      (t) =>
        t.status !== "review" &&
        !agentByTask[t.id]?.running &&
        !launchingTaskIds.has(t.id),
    )
    .sort(
      (a, b) =>
        Date.parse(b.lastInteractedAt ?? b.statusChangedAt ?? "") -
        Date.parse(a.lastInteractedAt ?? a.statusChangedAt ?? ""),
    )
    .slice(0, 5);
  // The design's groups, worst-first. "Recent" is an addition: ad-hoc tasks
  // have no board card, so without it they'd be unreachable outside ⌘K.
  const groups: { label: string; items: TaskDto[] }[] = [
    { label: "Needs you", items: needsYou },
    { label: "Running", items: running },
    { label: "Recent", items: recent },
  ].filter((g) => g.items.length > 0);

  return (
    <aside className="flyout">
      <div className="flyout-head" data-tauri-drag-region="deep">
        <span className="flyout-name">{project.name}</span>
        <button
          type="button"
          className="flyout-collapse"
          title={`Collapse (${hint("toggle-sidebar") || "⌘\\"})`}
          aria-label="Collapse project flyout"
          onClick={toggleVisible}
        >
          ‹
        </button>
      </div>
      <div className="flyout-path" title={project.path}>
        {shortPath(project.path)} · {shortRef(project.baseRef)}
      </div>

      {groups.length === 0 && <div className="flyout-empty">nothing running</div>}
      {groups.map((g) => (
        <div key={g.label} className="flyout-group">
          <div className="flyout-group-label">{g.label}</div>
          <div className="flyout-rows">
            {g.items.map((t) => {
              const runningNow = Boolean(agentByTask[t.id]?.running);
              const launchingNow = !runningNow && launchingTaskIds.has(t.id);
              const dotClass =
                t.status === "review"
                  ? "status-dot status-needs-you"
                  : runningNow
                    ? "status-dot status-in_progress"
                    : launchingNow
                      ? "status-dot status-launching"
                      : "status-dot tv-dot-idle";
              return (
                <button
                  key={t.id}
                  type="button"
                  className="flyout-row"
                  onClick={() => selectTask(t.id)}
                >
                  <span className={dotClass} />
                  <div style={{ minWidth: 0 }}>
                    <div className="flyout-row-title">{t.name}</div>
                    <div className="flyout-row-meta">
                      {t.status === "review" ? "needs you" : runningNow ? "running" : "stopped"}
                      {t.statusChangedAt ? ` · ${ago(t.statusChangedAt)}` : ""}
                    </div>
                  </div>
                </button>
              );
            })}
          </div>
        </div>
      ))}

      <button
        type="button"
        className="flyout-new-task"
        title={`New task (${hint("add-task") || "⌘N"})`}
        onClick={() => setCreateTaskTarget(project.id)}
      >
        + New task
      </button>
    </aside>
  );
}

/** Relative time, coarse: now / Nm / Nh / Nd / Nw. */
function ago(iso: string): string {
  const s = Math.max(0, (Date.now() - Date.parse(iso)) / 1000);
  if (s < 90) return "now";
  const m = s / 60;
  if (m < 60) return `${Math.round(m)}m`;
  const h = m / 60;
  if (h < 24) return `${Math.round(h)}h`;
  const d = h / 24;
  if (d < 7) return `${Math.round(d)}d`;
  return `${Math.round(d / 7)}w`;
}

/** …/Dev/ade — the last two path segments, as the design's meta line shows. */
function shortPath(path: string): string {
  const parts = path.replace(/\/+$/, "").split("/").filter(Boolean);
  return parts.length > 2 ? `…/${parts.slice(-2).join("/")}` : `/${parts.join("/")}`;
}

function shortRef(ref: string | null): string {
  return (ref ?? "main").replace(/^refs\/heads\//, "");
}
