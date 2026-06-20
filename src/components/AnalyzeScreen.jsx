/* Loupe — the analysis pipeline screen (replaces the simple spinner).
   Unlike the design-system mock (which fakes the timeline on a timer), this is driven by
   REAL engine progress events streamed over `analyze://progress` and reduced into `progress`
   by App.jsx. The work is shown as it actually happens:
     1. static  — deterministic prep scans the diff (N files).
     2. review  — clusters are reviewed by the AI in PARALLEL; each finishes on its own and
                  the moment one does, its chapter slides into the queue rail on the left.
     3. final   — once every cluster is reviewed, the final ordering pass merges + orders them.

   `progress` = { phase: 'static'|'review'|'final', files, clusters:[{id,chapter,cards}],
                  reviewed: { [clusterId]: realTitle } }. */

import React from 'react';

// Reliable entrance: mounts at opacity 0, then transitions in once.
function Appear({ delay = 0, children, style = {} }) {
  const [on, setOn] = React.useState(false);
  React.useEffect(() => {
    const t = setTimeout(() => setOn(true), delay + 20);
    return () => clearTimeout(t);
  }, [delay]);
  return (
    <div style={{ ...style, opacity: on ? 1 : 0, transform: on ? 'translateY(0)' : 'translateY(10px)',
      transition: 'opacity 620ms var(--ease-out), transform 620ms var(--ease-out)' }}>
      {children}
    </div>
  );
}

const Check = ({ c = 'currentColor' }) => (
  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke={c}
    strokeWidth="2.6" strokeLinecap="round" strokeLinejoin="round"><path d="M20 6 9 17l-5-5" /></svg>
);

export function AnalyzeScreen({ progress }) {
  const phase = progress?.phase || 'static';
  const files = progress?.files || 0;
  const clusters = progress?.clusters || [];
  const reviewed = progress?.reviewed || {};

  const isDone = (c) => Object.prototype.hasOwnProperty.call(reviewed, c.id);
  const totalCards = clusters.reduce((n, c) => n + (c.cards ? c.cards.length : 0), 0);
  const doneCards = clusters.reduce((n, c) => n + (isDone(c) ? (c.cards ? c.cards.length : 0) : 0), 0);
  const doneCount = clusters.filter(isDone).length;

  const phaseLabel = phase === 'static' ? 'Running static analysis'
    : phase === 'clustering' ? 'Clustering changes'
    : phase === 'review' ? 'Reviewing clusters'
    : 'Final ordering pass';
  const phaseSub = phase === 'static' ? `Scanning the diff · ${files} file${files === 1 ? '' : 's'}`
    : phase === 'clustering' ? 'Grouping & ordering by data flow · running on your model'
    : phase === 'review' ? `${doneCount}/${clusters.length} clusters · running on your model`
    : 'Merging and ordering by data flow';

  return (
    <div style={{ position: 'absolute', inset: 0, display: 'flex', background: 'var(--bg-base)' }}>

      {/* LEFT — the queue rail, building up as clusters finish */}
      <div style={{ width: 300, flex: 'none', borderRight: '1px solid var(--border-subtle)',
        background: 'var(--bg-raised)', display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        <div style={{ padding: '22px 22px 16px', borderBottom: '1px solid var(--border-subtle)' }}>
          <div style={{ font: 'var(--weight-semibold) var(--text-sm)/1 var(--font-ui)', color: 'var(--text-primary)' }}>Review queue</div>
          <div style={{ font: 'var(--text-xs)/1 var(--font-ui)', color: 'var(--text-tertiary)', marginTop: 6 }}>
            {doneCards} of {totalCards} changes
          </div>
        </div>
        <div style={{ flex: 1, overflowY: 'auto', padding: '14px 14px 22px' }}>
          {(phase === 'static' || clusters.length === 0) && (
            <div style={{ padding: '8px 9px', font: 'var(--text-sm)/1.5 var(--font-ui)', color: 'var(--text-faint)' }}>
              Analyzing the diff…
            </div>
          )}
          {phase !== 'static' && clusters.map((c) => {
            const done = isDone(c);
            const chapter = done ? (reviewed[c.id] || c.chapter) : c.chapter;
            return (
              <div key={c.id} style={{ marginBottom: 16 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '0 8px', marginBottom: 7 }}>
                  <span style={{ flex: 1, font: 'var(--weight-semibold) 10px/1 var(--font-ui)', letterSpacing: 'var(--tracking-caps)',
                    textTransform: 'uppercase', color: done ? 'var(--text-tertiary)' : 'var(--text-faint)',
                    whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{chapter}</span>
                  {done
                    ? <span style={{ display: 'inline-flex', color: 'var(--pass)' }}><Check c="var(--pass)" /></span>
                    : <span style={{ width: 12, height: 12, borderRadius: 999, flex: 'none',
                        border: '2px solid var(--accent-line)', borderTopColor: 'var(--accent)',
                        animation: 'loupe-spin 0.7s linear infinite' }} />}
                </div>
                {done
                  ? (c.cards || []).map((sym, i) => (
                      <Appear key={sym + i} delay={i * 120}>
                        <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '6px 9px' }}>
                          <span style={{ width: 6, height: 6, borderRadius: 999, background: 'var(--text-faint)', flex: 'none' }} />
                          <span style={{ font: 'var(--text-sm)/1.2 var(--font-mono)', color: 'var(--text-secondary)',
                            whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{sym}</span>
                        </div>
                      </Appear>
                    ))
                  : null}
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
                color: 'var(--text-primary)', animation: 'loupe-stage-in var(--dur-base) var(--ease-out)' }}>{phaseLabel}</div>
              <div style={{ font: 'var(--text-sm)/1.3 var(--font-ui)', color: 'var(--text-tertiary)', marginTop: 4 }}>{phaseSub}</div>
            </div>
          </div>

          {/* pipeline footer */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 26 }}>
            {[['static', 'Static'], ['clustering', 'Clustering'], ['review', 'Review'], ['final', 'Final']].map(([k, label], i) => {
              const order = { static: 0, clustering: 1, review: 2, final: 3 };
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
