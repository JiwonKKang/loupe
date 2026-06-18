import React from 'react';

/**
 * Badge — compact status / count marker.
 * Soft "dim" fills by default; matches the semantic palette.
 */
export function Badge({ tone = 'neutral', icon = null, soft = true, style = {}, children, ...rest }) {
  const tones = {
    neutral: { fg: 'var(--text-secondary)', bg: 'var(--surface-overlay)', bd: 'var(--border-default)' },
    pass:    { fg: 'var(--pass)',   bg: 'var(--pass-dim)',   bd: 'var(--pass-line)' },
    flag:    { fg: 'var(--flag)',   bg: 'var(--flag-dim)',   bd: 'var(--flag-line)' },
    accent:  { fg: 'var(--accent)', bg: 'var(--accent-dim)', bd: 'var(--accent-line)' },
  };
  const t = tones[tone] || tones.neutral;
  const solid = !soft;

  return (
    <span style={{
      display: 'inline-flex', alignItems: 'center', gap: 5,
      height: 22, padding: '0 9px', borderRadius: 'var(--radius-pill)',
      font: `var(--weight-medium) var(--text-xs)/1 var(--font-ui)`,
      letterSpacing: 'var(--tracking-wide)',
      color: solid ? 'var(--text-on-accent)' : t.fg,
      background: solid ? t.fg : t.bg,
      border: `1px solid ${solid ? 'transparent' : t.bd}`,
      whiteSpace: 'nowrap', ...style,
    }} {...rest}>
      {icon && <span style={{ display: 'inline-flex', flex: 'none', marginLeft: -1 }}>{icon}</span>}
      {children}
    </span>
  );
}
