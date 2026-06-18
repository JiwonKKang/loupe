/* Loupe UI kit — the loading mark. The dot holds steady while its halo
   breathes and rings pulse outward. Used while the diff is being read. */

function LoupeLoader({ label = 'Reading the diff…', full = false }) {
  const wrap = full
    ? { position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column',
        alignItems: 'center', justifyContent: 'center', gap: 30, background: 'var(--bg-base)' }
    : { display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 30 };

  return (
    <div style={wrap}>
      <div style={{ position: 'relative', width: 80, height: 80,
        display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
        <span style={{ position: 'absolute', top: '50%', left: '50%', width: 13, height: 13,
          borderRadius: 999, background: 'var(--accent)', transform: 'translate(-50%,-50%)',
          animation: 'loupe-halo 2s var(--ease-out) infinite' }} />
        <span style={{ position: 'absolute', top: '50%', left: '50%', width: 13, height: 13,
          borderRadius: 999, background: 'var(--accent)', transform: 'translate(-50%,-50%)',
          animation: 'loupe-halo 2s var(--ease-out) infinite', animationDelay: '1s' }} />
        <span style={{ position: 'relative', zIndex: 2, width: 13, height: 13, borderRadius: 999,
          background: 'var(--accent)', animation: 'loupe-core-glow 2s var(--ease-soft) infinite' }} />
      </div>
      <div style={{ font: 'var(--text-sm)/1 var(--font-ui)', color: 'var(--text-tertiary)',
        letterSpacing: 'var(--tracking-wide)', whiteSpace: 'nowrap' }}>{label}</div>
    </div>
  );
}

window.LoupeLoader = LoupeLoader;
