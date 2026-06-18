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
        cursor: 'pointer',
        animation: 'loupe-thread-badge-in var(--dur-base) var(--ease-out)',
        ...style,
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
          return (
            <div key={i}>
              <div style={{ display: 'flex', alignItems: 'baseline', gap: 8, marginBottom: 3 }}>
                <span style={{ font: 'var(--weight-semibold) var(--text-sm)/1.2 var(--font-ui)',
                  color: ai ? 'var(--accent)' : 'var(--text-primary)' }}>{ai ? 'Loupe' : (m.name || 'You')}</span>
                {m.time && <span style={{ font: 'var(--text-xs)/1 var(--font-ui)',
                  color: 'var(--text-faint)' }}>{m.time}</span>}
              </div>
              <div style={{ font: 'var(--text-base)/var(--leading-normal) var(--font-ui)',
                color: 'var(--text-secondary)', textWrap: 'pretty' }}>{m.text}</div>
            </div>
          );
        })}
      </div>

      {!resolved && (
        <input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter' && draft.trim()) { onSend && onSend(draft.trim()); setDraft(''); } }}
          placeholder="Reply…"
          style={{
            width: '100%', marginTop: 12, paddingTop: 10, boxSizing: 'border-box',
            background: 'transparent', border: 'none', borderTop: '1px solid var(--border-subtle)',
            color: 'var(--text-primary)', font: 'var(--text-sm)/1.3 var(--font-ui)', outline: 'none',
          }}
        />
      )}
    </div>
  );
}
