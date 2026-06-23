/* Loupe UI kit — review summary: verdict tallies + change requests as a
   markdown todo checklist, with "Copy all". */

import React from 'react';
import { Button } from '../components/Button';
import { Badge } from '../components/Badge';
import { KeyHint } from '../components/KeyHint';
import ProjectMenu from '../components/ProjectMenu';

export default function SummaryScreen(props) {
  const { cards, verdicts, threads, onRestart, project, base, target, onChangeProject,
    prStatus, approveState, onApprovePr } = props;
  const [copied, setCopied] = React.useState(false);
  // PR-approve is a two-step, explicit confirm (names the exact PR) — never one-click.
  const [confirming, setConfirming] = React.useState(false);

  const passed = cards.filter((c) => verdicts[c.id] === 'pass').length;
  const open = threads.filter((t) => !t.resolved);
  // a card with unresolved threads IS the flag
  const flaggedCards = new Set(open.map((t) => t.cardId));
  const flagged = flaggedCards.size;
  // All-pass = every card passed and no thread left open. Only then do we offer PR approval.
  const allPass = cards.length > 0 && passed === cards.length && open.length === 0;

  // file:line reference for a thread
  const refOf = (t) => {
    const c = cards.find((x) => x.id === t.cardId);
    if (!c) return '';
    const ln = c.lines[t.lineN];
    return c.path + (ln ? ':' + ln.n : '');
  };

  // Action items = every change request (⌘⏎ command) left across all threads.
  const commands = [];
  threads.forEach((t) => t.messages.forEach((m) => {
    if (m.kind === 'command') commands.push({ text: m.text, symbol: t.symbol, ref: refOf(t) });
  }));

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
    '### Change requests',
    ...(commands.length ? commands.map((c) => `- [ ] ${c.symbol} (${c.ref}) — ${c.text}`) : ['- none']),
  ].join('\n');

  return (
    <div data-tauri-drag-region style={{ position: 'absolute', inset: 0, background: 'var(--bg-base)' }}>
      {/* top-left project / branch menu — same as the review screen, so you can
         switch projects or re-run from the complete screen too. Pinned (outside the
         scroll area) so it stays put even when the summary is long. */}
      {onChangeProject && (
        <ProjectMenu project={project} base={base} target={target} onChangeProject={onChangeProject} prominent />
      )}
      <div style={{ position: 'absolute', inset: 0, display: 'flex', justifyContent: 'center',
        alignItems: 'flex-start', overflowY: 'auto', padding: '72px 24px' }}>
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

        {/* action items — the change requests left during review */}
        <div style={{ display: 'flex', alignItems: 'center', marginBottom: 12 }}>
          <span style={{ font: 'var(--weight-semibold) var(--text-md)/1 var(--font-ui)',
            color: 'var(--text-primary)' }}>Change requests</span>
          <span style={{ marginLeft: 8, font: 'var(--text-sm)/1 var(--font-mono)', color: 'var(--text-faint)' }}>{commands.length}</span>
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
          {commands.length === 0 && (
            <div style={{ padding: '22px 22px', font: 'var(--text-base)/1.5 var(--font-ui)',
              color: 'var(--text-tertiary)' }}>No change requests — nothing to act on.</div>
          )}
          {commands.map((c, i) => (
            <div key={i} style={{ display: 'flex', gap: 13, padding: '15px 22px',
              borderTop: i ? '1px solid var(--border-subtle)' : 'none' }}>
              <span style={{ width: 17, height: 17, borderRadius: 5, flex: 'none', marginTop: 1,
                border: '1.5px solid var(--border-strong)' }} />
              <div style={{ minWidth: 0 }}>
                <div style={{ display: 'flex', alignItems: 'baseline', gap: 9, flexWrap: 'wrap' }}>
                  <span style={{ font: 'var(--weight-medium) var(--text-sm)/1.4 var(--font-mono)',
                    color: 'var(--accent)' }}>{c.symbol}</span>
                  <span style={{ font: 'var(--text-xs)/1.4 var(--font-mono)',
                    color: 'var(--text-faint)' }}>{c.ref}</span>
                </div>
                <div style={{ font: 'var(--text-base)/1.5 var(--font-ui)', color: 'var(--text-secondary)',
                  marginTop: 3, textWrap: 'pretty' }}>{c.text}</div>
              </div>
            </div>
          ))}
        </div>

        {/* PR approval — only on an ALL-PASS review, and only when an OPEN PR exists for
            the reviewed branch. Delegated to the user's gh CLI; approving is an explicit
            two-step confirm that names the exact PR (outward-facing, hard to reverse). */}
        {prStatus && prStatus.state !== 'unknown' && (
          <div style={{ marginTop: 28, padding: '16px 18px', borderRadius: 'var(--radius-lg)',
            border: '1px solid var(--border-subtle)', background: 'var(--surface-inset)' }}>
            {prStatus.state === 'loading' && (
              <div style={{ font: 'var(--text-sm)/1.4 var(--font-ui)', color: 'var(--text-tertiary)' }}>
                PR 상태 확인 중…
              </div>
            )}
            {prStatus.state === 'none' && (
              <div style={{ font: 'var(--text-sm)/1.4 var(--font-ui)', color: 'var(--text-tertiary)' }}>
                이 브랜치(<span style={{ font: 'var(--text-sm)/1 var(--font-mono)' }}>{target}</span>)에 열린 PR이 없어요.
              </div>
            )}
            {prStatus.state === 'error' && (
              <div style={{ font: 'var(--text-sm)/1.4 var(--font-ui)', color: 'var(--text-tertiary)' }}>
                PR 상태를 확인할 수 없어요 — <span style={{ font: 'var(--text-sm)/1 var(--font-mono)' }}>gh</span> 설치/로그인(<span style={{ font: 'var(--text-sm)/1 var(--font-mono)' }}>gh auth login</span>)을 확인하세요.
              </div>
            )}
            {prStatus.state === 'closed' && prStatus.pr && (
              <div style={{ font: 'var(--text-sm)/1.4 var(--font-ui)', color: 'var(--text-tertiary)' }}>
                PR #{prStatus.pr.number}는 이미 {prStatus.pr.state === 'MERGED' ? '머지' : '닫힌'}됐어요.
              </div>
            )}
            {prStatus.state === 'open' && prStatus.pr && (
              <div>
                <div style={{ display: 'flex', alignItems: 'baseline', gap: 8, marginBottom: 12, minWidth: 0 }}>
                  <span style={{ font: 'var(--weight-semibold) var(--text-base)/1 var(--font-mono)',
                    color: 'var(--text-secondary)', flex: 'none' }}>#{prStatus.pr.number}</span>
                  <span style={{ font: 'var(--text-base)/1.3 var(--font-ui)', color: 'var(--text-primary)',
                    overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{prStatus.pr.title}</span>
                </div>
                {!allPass && approveState !== 'approved' && (
                  <div style={{ marginBottom: 10, font: 'var(--text-sm)/1.4 var(--font-ui)', color: 'var(--flag)' }}>
                    아직 통과 안 된 카드나 미해결 스레드가 있어요 — 그래도 승인할 수 있어요.
                  </div>
                )}
                {approveState === 'approved' ? (
                  <div style={{ display: 'inline-flex', alignItems: 'center', gap: 7,
                    font: 'var(--weight-medium) var(--text-sm)/1 var(--font-ui)', color: 'var(--pass)' }}>
                    <Ico d={check} w={15} /> PR #{prStatus.pr.number} 승인됨
                  </div>
                ) : confirming ? (
                  <div style={{ display: 'flex', alignItems: 'center', gap: 12, flexWrap: 'wrap' }}>
                    <span style={{ font: 'var(--text-sm)/1.4 var(--font-ui)', color: 'var(--text-secondary)' }}>
                      이 PR을 <b style={{ color: 'var(--text-primary)' }}>승인</b>할까요? 되돌리기 어려운 공개 작업이에요.
                    </span>
                    <div style={{ display: 'flex', gap: 8, marginLeft: 'auto' }}>
                      <Button variant="ghost" size="sm" disabled={approveState === 'approving'}
                        onClick={() => setConfirming(false)}>취소</Button>
                      <Button variant="pass" size="sm" disabled={approveState === 'approving'}
                        onClick={() => { onApprovePr && onApprovePr(); }}>
                        {approveState === 'approving' ? '승인 중…' : `Approve #${prStatus.pr.number}`}
                      </Button>
                    </div>
                  </div>
                ) : (
                  <Button variant="pass" size="sm" icon={<Ico d={check} w={14} />}
                    onClick={() => setConfirming(true)}>Approve PR</Button>
                )}
              </div>
            )}
          </div>
        )}

        {/* verdict-by-card list */}
        <div style={{ marginTop: 34, display: 'flex', alignItems: 'center', gap: 14 }}>
          <Button variant="secondary" onClick={onRestart}>Review again</Button>
          <div style={{ marginLeft: 'auto', opacity: 'var(--dim-rest)' }}>
            <KeyHint keys="⌘⏎" label="submit review" size="sm" />
          </div>
        </div>
      </div>
      </div>
    </div>
  );
}
