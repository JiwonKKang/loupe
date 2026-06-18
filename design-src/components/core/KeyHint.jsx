import React from 'react';

/**
 * KeyHint — the quiet keyboard affordance that defines Loupe's feel.
 * Renders one or more key glyphs and, optionally, a dim label beside them.
 * Used along the bottom edge of the review surface as an always-present hint.
 */
export function KeyHint({ keys, label, tone = 'dim', size = 'md', style = {}, ...rest }) {
  const keyList = Array.isArray(keys) ? keys : [keys];
  const sizes = {
    sm: { fs: 'var(--text-xs)', kp: '1px 5px', kf: '10px' },
    md: { fs: 'var(--text-sm)', kp: '2px 7px', kf: 'var(--text-xs)' },
  };
  const s = sizes[size] || sizes.md;
  const tones = {
    dim:    { fg: 'var(--text-tertiary)', kbg: 'var(--surface-overlay)', kbd: 'var(--border-default)', kfg: 'var(--text-secondary)' },
    pass:   { fg: 'var(--pass)',  kbg: 'var(--pass-dim)',  kbd: 'var(--pass-line)',  kfg: 'var(--pass)' },
    flag:   { fg: 'var(--flag)',  kbg: 'var(--flag-dim)',  kbd: 'var(--flag-line)',  kfg: 'var(--flag)' },
    accent: { fg: 'var(--accent)',kbg: 'var(--accent-dim)',kbd: 'var(--accent-line)',kfg: 'var(--accent)' },
  };
  const t = tones[tone] || tones.dim;

  return (
    <span style={{
      display: 'inline-flex', alignItems: 'center', gap: 7,
      color: t.fg, font: `var(--weight-medium) ${s.fs}/1 var(--font-ui)`,
      letterSpacing: 'var(--tracking-wide)', userSelect: 'none', ...style,
    }} {...rest}>
      <span style={{ display: 'inline-flex', gap: 4 }}>
        {keyList.map((k, i) => (
          <kbd key={i} style={{
            display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
            minWidth: 18, padding: s.kp, borderRadius: 'var(--radius-sm)',
            background: t.kbg, border: `1px solid ${t.kbd}`, color: t.kfg,
            font: `var(--weight-medium) ${s.kf}/1 var(--font-mono)`, letterSpacing: 0,
          }}>{k}</kbd>
        ))}
      </span>
      {label && <span>{label}</span>}
    </span>
  );
}
