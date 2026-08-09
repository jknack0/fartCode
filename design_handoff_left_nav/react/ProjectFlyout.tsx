import { c, mono, t, FLYOUT_W } from './theme';
import { StatusDot, RunState } from './StatusDot';

export type LiveTask = { id: number; title: string; state: RunState; status?: string; elapsed?: string };
export type LiveSession = { id: string; prompt: string; elapsed: string };

type Props = {
  name: string;
  path: string;          // already shortened: …/Dev/ade
  branch: string;        // main
  tasks: LiveTask[];     // ONLY in-flight work. the board owns everything else
  sessions?: LiveSession[];
  onCollapse(): void;
  onOpenTask(id: number): void;
};

export function ProjectFlyout({ name, path, branch, tasks, sessions = [], onCollapse, onOpenTask }: Props) {
  const blocked = tasks.filter(t => t.state === 'blocked');
  const running = tasks.filter(t => t.state !== 'blocked');

  return (
    <aside style={{
      width: FLYOUT_W, flex: 'none', background: c.flyout, borderRight: `1px solid ${c.hairline}`,
      display: 'flex', flexDirection: 'column', padding: '18px 14px',
    }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <span style={{ fontSize: 14, fontWeight: 600, letterSpacing: '-0.01em' }}>{name}</span>
        <button type="button" onClick={onCollapse} title="Collapse  ⌘\\"
          style={{ background: 'none', border: 0, color: '#55555c', fontSize: 14, cursor: 'pointer', padding: 0 }}>‹</button>
      </div>
      <div style={{ fontFamily: mono, fontSize: 11, color: '#66666d', marginBottom: 28 }}>{path} · {branch}</div>

      {blocked.length > 0 && <Group label="Needs you" items={blocked} onOpen={onOpenTask} />}
      {running.length > 0 && <Group label="Running" items={running} onOpen={onOpenTask} />}
      {blocked.length + running.length === 0 && (
        <div style={{ fontFamily: mono, fontSize: 11, color: '#4e4e55' }}>nothing running</div>
      )}

      {sessions.length > 0 && (
        <>
          <div style={{ ...t.label, margin: '28px 0 14px' }}>Sessions</div>
          {sessions.map(s => (
            <div key={s.id} style={{ display: 'flex', gap: 9, marginBottom: 16 }}>
              <span style={{ marginTop: 5 }}><StatusDot state="running" /></span>
              <div style={{ minWidth: 0 }}>
                <div style={{ fontFamily: mono, fontSize: 12, lineHeight: 1.4, color: c.textCard }}>&gt; {s.prompt}</div>
                <div style={{ ...t.meta, marginTop: 4 }}>{s.elapsed}</div>
              </div>
            </div>
          ))}
        </>
      )}
      <div style={{ flex: 1 }} />
    </aside>
  );
}

function Group({ label, items, onOpen }: { label: string; items: LiveTask[]; onOpen(id: number): void }) {
  return (
    <>
      <div style={{ ...t.label, marginBottom: 14 }}>{label}</div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 16, marginBottom: 26 }}>
        {items.map(task => (
          <div key={task.id} role="button" onClick={() => onOpen(task.id)}
            style={{ display: 'flex', gap: 9, cursor: 'pointer' }}>
            <span style={{ marginTop: 5 }}><StatusDot state={task.state} /></span>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 12.5, lineHeight: 1.4, color: c.textCard }}>{task.title}</div>
              <div style={{ ...t.meta, marginTop: 4 }}>
                #{task.id}{task.status ? ` · ${task.status}` : ''}{task.elapsed ? ` · ${task.elapsed}` : ''}
              </div>
            </div>
          </div>
        ))}
      </div>
    </>
  );
}
