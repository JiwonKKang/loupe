/* Loupe UI kit — review summary: verdict tallies + unresolved threads as a
   markdown todo checklist, with "Copy all". */

import React from 'react';
import { Button } from '../components/Button';
import { Badge } from '../components/Badge';
import { KeyHint } from '../components/KeyHint';

export default function SummaryScreen(props) {
  const { cards, verdicts, threads, onRestart } = props;
  const [copied, setCopied] = React.useState(false);

  const passed = cards.filter((c) => verdicts[c.id] === 'pass').length;
  const open = threads.filter((t) => !t.resolved);
  // a card with unresolved threads IS the flag
  const flaggedCards = new Set(open.map((t) => t.cardId));
  const flagged = flaggedCards.size;

  // file:line reference for a thread
  const refOf = (t) => {
    const c = cards.find((x) => x.id === t.cardId);
    if (!c) return '';
    const ln = c.lines[t.lineN];
    return c.path + (ln ? ':' + ln.n : '');
  };

  const Ico = ({ d, w = 15 }) => (
    <svg width={w} height={w} viewBox="0 0 24 24" fill="none" stroke="currentColor"
      strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round"><path d={d} /></svg>
  );
  const check = 'M20 6 9 17l-5-5';
  const copy = 'M8 4v12a2 2 0 0 0 2 2h8M8 4a2 2 0 0 1 2-2h6.5L20 5.5V14a2 2 0 0 1-2 2H10a2 2 0 0 1-2-2z M8 4H6a2 2 0 0 0-2 2v12';

  const md = [
    '## Review summary',
    `- Passed: ${passed}`,
    `- Needs attention: ${flagged}`,
    '',
    '### Open threads',
    ...open.map((t) => `- [ ] ${t.symbol} (${refOf(t)}) — ${t.messages[0] ? t.messages[0].text : 'follow up'}`),
  ].join('\n');

  return (
    <div style={{ position: 'absolute', inset: 0, display: 'flex', justifyContent: 'center',
      alignItems: 'flex-start', overflowY: 'auto', background: 'var(--bg-base)', padding: '72px 24px' }}>
      <div style={{ width: 600, maxWidth: '100%' }}>

        <div style={{ font: 'var(--weight-medium) var(--text-xs)/1 var(--font-ui)',
          letterSpacing: 'var(--tracking-caps)', textTransform: 'uppercase',
          color: 'var(--text-tertiary)', marginBottom: 12 }}>Review complete</div>
        <div style={{ font: 'var(--weight-semibold) var(--text-2xl)/1.05 var(--font-ui)',
          letterSpacing: 'var(--tracking-tight)', color: 'var(--text-primary)', marginBottom: 28 }}>
          {cards.length} changes reviewed
        </div>

        {/* tallies */}
        <div style={{ display: 'flex', gap: 14, marginBottom: 34 }}>
          {[['Passed', passed, 'var(--pass)', 'var(--pass-dim)', 'var(--pass-line)'],
            ['Needs attention', flagged, 'var(--flag)', 'var(--flag-dim)', 'var(--flag-line)'],
            ['Open threads', open.length, 'var(--accent)', 'var(--accent-dim)', 'var(--accent-line)']].map(
            ([label, n, fg, bg, bd]) => (
            <div key={label} style={{ flex: 1, padding: '18px 20px', borderRadius: 'var(--radius-lg)',
              background: bg, border: `1px solid ${bd}` }}>
              <div style={{ font: 'var(--weight-semibold) var(--text-2xl)/1 var(--font-ui)',
                color: fg, fontVariantNumeric: 'tabular-nums' }}>{n}</div>
              <div style={{ font: 'var(--text-sm)/1 var(--font-ui)', color: 'var(--text-secondary)', marginTop: 7 }}>{label}</div>
            </div>
          ))}
        </div>

        {/* todo checklist */}
        <div style={{ display: 'flex', alignItems: 'center', marginBottom: 12 }}>
          <span style={{ font: 'var(--weight-semibold) var(--text-md)/1 var(--font-ui)',
            color: 'var(--text-primary)' }}>Follow-ups</span>
          <div style={{ marginLeft: 'auto' }}>
            <Button size="sm" variant="secondary"
              icon={copied ? <span style={{ color: 'var(--pass)' }}><Ico d={check} w={14} /></span> : <Ico d={copy} w={14} />}
              onClick={() => { navigator.clipboard && navigator.clipboard.writeText(md); setCopied(true); setTimeout(() => setCopied(false), 1400); }}>
              {copied ? 'Copied' : 'Copy all'}
            </Button>
          </div>
        </div>

        <div style={{ background: 'var(--surface-card)', border: '1px solid var(--border-subtle)',
          borderRadius: 'var(--radius-lg)', overflow: 'hidden' }}>
          {open.length === 0 && (
            <div style={{ padding: '22px 22px', font: 'var(--text-base)/1.5 var(--font-ui)',
              color: 'var(--text-tertiary)' }}>No open threads — everything resolved.</div>
          )}
          {open.map((t, i) => (
            <div key={t.id} style={{ display: 'flex', gap: 13, padding: '15px 22px',
              borderTop: i ? '1px solid var(--border-subtle)' : 'none' }}>
              <span style={{ width: 17, height: 17, borderRadius: 5, flex: 'none', marginTop: 1,
                border: '1.5px solid var(--border-strong)' }} />
              <div style={{ minWidth: 0 }}>
                <div style={{ display: 'flex', alignItems: 'baseline', gap: 9, flexWrap: 'wrap' }}>
                  <span style={{ font: 'var(--weight-medium) var(--text-sm)/1.4 var(--font-mono)',
                    color: 'var(--accent)' }}>{t.symbol}</span>
                  <span style={{ font: 'var(--text-xs)/1.4 var(--font-mono)',
                    color: 'var(--text-faint)' }}>{refOf(t)}</span>
                </div>
                <div style={{ font: 'var(--text-base)/1.5 var(--font-ui)', color: 'var(--text-secondary)',
                  marginTop: 3, textWrap: 'pretty' }}>{t.messages[0] ? t.messages[0].text : 'Follow up'}</div>
              </div>
            </div>
          ))}
        </div>

        {/* verdict-by-card list */}
        <div style={{ marginTop: 34, display: 'flex', alignItems: 'center', gap: 14 }}>
          <Button variant="secondary" onClick={onRestart}>Review again</Button>
          <div style={{ marginLeft: 'auto', opacity: 'var(--dim-rest)' }}>
            <KeyHint keys="⌘⏎" label="submit review" size="sm" />
          </div>
        </div>
      </div>
    </div>
  );
}
