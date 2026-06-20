import React from 'react';

/**
 * Thread — an inline, GitHub-style conversation anchored to a code line.
 * Collapses to a small count badge; expands to show alternating you / AI
 * replies with a Resolve action and a reply box.
 */
export function Thread({
  messages = [],
  resolved = false,
  pending = false,
  collapsed = false,
  onToggle,
  onResolve,
  onSend,
  style = {},
}) {
  const [draft, setDraft] = React.useState('');

  if (collapsed) {
    const hasCommand = messages.some((m) => m.kind === 'command');
    return (
      <button onClick={onToggle} style={{
        display: 'inline-flex', alignItems: 'center', gap: 6,
        height: 22, padding: '0 9px', borderRadius: 'var(--radius-pill)',
        background: resolved ? 'var(--surface-overlay)' : 'var(--accent-dim)',
        border: `1px solid ${resolved ? 'var(--border-default)' : 'var(--accent-line)'}`,
        color: resolved ? 'var(--text-tertiary)' : 'var(--accent)',
        font: `var(--weight-medium) 11px/1 var(--font-ui)`,
        cursor: 'pointer',
        animation: 'loupe-thread-badge-in var(--dur-base) var(--ease-out)',
        ...style,
      }}>
        <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor"
          strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
        </svg>
        {messages.length}{resolved ? ' · resolved' : ''}
        {hasCommand && !resolved && (
          <span title="Has a change request" style={{ width: 5, height: 5, borderRadius: 999,
            background: 'var(--flag)', marginLeft: 1 }} />
        )}
      </button>
    );
  }

  return (
    <div style={{
      position: 'relative',
      background: 'var(--surface-overlay)', border: '1px solid var(--border-subtle)',
      borderRadius: 'var(--radius-md)', boxShadow: 'var(--shadow-sm)',
      overflow: 'hidden', transformOrigin: 'top left',
      animation: 'loupe-thread-in var(--dur-slow) var(--ease-out)',
      padding: '12px 14px', ...style,
    }}>
      {/* quiet top-right actions: collapse + resolve */}
      <div style={{ position: 'absolute', top: 9, right: 10, display: 'flex', gap: 2 }}>
        <button onClick={onToggle} title="Collapse" style={{
          display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
          width: 24, height: 24, borderRadius: 'var(--radius-sm)', cursor: 'pointer',
          background: 'transparent', border: 'none', color: 'var(--text-tertiary)' }}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor"
            strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round"><path d="M18 15l-6-6-6 6" /></svg>
        </button>
        <button onClick={onResolve} title={resolved ? 'Resolved' : 'Resolve'} style={{
          display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
          width: 24, height: 24, borderRadius: 'var(--radius-sm)', cursor: 'pointer',
          background: 'transparent', border: 'none',
          color: resolved ? 'var(--pass)' : 'var(--text-tertiary)' }}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor"
            strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round"><path d="M20 6 9 17l-5-5" /></svg>
        </button>
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 12, paddingRight: 44 }}>
        {messages.map((m, i) => {
          const ai = m.author === 'ai';
          const command = m.kind === 'command';
          const accent = command ? 'var(--flag)' : 'var(--accent)';
          const label = command ? '요청' : '질문';
          const who = ai ? 'Loupe' : (m.name || 'You');
          const whoFg = ai ? 'var(--accent)' : 'var(--text-primary)';
          return (
            <div key={i}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 7, marginBottom: 3 }}>
                <span style={{ font: 'var(--weight-semibold) var(--text-xs)/1.2 var(--font-ui)',
                  color: whoFg }}>{who}</span>
                {!ai && <span style={{ font: 'var(--weight-medium) 10px/1 var(--font-ui)', color: accent }}>{label}</span>}
              </div>
              <div style={{ font: 'var(--text-sm)/var(--leading-snug) var(--font-ui)',
                color: ai ? 'var(--text-secondary)' : 'var(--text-primary)', textWrap: 'pretty' }}>{m.text}</div>
            </div>
          );
        })}

        {/* AI is composing a real answer — a quiet "thinking" row with a spinner.
            Shows beneath the messages (works even when messages is empty). */}
        {pending && (
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <span style={{ width: 12, height: 12, borderRadius: 999, flex: 'none',
              border: '2px solid var(--accent-line)', borderTopColor: 'var(--accent)',
              animation: 'loupe-spin 0.7s linear infinite' }} />
            <span style={{ font: 'var(--text-sm)/var(--leading-snug) var(--font-ui)',
              color: 'var(--text-tertiary)' }}>AI가 생각 중…</span>
          </div>
        )}
      </div>

      {!resolved && (
        <div style={{ marginTop: 11, paddingTop: 9, borderTop: '1px solid var(--border-subtle)' }}>
          <textarea
            value={draft}
            rows={1}
            onChange={(e) => { setDraft(e.target.value);
              e.target.style.height = 'auto'; e.target.style.height = e.target.scrollHeight + 'px'; }}
            onKeyDown={(e) => {
              if (e.key !== 'Enter' || e.shiftKey || !draft.trim()) return;
              e.preventDefault();
              const kind = (e.metaKey || e.ctrlKey) ? 'command' : 'question';
              onSend && onSend(draft.trim(), kind);
              setDraft(''); e.target.style.height = 'auto';
            }}
            placeholder="질문을 남기거나, 변경을 요청하세요…"
            style={{
              width: '100%', boxSizing: 'border-box', resize: 'none', overflow: 'hidden',
              minHeight: 20, maxHeight: 160, display: 'block',
              background: 'transparent', border: 'none',
              color: 'var(--text-primary)', font: 'var(--text-sm)/var(--leading-snug) var(--font-ui)', outline: 'none',
            }}
          />
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginTop: 8 }}>
            <span style={{ flex: 1 }} />
            {(() => {
              const has = draft.trim().length > 0;
              const send = (kind) => { if (has) { onSend && onSend(draft.trim(), kind); setDraft(''); } };
              const btn = (label, kbd, kind, fg, bg, bd) => (
                <button onClick={() => send(kind)} disabled={!has} title={label} style={{
                  display: 'inline-flex', alignItems: 'center', gap: 4, height: 20, padding: '0 7px',
                  borderRadius: 'var(--radius-sm)', cursor: has ? 'pointer' : 'default',
                  background: bg, border: `1px solid ${bd}`, color: fg,
                  opacity: has ? 1 : 0.4, transition: 'var(--t-hover)',
                  font: 'var(--weight-medium) 10px/1 var(--font-ui)', whiteSpace: 'nowrap' }}>
                  {label}
                  <kbd style={{ display: 'inline-flex', alignItems: 'center', padding: '1px 3px',
                    borderRadius: 3, background: 'rgba(0,0,0,0.18)',
                    border: '1px solid rgba(255,255,255,0.14)', font: '8px/1 var(--font-mono)' }}>{kbd}</kbd>
                </button>
              );
              return (
                <React.Fragment>
                  {btn('질문', '⏎', 'question', 'var(--accent)', 'var(--accent-dim)', 'var(--accent-line)')}
                  {btn('요청', '⌘⏎', 'command', 'var(--flag)', 'var(--flag-dim)', 'var(--flag-line)')}
                </React.Fragment>
              );
            })()}
          </div>
        </div>
      )}
    </div>
  );
}
