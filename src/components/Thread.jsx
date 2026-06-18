import React from 'react';

/**
 * Thread — an inline, GitHub-style conversation anchored to a code line.
 * Collapses to a small count badge; expands to show alternating you / AI
 * replies with a Resolve action and a reply box.
 */
export function Thread({
  messages = [],
  resolved = false,
  collapsed = false,
  onToggle,
  onResolve,
  onSend,
  style = {},
}) {
  const [draft, setDraft] = React.useState('');

  if (collapsed) {
    return (
      <button onClick={onToggle} style={{
        display: 'inline-flex', alignItems: 'center', gap: 7,
        height: 26, padding: '0 11px', borderRadius: 'var(--radius-pill)',
        background: resolved ? 'var(--surface-overlay)' : 'var(--accent-dim)',
        border: `1px solid ${resolved ? 'var(--border-default)' : 'var(--accent-line)'}`,
        color: resolved ? 'var(--text-tertiary)' : 'var(--accent)',
        font: `var(--weight-medium) var(--text-xs)/1 var(--font-ui)`,
        cursor: 'pointer', ...style,
      }}>
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor"
          strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
        </svg>
        {messages.length}{resolved ? ' · resolved' : ''}
      </button>
    );
  }

  return (
    <div style={{
      background: 'var(--surface-overlay)', border: '1px solid var(--border-default)',
      borderRadius: 'var(--radius-lg)', boxShadow: 'var(--shadow-pop)',
      overflow: 'hidden', ...style,
    }}>
      <div style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        padding: '10px 14px', borderBottom: '1px solid var(--border-subtle)',
      }}>
        <button onClick={onToggle} title="Collapse thread" style={{
          display: 'inline-flex', alignItems: 'center', gap: 6,
          background: 'transparent', border: 'none', cursor: 'pointer', padding: 0,
          color: 'var(--text-tertiary)',
          font: `var(--weight-medium) var(--text-xs)/1 var(--font-ui)`,
          letterSpacing: 'var(--tracking-wide)', textTransform: 'uppercase',
        }}>
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor"
            strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M18 15l-6-6-6 6" />
          </svg>
          {resolved ? 'Resolved' : 'Thread'}
        </button>
        <button onClick={onResolve} style={{
          display: 'inline-flex', alignItems: 'center', gap: 5,
          background: 'transparent', border: 'none', cursor: 'pointer', padding: 0,
          color: resolved ? 'var(--pass)' : 'var(--text-secondary)',
          font: `var(--weight-medium) var(--text-xs)/1 var(--font-ui)`,
        }}>
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor"
            strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round">
            <path d="M20 6 9 17l-5-5" />
          </svg>
          {resolved ? 'Resolved' : 'Resolve'}
        </button>
      </div>

      <div style={{ padding: '6px 14px 4px', display: 'flex', flexDirection: 'column', gap: 2 }}>
        {messages.map((m, i) => {
          const ai = m.author === 'ai';
          return (
            <div key={i} style={{ padding: '10px 0',
              borderTop: i ? '1px solid var(--border-subtle)' : 'none' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6 }}>
                <span style={{
                  width: 20, height: 20, borderRadius: 999, flex: 'none',
                  display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
                  background: ai ? 'var(--accent-dim)' : 'var(--surface-card)',
                  border: `1px solid ${ai ? 'var(--accent-line)' : 'var(--border-default)'}`,
                  color: ai ? 'var(--accent)' : 'var(--text-secondary)',
                  font: `var(--weight-semibold) 9px/1 var(--font-ui)`,
                }}>{ai ? 'AI' : (m.name || 'You').slice(0, 1).toUpperCase()}</span>
                <span style={{ font: `var(--weight-medium) var(--text-sm)/1 var(--font-ui)`,
                  color: 'var(--text-primary)' }}>{ai ? 'Loupe AI' : (m.name || 'You')}</span>
                {m.time && <span style={{ font: `var(--text-xs)/1 var(--font-ui)`,
                  color: 'var(--text-faint)' }}>{m.time}</span>}
              </div>
              <div style={{ font: `var(--text-base)/var(--leading-normal) var(--font-ui)`,
                color: 'var(--text-secondary)', paddingLeft: 28 }}>{m.text}</div>
            </div>
          );
        })}
      </div>

      {!resolved && (
        <div style={{ display: 'flex', gap: 8, padding: '10px 14px 14px',
          borderTop: '1px solid var(--border-subtle)' }}>
          <input
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter' && draft.trim()) { onSend && onSend(draft.trim()); setDraft(''); } }}
            placeholder="Ask the AI about this line…"
            style={{
              flex: 1, height: 34, padding: '0 12px', borderRadius: 'var(--radius-md)',
              background: 'var(--surface-inset)', border: '1px solid var(--border-default)',
              color: 'var(--text-primary)', font: `var(--text-sm)/1 var(--font-ui)`, outline: 'none',
            }}
          />
        </div>
      )}
    </div>
  );
}
