/* Loupe UI kit — the loading mark. The dot holds steady while its halo
   breathes and rings pulse outward. While analyzing, the label cycles
   through dataflow-clustering stages, each fading into the next. */

function LoupeLoader({ label = '변경분을 읽는 중…', full = false, stages = null }) {
  const wrap = full
    ? { position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column',
        alignItems: 'center', justifyContent: 'center', gap: 30, background: 'var(--bg-base)' }
    : { display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 30 };

  // when `stages` is given, walk through them on a timer (last one holds).
  const [step, setStep] = React.useState(0);
  React.useEffect(() => {
    if (!stages || stages.length < 2) return;
    if (step >= stages.length - 1) return;
    const t = setTimeout(() => setStep((s) => s + 1), 760);
    return () => clearTimeout(t);
  }, [stages, step]);

  const current = stages ? stages[Math.min(step, stages.length - 1)] : label;
  const total = stages ? stages.length : 1;

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

      <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 16 }}>
        {/* the cycling label — re-keyed so each phrase fades in */}
        <div key={current} style={{ font: 'var(--text-sm)/1 var(--font-ui)', color: 'var(--text-secondary)',
          letterSpacing: 'var(--tracking-wide)', whiteSpace: 'nowrap',
          animation: 'loupe-stage-in var(--dur-base) var(--ease-out)' }}>{current}</div>

        {/* quiet progress dots, one per stage */}
        {stages && total > 1 && (
          <div style={{ display: 'flex', gap: 6 }}>
            {stages.map((_, i) => (
              <span key={i} style={{ width: 5, height: 5, borderRadius: 999,
                background: i <= step ? 'var(--accent)' : 'var(--border-default)',
                transition: 'background var(--dur-base) var(--ease-soft)' }} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

window.LoupeLoader = LoupeLoader;
