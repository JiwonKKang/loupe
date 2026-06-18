/* Loupe — brand logo. One point of focus: an accent dot with a soft glow ring,
   set against the "Loupe" wordmark. Two forms:
     • primary lockup (default) — dot + glow + "Loupe" wordmark, laid out across.
     • app mark (markOnly)      — the dot, larger, inside a rounded icon box.
   All color / type values come from design tokens (var(--*)); nothing hardcoded.
   Spec: guidelines/brand-logo.card.html. */

import React from 'react';

// Per-size geometry. Mirrors the brand card: lg is the canonical primary lockup
// (13px dot + 6px glow, 28px word); sm/md scale the same proportions down.
const SIZES = {
  sm: { dot: 11, glow: 5, gap: 9,  word: 'var(--text-md)',  track: 'var(--tracking-snug)' },
  md: { dot: 13, glow: 6, gap: 11, word: 'var(--text-lg)',  track: 'var(--tracking-snug)' },
  lg: { dot: 13, glow: 6, gap: 13, word: 'var(--text-xl)',  track: 'var(--tracking-tight)' },
};

// App-mark box geometry, keyed off the same size token. The dot is ~2x the
// lockup dot and the box/glow scale with it — matching the 64px iconbox spec.
const MARK_SIZES = {
  sm: { box: 44, radius: 11, dot: 18, glow: 6 },
  md: { box: 54, radius: 13, dot: 22, glow: 8 },
  lg: { box: 64, radius: 15, dot: 26, glow: 9 },
};

export default function Logo({ size = 'md', markOnly = false }) {
  if (markOnly) {
    const m = MARK_SIZES[size] || MARK_SIZES.md;
    return (
      <div style={{
        width: m.box, height: m.box, borderRadius: m.radius,
        background: 'var(--surface-card)', border: '1px solid var(--border-subtle)',
        display: 'flex', alignItems: 'center', justifyContent: 'center', flex: 'none',
      }}>
        <span style={{
          width: m.dot, height: m.dot, borderRadius: 999, background: 'var(--accent)',
          boxShadow: `0 0 0 ${m.glow}px var(--accent-dim)`,
        }} />
      </div>
    );
  }

  const s = SIZES[size] || SIZES.md;
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: s.gap }}>
      <span style={{
        width: s.dot, height: s.dot, borderRadius: 999, background: 'var(--accent)',
        boxShadow: `0 0 0 ${s.glow}px var(--accent-dim)`, flex: 'none',
      }} />
      <span style={{
        font: `var(--weight-semibold) ${s.word}/1 var(--font-ui)`,
        letterSpacing: s.track, color: 'var(--text-primary)',
      }}>Loupe</span>
    </div>
  );
}
