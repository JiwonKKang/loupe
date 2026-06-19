/* Loupe UI kit — the analysis pipeline screen.
   Replaces the simple spinner. The work is shown as it happens:
     1. Static analysis scans the diff.
     2. Clusters are reviewed by the AI in PARALLEL — each finishes on its
        own, and the moment one does, its chapter slides into the queue rail
        on the left (the user watches the queue build up).
     3. Once every cluster (incl. Unclustered) is done, a final AI
        clustering pass merges and orders them.
   onDone() fires when the pipeline completes. `loop` restarts it (for the
   design-system card). */

// Reliable entrance: mounts at opacity 0, then transitions in once. Using a
// transition (not a CSS animation) avoids the keyframe-restart-stuck issue.
function Appear({ delay = 0, children, style = {} }) {
  const [on, setOn] = React.useState(false);
  React.useEffect(() => { const t = setTimeout(() => setOn(true), delay + 20); return () => clearTimeout(t); }, []);
  return (
    <div style={{ ...style, opacity: on ? 1 : 0, transform: on ? 'translateY(0)' : 'translateY(10px)',
      transition: 'opacity 620ms var(--ease-out), transform 620ms var(--ease-out)' }}>
      {children}
    </div>
  );
}

function AnalyzeScreen({ clusters, onDone, loop = false }) {
  const [phase, setPhase] = React.useState('static');     // static | review | final
  const [status, setStatus] = React.useState({});          // chapter -> reviewing | done
  const [rail, setRail] = React.useState([]);              // chapters revealed, in finish order
  const [scanned, setScanned] = React.useState(0);
  const timers = React.useRef([]);
  const FILES = 14;

  const at = (ms, fn) => timers.current.push(setTimeout(fn, ms));

  // Run the pipeline ONCE on mount. Refs keep the latest callbacks without
  // restarting the timeline when the parent re-renders.
  const onDoneRef = React.useRef(onDone); onDoneRef.current = onDone;
  const clustersRef = React.useRef(clusters); clustersRef.current = clusters;

  React.useEffect(() => {
    const run = () => {
      timers.current.forEach(clearTimeout); timers.current = [];
      const cls = clustersRef.current;
      setPhase('static'); setStatus({}); setRail([]); setScanned(0);

      for (let i = 1; i <= FILES; i++) at(38 * i, () => setScanned(i));
      const t0 = 38 * FILES + 280;                            // ~812ms

      at(t0, () => {
        setPhase('review');
        const s = {}; cls.forEach((c) => { s[c.chapter] = 'reviewing'; });
        setStatus(s);
      });

      let maxDone = 0;
      cls.forEach((c) => {
        const done = t0 + c.dur;
        maxDone = Math.max(maxDone, done);
        at(done, () => {
          setStatus((p) => ({ ...p, [c.chapter]: 'done' }));
          setRail((p) => [...p, c]);
        });
      });

      const finalStart = maxDone + 260;
      at(finalStart, () => setPhase('final'));
      at(finalStart + 1050, () => { if (loop) run(); else onDoneRef.current && onDoneRef.current(); });
    };
    run();
    return () => timers.current.forEach(clearTimeout);
  }, [loop]);

  const totalCards = clusters.reduce((n, c) => n + c.cards.length, 0);
  const doneCount = clusters.filter((c) => status[c.chapter] === 'done').length;

  const phaseLabel = phase === 'static' ? 'Running static analysis'
    : phase === 'review' ? 'Reviewing in parallel'
    : 'Final clustering pass';
  const phaseSub = phase === 'static' ? `Scanning the diff · ${scanned}/${FILES} files`
    : phase === 'review' ? `${doneCount}/${clusters.length} clusters · running on your model`
    : 'Merging and ordering by data flow';

  const Check = ({ c = 'currentColor' }) => (
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke={c}
      strokeWidth="2.6" strokeLinecap="round" strokeLinejoin="round"><path d="M20 6 9 17l-5-5" /></svg>
  );

  return (
    <div style={{ position: 'absolute', inset: 0, display: 'flex', background: 'var(--bg-base)' }}>

      {/* LEFT — the queue rail, building up as clusters finish */}
      <div style={{ width: 300, flex: 'none', borderRight: '1px solid var(--border-subtle)',
        background: 'var(--bg-raised)', display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        <div style={{ padding: '22px 22px 16px', borderBottom: '1px solid var(--border-subtle)' }}>
          <div style={{ font: 'var(--weight-semibold) var(--text-sm)/1 var(--font-ui)', color: 'var(--text-primary)' }}>Review queue</div>
          <div style={{ font: 'var(--text-xs)/1 var(--font-ui)', color: 'var(--text-tertiary)', marginTop: 6 }}>
            {rail.reduce((n, c) => n + c.cards.length, 0)} of {totalCards} changes
          </div>
        </div>
        <div style={{ flex: 1, overflowY: 'auto', padding: '14px 14px 22px' }}>
          {phase === 'static' && (
            <div style={{ padding: '8px 9px', font: 'var(--text-sm)/1.5 var(--font-ui)', color: 'var(--text-faint)' }}>
              Analyzing the diff…
            </div>
          )}
          {phase !== 'static' && clusters.map((c) => {
            const done = status[c.chapter] === 'done';
            return (
              <div key={c.chapter} style={{ marginBottom: 16 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '0 8px', marginBottom: 7 }}>
                  <span style={{ flex: 1, font: 'var(--weight-semibold) 10px/1 var(--font-ui)', letterSpacing: 'var(--tracking-caps)',
                    textTransform: 'uppercase', color: done ? 'var(--text-tertiary)' : 'var(--text-faint)',
                    whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{c.chapter}</span>
                  {done
                    ? <span style={{ display: 'inline-flex', color: 'var(--pass)' }}><Check c="var(--pass)" /></span>
                    : <span style={{ width: 12, height: 12, borderRadius: 999, flex: 'none',
                        border: '2px solid var(--accent-line)', borderTopColor: 'var(--accent)',
                        animation: 'loupe-spin 0.7s linear infinite' }} />}
                </div>
                {done
                  ? c.cards.map((sym, i) => (
                      <Appear key={sym} delay={i * 150}>
                        <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '6px 9px' }}>
                          <span style={{ width: 6, height: 6, borderRadius: 999, background: 'var(--text-faint)', flex: 'none' }} />
                          <span style={{ font: 'var(--text-sm)/1.2 var(--font-mono)', color: 'var(--text-secondary)',
                            whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{sym}</span>
                        </div>
                      </Appear>
                    ))
                  : <div style={{ padding: '2px 9px', font: 'var(--text-xs)/1.4 var(--font-ui)', color: 'var(--accent)' }}>Reviewing…</div>}
              </div>
            );
          })}
        </div>
      </div>

      {/* RIGHT — pipeline status */}
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', alignItems: 'center',
        justifyContent: 'center', padding: 40, minWidth: 0 }}>
        <div style={{ width: '100%', maxWidth: 460 }}>

          {/* mark + phase */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 16, marginBottom: 30 }}>
            <div style={{ position: 'relative', width: 40, height: 40, flex: 'none',
              display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
              <span style={{ position: 'absolute', top: '50%', left: '50%', width: 11, height: 11, borderRadius: 999,
                background: 'var(--accent)', transform: 'translate(-50%,-50%)', animation: 'loupe-halo 2s var(--ease-out) infinite' }} />
              <span style={{ position: 'relative', zIndex: 2, width: 11, height: 11, borderRadius: 999,
                background: 'var(--accent)', animation: 'loupe-core-glow 2s var(--ease-soft) infinite' }} />
            </div>
            <div style={{ minWidth: 0 }}>
              <div key={phaseLabel} style={{ font: 'var(--weight-semibold) var(--text-md)/1.2 var(--font-ui)',
                color: 'var(--text-primary)' }}>{phaseLabel}</div>
              <div style={{ font: 'var(--text-sm)/1.3 var(--font-ui)', color: 'var(--text-tertiary)', marginTop: 4 }}>{phaseSub}</div>
            </div>
          </div>

          {/* pipeline footer */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 26 }}>
            {[['static', 'Static analysis'], ['review', 'Parallel review'], ['final', 'Final clustering']].map(([k, label], i) => {
              const order = { static: 0, review: 1, final: 2 };
              const active = order[phase] === i;
              const passed = order[phase] > i;
              return (
                <React.Fragment key={k}>
                  {i > 0 && <span style={{ flex: 1, height: 1, background: passed ? 'var(--accent-line)' : 'var(--border-subtle)' }} />}
                  <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6,
                    font: 'var(--weight-medium) 10px/1 var(--font-ui)', letterSpacing: 'var(--tracking-wide)',
                    color: active ? 'var(--accent)' : passed ? 'var(--text-secondary)' : 'var(--text-faint)' }}>
                    <span style={{ width: 6, height: 6, borderRadius: 999, flex: 'none',
                      background: active ? 'var(--accent)' : passed ? 'var(--pass)' : 'var(--border-default)' }} />
                    {label}
                  </span>
                </React.Fragment>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}

window.AnalyzeScreen = AnalyzeScreen;
