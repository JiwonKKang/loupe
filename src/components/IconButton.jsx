import React from 'react';

/**
 * IconButton — square, icon-only control for minimal chrome.
 * Ghost by default so toolbars stay invisible until needed.
 */
export function IconButton({
  size = 'md',
  variant = 'ghost',
  active = false,
  disabled = false,
  label,
  style = {},
  children,
  ...rest
}) {
  const dims = { sm: 28, md: 34, lg: 40 };
  const d = dims[size] || dims.md;

  const variants = {
    ghost:   { bg: active ? 'var(--surface-overlay)' : 'transparent',
               fg: active ? 'var(--text-primary)' : 'var(--text-secondary)',
               bd: active ? 'var(--border-default)' : 'transparent' },
    solid:   { bg: 'var(--surface-overlay)', fg: 'var(--text-primary)', bd: 'var(--border-default)' },
  };
  const v = variants[variant] || variants.ghost;

  return (
    <button
      aria-label={label}
      title={label}
      disabled={disabled}
      style={{
        display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
        width: d, height: d, padding: 0,
        color: v.fg, background: v.bg,
        border: `1px solid ${v.bd}`, borderRadius: 'var(--radius-md)',
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.4 : 1,
        transition: 'var(--t-hover)',
        ...style,
      }}
      onMouseEnter={(e) => { if (!active && !disabled) { e.currentTarget.style.background = 'var(--surface-overlay)'; e.currentTarget.style.color = 'var(--text-primary)'; } }}
      onMouseLeave={(e) => { if (!active && !disabled) { e.currentTarget.style.background = v.bg; e.currentTarget.style.color = v.fg; } }}
      {...rest}
    >
      {children}
    </button>
  );
}
