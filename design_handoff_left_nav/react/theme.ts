// fartCode design tokens. Inline-style friendly: no CSS file needed.
export const c = {
  window: '#101012',
  rail: '#0b0b0d',
  flyout: '#0e0e10',
  overlay: '#17171b',
  hairline: 'rgba(255,255,255,.06)',
  hairlineStrong: 'rgba(255,255,255,.12)',
  hover: 'rgba(255,255,255,.035)',
  focusBg: 'rgba(255,255,255,.05)',
  text: '#e8e8ea',
  textCard: '#dcdce1',
  textMuted: '#9a9aa1',
  meta: '#5f5f66',
  metaDim: '#5f5f66',   // floor. nothing legible goes dimmer
  accent: 'oklch(.78 .15 155)',   // selection + additions. never decorative
  working: 'oklch(.8 .13 80)',    // an agent is working. filled = running, hollow = needs you
  bad: '#c96b6b',                 // a run ended badly. nothing else
  link: '#7c8fd0',                // a link out. nothing else
} as const;

export const mono = "'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace";
export const sans = "-apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, sans-serif";

export const t = {
  meta: { fontFamily: mono, fontSize: 10.5, color: c.meta, lineHeight: 1.4 },
  label: { fontFamily: mono, fontSize: 10, letterSpacing: '.14em', textTransform: 'uppercase', color: c.meta },
  card: { fontSize: 13, lineHeight: 1.45, color: c.textCard },
  key: { fontFamily: mono, fontSize: 11, color: '#66666d' },
} as const;

export const RAIL_W = 56;
export const FLYOUT_W = 244;
