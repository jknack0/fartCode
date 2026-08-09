import { c } from './theme';

export type RunState = 'running' | 'blocked' | 'failed' | 'conflict' | 'stopped' | 'queued' | 'idle';

/** The entire status vocabulary. Filled amber = working, hollow amber = needs you,
 *  red = ended badly, grey = you stopped it, dashed = waiting for a slot. */
export function StatusDot({ state, size = 7 }: { state: RunState; size?: number }) {
  const base: React.CSSProperties = {
    width: size, height: size, borderRadius: '50%', flex: 'none', boxSizing: 'border-box',
  };
  switch (state) {
    case 'running':
      return <span style={{ ...base, background: c.working, animation: 'fc-pulse 1.8s ease-in-out infinite' }} />;
    case 'blocked':
      return <span style={{ ...base, width: size + 1, height: size + 1, border: `1.5px solid ${c.working}` }} />;
    case 'failed':
    case 'conflict':
      return <span style={{ ...base, background: c.bad }} />;
    case 'stopped':
      return <span style={{ ...base, background: '#46464d' }} />;
    case 'queued':
      return <span style={{ ...base, border: '1px dashed #6e6e75' }} />;
    default:
      return <span style={{ ...base, background: 'transparent' }} />;
  }
}

/** Keyframes live once, at the app root. */
export const keyframes = `
@keyframes fc-pulse { 0%,100% { opacity: 1 } 50% { opacity: .35 } }
@keyframes fc-caret { 0%,49% { opacity: 1 } 50%,100% { opacity: 0 } }
`;
