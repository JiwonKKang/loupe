import React from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

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
  unread = false,
  model = 'sonnet',
  onSetModel,
  onToggle,
  onResolve,
  onSend,
  onDelete,
  onNavigateCard,
  style = {},
}) {
  const [draft, setDraft] = React.useState('');
  // #autofocus — when an open thread mounts (created by drag/click, or expanded),
  // drop the caret straight into its composer so the reviewer can type at once.
  // ref + layout effect (rather than the autoFocus attr) so it also fires when a
  // collapsed thread is re-opened (the textarea remounts) and runs before paint.
  const taRef = React.useRef(null);
  React.useLayoutEffect(() => {
    if (!collapsed && !resolved && taRef.current) {
      taRef.current.focus({ preventScroll: true });
    }
  }, [collapsed, resolved]);
  // Collapse/resolve play a brief "fold up" exit before the parent unmounts the
  // open thread (it swaps in the small badge). We stay mounted for the animation,
  // then fire the real action. Transform/opacity only — no height animation (that
  // reflowed the virtualizer every frame).
  const [closing, setClosing] = React.useState(false);
  const closeWith = (fn) => () => { setClosing(true); setTimeout(() => fn && fn(), 175); };

  // Markdown renderers for AI replies. `loupe-card:N` links become click-to-jump
  // chips that navigate to review card N (the number from the prompt's jump list);
  // any other link is rendered as inert styled text (this is a desktop webview —
  // we don't navigate it to arbitrary AI-supplied URLs).
  const mdComponents = React.useMemo(() => ({
    a: ({ href, children }) => {
      if (typeof href === 'string' && href.startsWith('loupe-card:')) {
        const n = href.slice('loupe-card:'.length).trim();
        return (
          <button type="button" onClick={() => onNavigateCard && onNavigateCard(n)}
            title="이 카드로 이동" style={{
              display: 'inline-flex', alignItems: 'center', gap: 1,
              padding: 0, margin: 0, border: 'none', background: 'none', cursor: 'pointer',
              color: 'var(--accent)', font: 'inherit', verticalAlign: 'baseline',
              borderBottom: '1px solid var(--accent-line)', transition: 'var(--t-hover)' }}
            onMouseEnter={(e) => { e.currentTarget.style.borderBottomColor = 'var(--accent)'; }}
            onMouseLeave={(e) => { e.currentTarget.style.borderBottomColor = 'var(--accent-line)'; }}>
            {children}
            <svg width="8" height="8" viewBox="0 0 24 24" fill="none" stroke="currentColor"
              strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round"
              style={{ alignSelf: 'center', flex: 'none', opacity: 0.65 }}>
              <path d="M7 17 17 7M9 7h8v8" /></svg>
          </button>
        );
      }
      return <span style={{ color: 'var(--text-secondary)', textDecoration: 'underline', textUnderlineOffset: 2 }}>{children}</span>;
    },
  }), [onNavigateCard]);

  if (collapsed) {
    const hasCommand = messages.some((m) => m.kind === 'command');
    // #9 — a collapsed thread that has an unread AI answer (the user hasn't
    // expanded it since the reply landed) reads as "답변 왔음": the pill keeps the
    // accent tone even if resolved-styling would otherwise dim it, gains an accent
    // ring glow, and carries a pulsing accent dot. Read = expanding the thread,
    // which clears unread upstream (App.openLine), so this only shows pre-open.
    const showUnread = unread && !resolved;
    return (
      <button onClick={onToggle} title={showUnread ? '답변 왔음 — 펼쳐서 확인하세요' : undefined} style={{
        display: 'inline-flex', alignItems: 'center', gap: 6,
        height: 22, padding: '0 9px', borderRadius: 'var(--radius-pill)',
        background: resolved && !showUnread ? 'var(--surface-overlay)' : 'var(--accent-dim)',
        border: `1px solid ${resolved && !showUnread ? 'var(--border-default)' : 'var(--accent-line)'}`,
        color: resolved && !showUnread ? 'var(--text-tertiary)' : 'var(--accent)',
        font: `var(--weight-medium) 11px/1 var(--font-ui)`,
        cursor: 'pointer',
        boxShadow: showUnread ? '0 0 0 3px var(--accent-dim), 0 0 12px 1px rgba(110,139,255,0.35)' : 'none',
        animation: 'loupe-thread-badge-in var(--dur-base) var(--ease-out)',
        ...style,
      }}>
        {showUnread && (
          <span aria-label="답변 왔음" style={{ width: 6, height: 6, borderRadius: 999, flex: 'none',
            background: 'var(--accent)', marginRight: 1,
            animation: 'loupe-unread-blink 1.4s var(--ease-soft) infinite' }} />
        )}
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
      overflow: 'hidden', transformOrigin: 'top center',
      animation: closing
        ? 'loupe-thread-out 0.17s var(--ease-out) forwards'
        : 'loupe-thread-in var(--dur-slow) var(--ease-out)',
      padding: '14px 16px 13px', ...style,
    }}>
      {/* quiet top-right actions: delete · collapse · resolve */}
      <div style={{ position: 'absolute', top: 0, right: 11, display: 'flex', gap: 2 }}>
        {onDelete && (
          <button onClick={closeWith(onDelete)} title="Delete thread"
            onMouseEnter={(e) => { e.currentTarget.style.color = 'var(--flag)'; }}
            onMouseLeave={(e) => { e.currentTarget.style.color = 'var(--text-tertiary)'; }}
            style={{
              display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
              width: 28, height: 28, borderRadius: 'var(--radius-sm)', cursor: 'pointer',
              background: 'transparent', border: 'none', color: 'var(--text-tertiary)',
              transition: 'var(--t-hover)' }}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor"
              strokeWidth="2.1" strokeLinecap="round" strokeLinejoin="round">
              <path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m2 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" />
            </svg>
          </button>
        )}
        <button onClick={closeWith(onToggle)} title="Collapse" style={{
          display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
          width: 28, height: 28, borderRadius: 'var(--radius-sm)', cursor: 'pointer',
          background: 'transparent', border: 'none', color: 'var(--text-tertiary)' }}>
          <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor"
            strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round"><path d="M18 15l-6-6-6 6" /></svg>
        </button>
        <button onClick={closeWith(onResolve)} title={resolved ? 'Resolved' : 'Resolve'} style={{
          display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
          width: 28, height: 28, borderRadius: 'var(--radius-sm)', cursor: 'pointer',
          background: 'transparent', border: 'none',
          color: resolved ? 'var(--pass)' : 'var(--text-tertiary)' }}>
          <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor"
            strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round"><path d="M20 6 9 17l-5-5" /></svg>
        </button>
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 12, paddingRight: onDelete ? 86 : 52 }}>
        {messages.map((m, i) => {
          const ai = m.author === 'ai';
          const command = m.kind === 'command';
          const accent = command ? 'var(--flag)' : 'var(--accent)';
          const label = command ? '요청' : '질문';
          const who = ai ? 'Loupe' : (m.name || 'You');
          const whoFg = ai ? 'var(--accent)' : 'var(--text-primary)';
          return (
            <div key={i} style={i ? { borderTop: '1px solid var(--border-subtle)', paddingTop: 12 } : undefined}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 7, marginBottom: 4 }}>
                {/* author name a touch larger than the body so AI vs you reads at a glance */}
                <span style={{ font: 'var(--weight-semibold) var(--text-base)/1.2 var(--font-ui)',
                  color: whoFg }}>{who}</span>
                {!ai && <span style={{ font: 'var(--weight-medium) var(--text-xs)/1 var(--font-ui)', color: accent }}>{label}</span>}
              </div>
              <div style={{ font: 'var(--text-sm)/var(--leading-snug) var(--font-ui)',
                color: ai ? 'var(--text-secondary)' : 'var(--text-primary)', textWrap: 'pretty' }}>
                {ai
                  ? <div className="loupe-md"><ReactMarkdown remarkPlugins={[remarkGfm]} components={mdComponents} urlTransform={(u) => u}>{m.text}</ReactMarkdown></div>
                  : m.text}
              </div>
            </div>
          );
        })}

        {/* AI is composing a real answer — a quiet "thinking" row with a spinner.
            Shows beneath the messages (works even when messages is empty). */}
        {pending && (
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            {/* #1 — AI 대기 로딩 = 로고 로딩. A small accent core that breathes with
                the same loupe-core-glow halo the brand mark uses (not a generic
                spinner ring). The 4px margin leaves room for the glow box-shadow. */}
            <span style={{ width: 10, height: 10, borderRadius: 999, flex: 'none',
              margin: 4, background: 'var(--accent)',
              animation: 'loupe-core-glow 2s var(--ease-soft) infinite' }} />
            <span style={{ font: 'var(--text-sm)/var(--leading-snug) var(--font-ui)',
              color: 'var(--text-tertiary)' }}>AI가 생각 중…</span>
          </div>
        )}
      </div>

      {!resolved && (
        <div style={{ marginTop: 11, paddingTop: 9, borderTop: '1px solid var(--border-subtle)' }}>
          {/* #model + composer: the model dropdown sits to the LEFT of the textarea. */}
          <div style={{ display: 'flex', alignItems: 'flex-start', gap: 10 }}>
          <ModelMenu model={model} onSetModel={onSetModel} />
          <textarea
            ref={taRef}
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
              flex: 1, minWidth: 0, boxSizing: 'border-box', resize: 'none', overflow: 'hidden',
              minHeight: 20, maxHeight: 160, display: 'block', marginTop: 3,
              background: 'transparent', border: 'none',
              color: 'var(--text-primary)', font: 'var(--text-sm)/var(--leading-snug) var(--font-ui)', outline: 'none',
            }}
          />
          </div>
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

