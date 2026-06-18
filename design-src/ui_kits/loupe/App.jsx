/* Loupe UI kit — app shell: screen state, verdicts, threads, keyboard nav. */

function App() {
  const cards = window.LoupeData.cards;
  const [screen, setScreen] = React.useState('review'); // onboarding | review | summary
  const [index, setIndex] = React.useState(2);
  const [treeOpen, setTreeOpen] = React.useState(false);
  const [dir, setDir] = React.useState(1);
  const [verdicts, setVerdicts] = React.useState({ decodeJSON: 'pass', handleLogin: 'pass' });
  const [threads, setThreads] = React.useState([
    { id: 't1', cardId: 'validate', side: 'old', lineN: 2, symbol: 'Session.Validate', open: false, resolved: false,
      messages: [
        { author: 'you', text: 'Why drop the empty-token guard?', time: '3m', kind: 'question' },
        { author: 'ai', text: 'expired(now()) covers it — an unset token has a zero expiry, so it takes the same path and returns ErrLeaseExpired.', time: '3m' },
        { author: 'you', text: 'Add a unit test for the already-expired lease case.', time: '2m', kind: 'command' },
      ] },
  ]);
  const tid = React.useRef(2);

  const card = cards[index];

  // Derived spine items. A card with unresolved threads IS the "flag".
  const threadCount = (cid) => threads.filter((t) => t.cardId === cid).length;
  const hasUnresolved = (cid) => threads.some((t) => t.cardId === cid && !t.resolved);
  const spineItems = cards.map((c) => ({
    id: c.id, symbol: c.symbol, label: c.symbol, chapter: c.chapter,
    file: c.path.split('/').pop(), threads: threadCount(c.id),
    status: hasUnresolved(c.id) ? 'flag' : (verdicts[c.id] === 'pass' ? 'pass' : 'pending'),
  }));
  const unresolved = threads.filter((t) => !t.resolved).length;

  const goTo = (i, d) => { setDir(d); setIndex(Math.max(0, Math.min(cards.length - 1, i))); };

  const advance = () => {
    if (index >= cards.length - 1) { setScreen('summary'); return; }
    goTo(index + 1, 1);
  };
  const pass = () => { setVerdicts((p) => ({ ...p, [card.id]: 'pass' })); advance(); };
  const next = () => { if (index < cards.length - 1) goTo(index + 1, 1); else setScreen('summary'); };
  const prev = () => { if (index > 0) goTo(index - 1, -1); };
  const jumpUnresolved = () => {
    const openCardIds = threads.filter((t) => !t.resolved).map((t) => t.cardId);
    // first unresolved card after the current one, wrapping around
    for (let step = 1; step <= cards.length; step++) {
      const i = (index + step) % cards.length;
      if (openCardIds.includes(cards[i].id)) { goTo(i, i > index ? 1 : -1); return; }
    }
  };

  const openLine = (side, row) => {
    const existing = threads.find((t) => t.cardId === card.id && t.lineN === row);
    if (existing) {
      setThreads((p) => p.map((t) => t.id === existing.id ? { ...t, open: !t.open } : t));
      return;
    }
    const id = 't' + (tid.current++);
    setThreads((p) => [...p, {
      id, cardId: card.id, side: side || 'old', lineN: row, symbol: card.symbol, open: true, resolved: false,
      messages: [{ author: 'ai', text: aiSeed(card), time: 'now' }],
    }]);
  };
  const sendThread = (id, text, kind) => {
    setThreads((p) => p.map((t) => t.id === id
      ? { ...t, messages: [...t.messages, { author: 'you', text, time: 'now', kind: kind || 'question' }] } : t));
    // Only questions get an AI reply; commands are change requests for the summary.
    if (kind === 'command') return;
    setTimeout(() => {
      setThreads((p) => p.map((t) => t.id === id
        ? { ...t, messages: [...t.messages, { author: 'ai', text: 'Good question — based on the surrounding change, that path is exercised by the new lease check; the previous behavior is preserved for the non-expired case.', time: 'now' }] } : t));
    }, 650);
  };
  const resolveThread = (id) => setThreads((p) => p.map((t) => t.id === id ? { ...t, resolved: !t.resolved, open: false } : t));

  function aiSeed(c) {
    return `This change: ${c.summary.charAt(0).toLowerCase() + c.summary.slice(1)} Ask a question, or ⌘⏎ to leave a change request.`;
  }

  // Keyboard — review screen only
  React.useEffect(() => {
    if (screen !== 'review') return;
    const onKey = (e) => {
      const tag = (e.target.tagName || '').toLowerCase();
      if (tag === 'input' || tag === 'textarea' || tag === 'select') return;
      if (e.key === ' ') { e.preventDefault(); pass(); }
      else if (e.key === 'j' || e.key === 'J' || e.key === 'ArrowRight') { e.preventDefault(); next(); }
      else if (e.key === 'k' || e.key === 'K' || e.key === 'ArrowLeft') { e.preventDefault(); prev(); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [screen, index, card]);

  const cardThreads = threads.filter((t) => t.cardId === card.id);

  return (
    <React.Fragment>
      {screen === 'onboarding' && <Onboarding onFinish={() => {
        setScreen('loading');
        setTimeout(() => setScreen('review'), 2200);
      }} />}
      {screen === 'loading' && <LoupeLoader full />}
      {screen === 'review' && (
        <FileTree tree={window.LoupeData.tree} activeId={card.id} open={treeOpen}
          onToggle={() => setTreeOpen((v) => !v)}
          onSelect={(id) => { const i = cards.findIndex((c) => c.id === id); goTo(i, i > index ? 1 : -1); }} />
      )}
      {screen === 'review' && (
        <ReviewScreen
          card={card} index={index} total={cards.length} dir={dir}
          base="main" target="agent/refactor-auth" unresolved={unresolved}
          onOpenSummary={() => setScreen('summary')}
          spineItems={spineItems} onSelect={(id) => { const i = cards.findIndex((c) => c.id === id); goTo(i, i > index ? 1 : -1); }}
          verdict={verdicts[card.id]} flagged={hasUnresolved(card.id)}
          hasPrev={index > 0} hasNext={index < cards.length - 1}
          onPass={pass} onPrev={prev} onNext={next} onJumpUnresolved={jumpUnresolved}
          threads={cardThreads}
          onOpenLine={openLine} onResolve={resolveThread} onSend={sendThread}
        />
      )}
      {screen === 'summary' && (
        <SummaryScreen cards={cards} verdicts={verdicts} threads={threads}
          onRestart={() => { setIndex(0); setDir(1); setScreen('review'); }} />
      )}

      <ScreenSwitcher screen={screen} setScreen={setScreen} />
    </React.Fragment>
  );
}

function ScreenSwitcher({ screen, setScreen }) {
  const [hover, setHover] = React.useState(false);
  const tabs = [['onboarding', 'Onboarding'], ['review', 'Review'], ['summary', 'Summary']];
  return (
    <div onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)}
      title="Demo: jump between screens"
      style={{ position: 'fixed', bottom: 14, left: 14, display: 'flex', alignItems: 'center',
        gap: hover ? 2 : 5, padding: hover ? '3px' : '5px 6px',
        borderRadius: 'var(--radius-pill)', zIndex: 60,
        background: hover ? 'var(--bg-raised)' : 'transparent',
        border: `1px solid ${hover ? 'var(--border-subtle)' : 'transparent'}`,
        opacity: hover ? 1 : 0.3, transition: 'var(--t-dim), var(--t-hover)' }}>
      {tabs.map(([k, label]) => (
        hover ? (
          <button key={k} onClick={() => setScreen(k)} style={{
            padding: '4px 9px', borderRadius: 'var(--radius-pill)', cursor: 'pointer', border: 'none',
            font: 'var(--weight-medium) 10px/1 var(--font-ui)', letterSpacing: 'var(--tracking-wide)',
            background: screen === k ? 'var(--surface-overlay)' : 'transparent',
            color: screen === k ? 'var(--text-primary)' : 'var(--text-tertiary)',
            transition: 'var(--t-hover)' }}>{label}</button>
        ) : (
          <span key={k} style={{ width: 5, height: 5, borderRadius: 999,
            background: screen === k ? 'var(--text-secondary)' : 'var(--text-faint)' }} />
        )
      ))}
    </div>
  );
}

window.App = App;
