import { useState } from 'react';
import { c, mono } from './theme';

/** One field. Typing '>' first turns a task into a bare session — only the footer changes. */
export function Composer({ project, branch, onSubmit, onClose }: {
  project: string; branch: string;
  onSubmit(input: { text: string; kind: 'task' | 'session'; startNow: boolean }): void;
  onClose(): void;
}) {
  const [text, setText] = useState('');
  const session = text.startsWith('>');
  const body = session ? text.slice(1).trimStart() : text;

  return (
    <div style={{
      width: 440, borderRadius: 10, background: c.overlay, border: `1px solid ${c.hairlineStrong}`,
      boxShadow: '0 24px 60px rgba(0,0,0,.65)', overflow: 'hidden',
    }}>
      <div style={{ padding: '18px 18px 16px', display: 'flex', gap: 8, alignItems: 'flex-start' }}>
        <span style={{ fontFamily: mono, fontSize: 14, lineHeight: 1.5, color: session ? c.accent : c.meta }}>
          {session ? '>' : '›'}
        </span>
        <input
          autoFocus value={text}
          onChange={e => setText(e.target.value)}
          onKeyDown={e => {
            if (e.key === 'Escape') onClose();
            if (e.key === 'Enter') onSubmit({ text: body, kind: session ? 'session' : 'task', startNow: e.metaKey });
          }}
          placeholder={session ? 'ask anything' : 'what needs doing'}
          style={{
            flex: 1, background: 'none', border: 0, outline: 'none', color: c.text,
            fontFamily: session ? mono : undefined, fontSize: session ? 13.5 : 14, lineHeight: 1.5,
          }}
        />
      </div>
      <div style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        padding: '11px 18px', borderTop: `1px solid rgba(255,255,255,.07)`,
        fontFamily: mono, fontSize: 11, color: '#66666d',
      }}>
        <span>{session ? 'session · ' : ''}{project} · {branch}</span>
        <span>{session ? <><b style={{ color: '#a4a4ab', fontWeight: 400 }}>↵</b> open</> : <><b style={{ color: '#a4a4ab', fontWeight: 400 }}>↵</b> queue · <b style={{ color: '#a4a4ab', fontWeight: 400 }}>⌘↵</b> start now</>}</span>
      </div>
    </div>
  );
}