/**
 * ModelMenu — a compact dropdown to the left of the composer that picks which
 * model answers THIS thread (Sonnet / Haiku). A custom popover (mirroring the
 * project's BranchSelect) rather than a native <select>. The popover is
 * `position: fixed` (anchored off the trigger's rect) so it escapes the
 * thread's `overflow: hidden` clip — the reason a native select was used before.
 */
const MODEL_OPTIONS = [
  { value: 'sonnet', label: 'Sonnet' },
  { value: 'haiku', label: 'Haiku' },
];
function ModelMenu({ model = 'sonnet', onSetModel }) {
  const [open, setOpen] = React.useState(false);
  const [rect, setRect] = React.useState(null);
  const trigRef = React.useRef(null);
  const popRef = React.useRef(null);
  const enabled = !!onSetModel;

  React.useEffect(() => {
    if (!open) return;
    const close = (e) => {
      if (trigRef.current && trigRef.current.contains(e.target)) return;
      if (popRef.current && popRef.current.contains(e.target)) return;
      setOpen(false);
    };
    const onKey = (e) => { if (e.key === 'Escape') setOpen(false); };
    document.addEventListener('mousedown', close);
    document.addEventListener('keydown', onKey);
    return () => { document.removeEventListener('mousedown', close); document.removeEventListener('keydown', onKey); };
  }, [open]);

  const toggle = () => {
    if (!enabled) return;
    if (!open && trigRef.current) setRect(trigRef.current.getBoundingClientRect());
    setOpen((v) => !v);
  };
  const current = MODEL_OPTIONS.find((o) => o.value === model) || MODEL_OPTIONS[0];

  return (
    <React.Fragment>
      <button ref={trigRef} onClick={toggle} type="button" title="이 스레드에 답할 모델"
        style={{ flex: 'none', display: 'inline-flex', alignItems: 'center', gap: 5,
          height: 22, padding: '0 8px 0 10px', borderRadius: 'var(--radius-pill)',
          background: 'var(--surface-inset)',
          border: `1px solid ${open ? 'var(--border-strong)' : 'var(--border-subtle)'}`,
          color: open ? 'var(--text-primary)' : 'var(--text-secondary)',
          font: 'var(--weight-medium) 11px/1 var(--font-ui)',
          cursor: enabled ? 'pointer' : 'default', outline: 'none' }}>
        <span>{current.label}</span>
        <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4"
          strokeLinecap="round" strokeLinejoin="round" style={{ color: 'var(--text-faint)', flex: 'none',
            transform: open ? 'rotate(180deg)' : 'none', transition: 'transform var(--dur-base) var(--ease-soft)' }}><path d="M6 9l6 6 6-6" /></svg>
      </button>
      {open && rect && (
        // Opens UPWARD (translateY -100%) — the composer sits low in the card, so
        // a downward menu could fall off-screen. Fixed positioning dodges the
        // thread's overflow:hidden.
        <div ref={popRef} style={{ position: 'fixed', left: rect.left, top: rect.top,
          transform: 'translateY(calc(-100% - 6px))',
          minWidth: rect.width, zIndex: 60, background: 'var(--surface-overlay)',
          border: '1px solid var(--border-default)', borderRadius: 'var(--radius-md)',
          boxShadow: 'var(--shadow-pop)', padding: 4 }}>
          {MODEL_OPTIONS.map((o) => {
            const sel = o.value === model;
            return (
              <button key={o.value} type="button" onClick={() => { onSetModel(o.value); setOpen(false); }}
                style={{ display: 'flex', alignItems: 'center', gap: 8, width: '100%', padding: '7px 8px',
                  borderRadius: 'var(--radius-sm)', cursor: 'pointer', textAlign: 'left', border: 'none',
                  whiteSpace: 'nowrap', background: sel ? 'var(--accent-dim)' : 'transparent',
                  color: sel ? 'var(--text-primary)' : 'var(--text-secondary)',
                  font: 'var(--weight-medium) 12px/1 var(--font-ui)', transition: 'background var(--dur-fast) var(--ease-soft)' }}
                onMouseEnter={(e) => { if (!sel) e.currentTarget.style.background = 'var(--surface-inset)'; }}
                onMouseLeave={(e) => { if (!sel) e.currentTarget.style.background = 'transparent'; }}>
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.8"
                  strokeLinecap="round" strokeLinejoin="round" style={{ color: sel ? 'var(--accent)' : 'transparent', flex: 'none' }}><path d="M20 6 9 17l-5-5" /></svg>
                <span style={{ flex: 1 }}>{o.label}</span>
              </button>
            );
          })}
        </div>
      )}
    </React.Fragment>
  );
}
