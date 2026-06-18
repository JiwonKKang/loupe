import React from 'react';

/**
 * Button — Loupe's primary action control.
 * Calm by default; semantic only when the action is Pass or Flag.
 */
export function Button({
  variant = 'secondary',
  size = 'md',
  icon = null,
  iconRight = null,
  kbd = null,
  disabled = false,
  fullWidth = false,
  style = {},
  children,
  ...rest
}) {
  const sizes = {
    sm: { h: 30, px: 12, fs: 'var(--text-sm)', gap: 7 },
    md: { h: 38, px: 16, fs: 'var(--text-base)', gap: 9 },
    lg: { h: 46, px: 22, fs: 'var(--text-md)', gap: 10 },
  };
  const s = sizes[size] || sizes.md;

  const variants = {
    primary:   { bg: 'var(--accent)',         fg: 'var(--text-on-accent)', bd: 'transparent' },
    pass:      { bg: 'var(--pass)',           fg: 'var(--text-on-accent)', bd: 'transparent' },
    flag:      { bg: 'var(--flag)',           fg: 'var(--text-on-accent)', bd: 'transparent' },
    secondary: { bg: 'var(--surface-overlay)',fg: 'var(--text-primary)',   bd: 'var(--border-default)' },
    ghost:     { bg: 'transparent',           fg: 'var(--text-secondary)', bd: 'transparent' },
  };
  const v = variants[variant] || variants.secondary;

  return (
    <button
      disabled={disabled}
      style={{
        display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
        gap: s.gap, height: s.h, padding: `0 ${s.px}px`,
        width: fullWidth ? '100%' : 'auto',
        font: `var(--weight-medium) ${s.fs}/1 var(--font-ui)`,
        letterSpacing: 'var(--tracking-snug)',
        color: v.fg, background: v.bg,
        border: `1px solid ${v.bd}`, borderRadius: 'var(--radius-md)',
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.4 : 1,
        transition: 'var(--t-hover), transform var(--dur-fast) var(--ease-soft)',
        whiteSpace: 'nowrap', userSelect: 'none', ...style,
      }}
      onMouseDown={(e) => { if (!disabled) e.currentTarget.style.transform = 'scale(0.98)'; }}
      onMouseUp={(e) => { e.currentTarget.style.transform = 'scale(1)'; }}
      onMouseLeave={(e) => { e.currentTarget.style.transform = 'scale(1)'; }}
      {...rest}
    >
      {icon && <span style={{ display: 'inline-flex', flex: 'none' }}>{icon}</span>}
      {children}
      {iconRight && <span style={{ display: 'inline-flex', flex: 'none' }}>{iconRight}</span>}
      {kbd && (
        <kbd style={{
          marginLeft: 4, padding: '2px 6px', borderRadius: 'var(--radius-sm)',
          background: 'rgba(0,0,0,0.18)', border: '1px solid rgba(255,255,255,0.14)',
          font: `var(--weight-medium) var(--text-xs)/1 var(--font-mono)`,
          letterSpacing: 0,
        }}>{kbd}</kbd>
      )}
    </button>
  );
}
