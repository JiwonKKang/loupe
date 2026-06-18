/* Loupe UI kit — app shell: screen state, verdicts, threads, keyboard nav.
   Stage 1: real cards come from the Rust engine via invoke('load_review', …).
   Onboarding collects repoPath/base/target; everything else (verdicts/threads/
   spineItems/unresolved) is derived on the front-end from `cards`. */

import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import Onboarding from './screens/Onboarding';
import ReviewScreen from './screens/ReviewScreen';
import SummaryScreen from './screens/SummaryScreen';
import { LoupeLoader } from './components/LoupeLoader';
import { FileTree } from './components/FileTree';
import { buildTree } from './data/fixtures';
// Note: syntax color is a front-end concern — ReviewScreen imports highlightGo
// from ./data/fixtures directly. App.jsx only wires data + screen state.

export default function App() {
  const [screen, setScreen] = React.useState('onboarding'); // onboarding | review | summary
  const [range, setRange] = React.useState(null);   // { repoPath, base, target }
  const [cards, setCards] = React.useState(null);    // null = loading, [] = empty diff
  const [loadError, setLoadError] = React.useState(null);

  const [index, setIndex] = React.useState(0);
  const [dir, setDir] = React.useState(1);
  const [verdicts, setVerdicts] = React.useState({});
  const [threads, setThreads] = React.useState([]);
  const [treeOpen, setTreeOpen] = React.useState(false);
  const tid = React.useRef(1);

  // Load the review whenever a range is chosen (and on retry).
  const load = React.useCallback((r) => {
    if (!r) return;
    setCards(null);
    setLoadError(null);
    setIndex(0);
    setVerdicts({});
    setThreads([]);
    invoke('load_review', { repoPath: r.repoPath, base: r.base, target: r.target })
      .then((data) => { setCards(data.cards); setLoadError(null); })
      .catch((err) => { setLoadError(String(err)); setCards([]); });
  }, []);

  React.useEffect(() => { load(range); }, [range, load]);

  const startReview = (r) => {
    setRange({ repoPath: r.repoPath, base: r.base || 'main', target: r.target });
    setScreen('review');
  };

  // Derived (safe even when cards is null — guarded reads).
  const list = cards || [];
  const card = list[index];

  const threadCount = (cid) => threads.filter((t) => t.cardId === cid).length;
  // A card with unresolved threads IS the "flag" (Needs attention).
  const hasUnresolved = (cid) => threads.some((t) => t.cardId === cid && !t.resolved);
  const spineItems = list.map((c) => ({
    id: c.id, symbol: c.symbol, label: c.symbol, chapter: c.chapter,
    file: c.path.split('/').pop(), threads: threadCount(c.id),
    status: hasUnresolved(c.id) ? 'flag' : (verdicts[c.id] === 'pass' ? 'pass' : 'pending'),
  }));
  const unresolved = threads.filter((t) => !t.resolved).length;
  // Changed-files tree for the right-hand FileTree sidebar (built from the
  // real cards returned by the engine).
  const tree = React.useMemo(() => buildTree(list), [list]);

  const goTo = (i, d) => { setDir(d); setIndex(Math.max(0, Math.min(list.length - 1, i))); };

  const advance = () => {
    if (index >= list.length - 1) { setScreen('summary'); return; }
    goTo(index + 1, 1);
  };
  const pass = () => { if (!card) return; setVerdicts((p) => ({ ...p, [card.id]: 'pass' })); advance(); };
  const next = () => { if (index < list.length - 1) goTo(index + 1, 1); else setScreen('summary'); };
  const prev = () => { if (index > 0) goTo(index - 1, -1); };

  const openLine = (side, lineN) => {
    if (!card) return;
    // Threads are keyed by ROW now (before+after highlight together, GitHub-style);
    // `side` is only remembered so a collapsed badge sits in that column's gutter.
    const existing = threads.find((t) => t.cardId === card.id && t.lineN === lineN);
    if (existing) {
      setThreads((p) => p.map((t) => t.id === existing.id ? { ...t, open: !t.open } : t));
      return;
    }
    const id = 't' + (tid.current++);
    setThreads((p) => [...p, {
      id, cardId: card.id, side: side || 'old', lineN, symbol: card.symbol, open: true, resolved: false,
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
    const s = c.summary || 'this change.';
    return `This change: ${s.charAt(0).toLowerCase() + s.slice(1)} Ask a question, or ⌘⏎ to leave a change request.`;
  }

  // Keyboard — review screen only. Declared before any early return (hooks rule).
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
  }, [screen, index, card, list.length]);

  const cardThreads = card ? threads.filter((t) => t.cardId === card.id) : [];

  // --- Render. All hooks are declared above; guards live here. ---

  // Onboarding stands on its own (no cards needed).
  if (screen === 'onboarding') {
    return (
      <React.Fragment>
        <Onboarding onFinish={startReview} />
        <ScreenSwitcher screen={screen} setScreen={setScreen} />
      </React.Fragment>
    );
  }

  // Jumped straight to review/summary (e.g. via the dev switcher) without a range.
  if (!range) {
    return (
      <React.Fragment>
        <EmptyDiffScreen range={null} onBack={() => setScreen('onboarding')} />
        <ScreenSwitcher screen={screen} setScreen={setScreen} />
      </React.Fragment>
    );
  }

  // Review/Summary need data: show loading / error / empty guards.
  if (loadError) {
    return (
      <React.Fragment>
        <LoadErrorScreen message={loadError} onRetry={() => load(range)}
          onBack={() => setScreen('onboarding')} />
        <ScreenSwitcher screen={screen} setScreen={setScreen} />
      </React.Fragment>
    );
  }
  if (cards === null) {
    return (
      <React.Fragment>
        <LoadingScreen />
        <ScreenSwitcher screen={screen} setScreen={setScreen} />
      </React.Fragment>
    );
  }
  if (cards.length === 0) {
    return (
      <React.Fragment>
        <EmptyDiffScreen range={range} onBack={() => setScreen('onboarding')} />
        <ScreenSwitcher screen={screen} setScreen={setScreen} />
      </React.Fragment>
    );
  }

  return (
    <React.Fragment>
      {screen === 'review' && card && (
        <FileTree tree={tree} activeId={card.id} open={treeOpen}
          onToggle={() => setTreeOpen((v) => !v)}
          onSelect={(id) => { const i = list.findIndex((c) => c.id === id); goTo(i, i > index ? 1 : -1); }} />
      )}
      {screen === 'review' && card && (
        <ReviewScreen
          card={card} index={index} total={list.length} dir={dir}
          base={range ? range.base : 'base'} target={range ? range.target : 'target'} unresolved={unresolved}
          onOpenSummary={() => setScreen('summary')}
          spineItems={spineItems} onSelect={(id) => { const i = list.findIndex((c) => c.id === id); goTo(i, i > index ? 1 : -1); }}
          verdict={verdicts[card.id]} flagged={hasUnresolved(card.id)}
          hasPrev={index > 0} hasNext={index < list.length - 1}
          onPass={pass} onPrev={prev} onNext={next}
          threads={cardThreads}
          onOpenLine={openLine} onResolve={resolveThread} onSend={sendThread}
        />
      )}
      {screen === 'summary' && (
        <SummaryScreen cards={list} verdicts={verdicts} threads={threads}
          onRestart={() => { setIndex(0); setDir(1); setScreen('review'); }} />
      )}

      <ScreenSwitcher screen={screen} setScreen={setScreen} />
    </React.Fragment>
  );
}

function CenterPane({ children }) {
  return (
    <div style={{ position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column',
      alignItems: 'center', justifyContent: 'center', gap: 16,
      background: 'var(--bg-base)', padding: 24, textAlign: 'center' }}>
      {children}
    </div>
  );
}

function LoadingScreen() {
  // The loading mark (steady dot + breathing halo). Shown while the engine
  // reads the diff and extracts changed symbols.
  return <LoupeLoader full label="Reading the diff…" />;
}

function EmptyDiffScreen({ range, onBack }) {
  return (
    <CenterPane>
      <div style={{ font: 'var(--weight-semibold) var(--text-xl)/1.1 var(--font-ui)',
        color: 'var(--text-primary)', letterSpacing: 'var(--tracking-snug)' }}>
        Nothing to review
      </div>
      <div style={{ font: 'var(--text-base)/1.5 var(--font-ui)', color: 'var(--text-secondary)', maxWidth: 420 }}>
        {range ? `${range.base} and ${range.target} have no differences.` : 'No differences found.'}
      </div>
      <button onClick={onBack} style={pillButtonStyle}>Pick another range</button>
    </CenterPane>
  );
}

function LoadErrorScreen({ message, onRetry, onBack }) {
  return (
    <CenterPane>
      <div style={{ font: 'var(--weight-semibold) var(--text-xl)/1.1 var(--font-ui)',
        color: 'var(--flag)', letterSpacing: 'var(--tracking-snug)' }}>
        Couldn’t load the review
      </div>
      <div style={{ font: 'var(--code-base)/1.5 var(--font-mono)', color: 'var(--text-secondary)',
        maxWidth: 520, background: 'var(--surface-inset)', border: '1px solid var(--border-default)',
        borderRadius: 'var(--radius-md)', padding: '12px 14px', textAlign: 'left',
        wordBreak: 'break-word' }}>
        {message}
      </div>
      <div style={{ display: 'flex', gap: 10 }}>
        <button onClick={onRetry} style={pillButtonStyle}>Retry</button>
        <button onClick={onBack} style={{ ...pillButtonStyle, background: 'transparent',
          color: 'var(--text-tertiary)', borderColor: 'var(--border-default)' }}>Back</button>
      </div>
    </CenterPane>
  );
}

const pillButtonStyle = {
  height: 34, padding: '0 16px', borderRadius: 'var(--radius-pill)', cursor: 'pointer',
  background: 'var(--accent-dim)', border: '1px solid var(--accent-line)', color: 'var(--accent)',
  font: 'var(--weight-medium) var(--text-sm)/1 var(--font-ui)',
};

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
