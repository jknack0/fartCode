import { useState } from 'react';
import { c, mono, t } from './theme';
import { StatusDot, RunState } from './StatusDot';

export type Task = {
  id: number;
  title: string;
  state: RunState;
  status?: string;          // 'writing tests', 'tests exit 1', 'queued · 2nd of 3'
  elapsed?: string;
  gh?: { kind: 'issue' | 'pr'; number: number; url: string };
  hint?: string;            // '↵ read · ⌘r retry' — only for states that need an out
};

/** No box. Hover paints a background, focus adds the accent rail, that's it. */
export function TaskCard({ task, focused, onOpen }: { task: Task; focused?: boolean; onOpen(): void }) {
  const [hover, setHover] = useState(false);
  const dim = task.state === 'queued';
  const bad = task.state === 'failed' || task.state === 'conflict';

  return (
    <div
      role="button" tabIndex={0} onClick={onOpen}
      onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)}
      style={{
        display: 'flex', gap: 9, padding: '8px 12px', cursor: 'pointer', outline: 'none',
        borderLeft: `2px solid ${focused ? c.accent : 'transparent'}`,
        borderRadius: '0 6px 6px 0',
        background: focused ? c.focusBg : hover ? c.hover : 'transparent',
        opacity: dim ? .55 : 1,
      }}
    >
      {task.state !== 'idle' && <span style={{ marginTop: 6 }}><StatusDot state={task.state} /></span>}
      <div style={{ minWidth: 0 }}>
        <div style={{ ...t.meta, marginBottom: 5, color: bad ? '#c98d8d' : c.meta }}>
          #{task.id}
          {task.gh && <> · <a href={task.gh.url} style={{ color: c.link, textDecoration: 'none' }}>{task.gh.kind === 'pr' ? 'pr' : 'gh'} {task.gh.number}</a></>}
          {task.status && ` · ${task.status}`}
          {task.elapsed && ` · ${task.elapsed}`}
        </div>
        <div style={{ ...t.card, color: hover || focused ? '#fff' : c.textCard }}>{task.title}</div>
        {task.hint && <div style={{ ...t.meta, marginTop: 6, color: '#66666d' }}>{task.hint}</div>}
      </div>
    </div>
  );
}

export function Column({ name, count, dimmed, children }: {
  name: string; count: number; dimmed?: boolean; children: React.ReactNode;
}) {
  return (
    <section style={{ flex: 1, minWidth: 0, padding: '22px 16px 0 22px', display: 'flex', flexDirection: 'column' }}>
      <header style={{
        display: 'flex', alignItems: 'baseline', justifyContent: 'space-between',
        paddingBottom: 12, marginBottom: 16, borderBottom: '1px solid rgba(255,255,255,.07)',
      }}>
        <span style={{ fontSize: 13, color: dimmed ? '#6e6e75' : '#c9c9cf' }}>{name}</span>
        <span style={{ fontFamily: mono, fontSize: 11, color: dimmed ? c.metaDim : c.meta }}>{count}</span>
      </header>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>{children}</div>
    </section>
  );
}
