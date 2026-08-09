import { c, mono, RAIL_W } from './theme';
import { StatusDot, RunState } from './StatusDot';

export type Project = {
  id: string;
  name: string;
  /** worst-of the project's runs: running | blocked | failed | undefined */
  agent?: Extract<RunState, 'running' | 'blocked' | 'failed'>;
};

type Props = {
  projects: Project[];               // 2-5 in practice
  activeId: string | null;
  sessionsLive?: boolean;            // a bare TUI session is open
  view: 'board' | 'session' | 'settings';
  onSelect(id: string): void;
  onNewTask(): void;
  onSessions(): void;
  onSettings(): void;
};

export function LeftRail({ projects, activeId, sessionsLive, view, onSelect, onNewTask, onSessions, onSettings }: Props) {
  return (
    <nav style={{
      width: RAIL_W, flex: 'none', background: c.rail, borderRight: `1px solid ${c.hairline}`,
      display: 'flex', flexDirection: 'column', alignItems: 'center', padding: '14px 0', gap: 14,
    }}>
      <div style={{ width: 18, height: 18, borderRadius: 5, background: c.accent, marginBottom: 6 }} />

      {projects.map((p, i) => (
        <Tile
          key={p.id}
          label={p.name.slice(0, 1)}
          active={view === 'board' && p.id === activeId}
          dot={p.agent}
          title={`${p.name}  ⌘${i + 1}`}
          onClick={() => onSelect(p.id)}
        />
      ))}

      <Tile label="+" glyph onClick={onNewTask} title="New task  ⌘N" />
      <Tile label=">_" mono active={view === 'session'} dot={sessionsLive ? 'running' : undefined} onClick={onSessions} title="Sessions  ⌘⇧↵" />

      <div style={{ flex: 1 }} />
      <Tile label="⌘" mono active={view === 'settings'} onClick={onSettings} title="Settings  ⌘," />
    </nav>
  );
}

function Tile({ label, active, dot, glyph, mono: isMono, title, onClick }: {
  label: string; active?: boolean; dot?: RunState; glyph?: boolean; mono?: boolean; title?: string; onClick?(): void;
}) {
  return (
    <button
      type="button" title={title} onClick={onClick}
      style={{
        position: 'relative', width: 34, height: 34, borderRadius: 10, cursor: 'pointer',
        background: active ? '#1e1e22' : 'transparent',
        border: active ? '1px solid rgba(255,255,255,.14)' : glyph ? '1px solid transparent' : `1px solid ${c.hairline}`,
        color: active ? c.text : glyph ? '#55555c' : '#7c7c83',
        fontFamily: isMono ? mono : undefined,
        fontSize: glyph ? 18 : 13, fontWeight: isMono || glyph ? 400 : 600,
        display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 0,
      }}
    >
      {label}
      {active && (
        <span style={{ position: 'absolute', left: -14, top: 9, width: 2, height: 16, borderRadius: 2, background: c.accent }} />
      )}
      {dot && (
        <span style={{ position: 'absolute', right: -3, bottom: -3, borderRadius: '50%', border: `2px solid ${c.rail}`, display: 'flex' }}>
          <StatusDot state={dot} size={8} />
        </span>
      )}
    </button>
  );
}
