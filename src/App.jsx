/* Loupe UI kit — app shell: screen state, verdicts, threads, keyboard nav.
   Cards + clusters come from the Rust engine in one shot via
   invoke('analyze_review', …): Onboarding → loading screen → cluster review.
   There is no flat intermediate stage — the loading screen holds until the
   analysis lands, then the two-tier cluster view renders directly.
   Onboarding collects repoPath/base/target/token; everything else
   (verdicts/threads/spineItems/unresolved) is derived on the front-end. */

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

// The Unclustered bucket id (engine §3.1) and its display label/kind.
const UNCLUSTERED = '__unclustered';

/**
 * Flatten the flat `cards` into the cluster flow order (`orderedCardIds`), trailing the
 * Unclustered bucket, then any cards the layout did not place (defensive — "every change is
 * shown", §3.1). When the analysis has not arrived yet (`orderedCardIds` empty) the source
 * card order is kept — but in practice the review screen only renders once analyze_review
 * has returned both cards and the cluster overlay together.
 */
function flattenByOrder(cards, orderedCardIds) {
  if (!cards || cards.length === 0) return [];
  if (!orderedCardIds || orderedCardIds.length === 0) return cards;
  const byId = new Map(cards.map((c) => [c.id, c]));
  const out = [];
  const seen = new Set();
  orderedCardIds.forEach((id) => {
    const c = byId.get(id);
    if (c && !seen.has(id)) { out.push(c); seen.add(id); }
  });
  // Any card not named by the layout still appears (never drop a change).
  cards.forEach((c) => { if (!seen.has(c.id)) out.push(c); });
  return out;
}

export default function App() {
  const [screen, setScreen] = React.useState('onboarding'); // onboarding | review | summary
  const [range, setRange] = React.useState(null);   // { repoPath, base, target, token }
  const [cards, setCards] = React.useState(null);    // null = loading, [] = empty diff
  const [loadError, setLoadError] = React.useState(null);

  // ⑧ — cluster two-tier overlay (filled by analyze_review).
  // analysisState: 'idle' | 'clustering' (AI running) | 'done' | 'fallback'.
  const [clusters, setClusters] = React.useState([]);
  const [clusterOrder, setClusterOrder] = React.useState([]);
  const [orderedCardIds, setOrderedCardIds] = React.useState([]);
  const [unclustered, setUnclustered] = React.useState([]);
  const [analysisState, setAnalysisState] = React.useState('idle');

  const [index, setIndex] = React.useState(0);
  const [dir, setDir] = React.useState(1);
  const [verdicts, setVerdicts] = React.useState({});
  const [threads, setThreads] = React.useState([]);
  const [treeOpen, setTreeOpen] = React.useState(false);
  const tid = React.useRef(1);
  // Guards a stale analyze_review response from clobbering a newer range's state.
  const analyzeSeq = React.useRef(0);

  // Load the review whenever a range is chosen (and on retry). Single-phase:
  // analyze_review returns the cards AND the cluster two-tier (flow order + spine
  // groups) together. While it runs the loading screen holds; when it lands the
  // cluster review renders directly — there is NO flat intermediate view.
  // (A cache hit returns immediately → straight to clusters; a miss can take
  // minutes, during which the loading screen's staged labels run.)
  const load = React.useCallback((r) => {
    if (!r) return;
    setCards(null);
    setLoadError(null);
    setIndex(0);
    setVerdicts({});
    setThreads([]);
    // Reset the cluster overlay for the new range.
    setClusters([]); setClusterOrder([]); setOrderedCardIds([]); setUnclustered([]);
    setAnalysisState('clustering');
    const seq = ++analyzeSeq.current;

    // analyze_review needs the model token. It returns cards + clusters in one shot.
    // A failure is fatal here (no flat fallback to fall back to) → surface the error.
    invoke('analyze_review', {
      repoPath: r.repoPath, base: r.base, target: r.target, token: r.token,
    })
      .then((data) => {
        if (seq !== analyzeSeq.current) return;
        applyAnalysis(data, seq);
      })
      .catch((err) => { if (seq === analyzeSeq.current) { setLoadError(String(err)); setCards([]); } });
  // applyAnalysis is stable (defined below via useCallback).
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Adopt an analyze_review payload: the cards and the cluster overlay together.
  const applyAnalysis = React.useCallback((data, seq) => {
    if (seq !== analyzeSeq.current) return;
    setCards(data.cards || []);
    setClusters(data.clusters || []);
    setClusterOrder(data.clusterOrder || []);
    setOrderedCardIds(data.orderedCardIds || []);
    setUnclustered(data.unclustered || []);
    setAnalysisState(data.analysis === 'fallback' ? 'fallback' : 'done');
    setLoadError(null);
  }, []);

  React.useEffect(() => { load(range); }, [range, load]);

  const startReview = (r) => {
    setRange({ repoPath: r.repoPath, base: r.base || 'main', target: r.target, token: r.token });
    setScreen('review');
  };

  // Derived (safe even when cards is null — guarded reads).
  // Flat cards re-ordered into cluster flow order once the analysis arrives (§3.1: every
  // card still appears, Unclustered trailing). Index/keyboard nav are unchanged — they just
  // walk this re-ordered list.
  const list = React.useMemo(
    () => flattenByOrder(cards, orderedCardIds),
    [cards, orderedCardIds],
  );
  const card = list[index];

  // cluster id -> its title/kind, for spine grouping + the card's cluster band.
  const clusterById = React.useMemo(() => {
    const m = new Map();
    clusters.forEach((c) => m.set(c.id, c));
    return m;
  }, [clusters]);
  const unclusteredSet = React.useMemo(() => new Set(unclustered), [unclustered]);

  // The cluster a card belongs to (id/title/kind/summary), or the Unclustered bucket.
  // The review screen only renders after analyze_review lands, so an overlay is always
  // present here; the null path is a defensive guard for a fallback that produced no
  // clusters at all (keeps the file view rather than labelling everything Unclustered).
  const hasOverlay = clusters.length > 0 || unclustered.length > 0;
  const clusterOf = React.useCallback((c) => {
    if (!c) return null;
    if (c.clusterId && clusterById.has(c.clusterId)) {
      const cl = clusterById.get(c.clusterId);
      return { id: cl.id, title: cl.title, kind: cl.kind, summary: cl.summary };
    }
    // Only bucket a card as "기타" once a real overlay exists (engine §3.1); a bare
    // fallback with no clusters keeps the file view rather than labelling everything.
    // Note: infra/config files now get real `infra` clusters (engine file-seeds), so this
    // residual bucket holds only genuinely-unrelated changes — phrased as "기타 변경"
    // (other changes), NOT a classification *failure*.
    if (unclusteredSet.has(c.id) || (hasOverlay && (analysisState === 'done' || analysisState === 'fallback'))) {
      return { id: UNCLUSTERED, title: '기타 변경', kind: 'unclustered',
        summary: '특정 흐름·주제에 묶이지 않는 개별 변경입니다.' };
    }
    return null; // no overlay → no band (defensive).
  }, [clusterById, unclusteredSet, analysisState, hasOverlay]);

  // Position of a card within its cluster ({ pos, of }), for "n / m in this cluster".
  const clusterPosition = React.useCallback((c) => {
    const cl = clusterOf(c);
    if (!cl) return null;
    const members = list.filter((x) => {
      const xc = clusterOf(x);
      return xc && xc.id === cl.id;
    });
    const pos = members.findIndex((x) => x.id === c.id);
    if (pos < 0) return null;
    return { pos: pos + 1, of: members.length };
  }, [list, clusterOf]);

  const threadCount = (cid) => threads.filter((t) => t.cardId === cid).length;
  // A card with unresolved threads IS the "flag" (Needs attention).
  const hasUnresolved = (cid) => threads.some((t) => t.cardId === cid && !t.resolved);
  const spineItems = list.map((c) => {
    const cl = clusterOf(c);
    // Group key: cluster id when analysed, else the Stage-1 chapter (file) so the spine is
    // still grouped before the AI overlay arrives.
    const clusterId = cl ? cl.id : c.chapter;
    return {
      id: c.id, symbol: c.symbol, label: c.symbol, chapter: c.chapter,
      clusterId, clusterTitle: cl ? cl.title : c.chapter,
      clusterKind: cl ? cl.kind : null, clusterSummary: cl ? cl.summary : '',
      file: c.path.split('/').pop(), threads: threadCount(c.id),
      status: hasUnresolved(c.id) ? 'flag' : (verdicts[c.id] === 'pass' ? 'pass' : 'pending'),
    };
  });
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
          cluster={clusterOf(card)} clusterIndex={clusterPosition(card)}
          analysisState={analysisState}
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

// The dataflow-clustering stage sequence shown on the full-screen loader while
// analyze_review runs (cards + clusters arrive together at the end). The labels
// cycle on a timer and hold on the last one; a cache miss can take minutes, so
// the staged sequence keeps the wait legible.
const LOADING_STAGES = [
  'Reading the diff…',
  'Tracing data flow…',
  'Clustering related changes…',
  'Grouping into chapters…',
  'Building the review queue…',
];

function LoadingScreen() {
  // The loading mark (steady dot + breathing halo) with the staged dataflow
  // labels. Holds until analyze_review returns the cards + cluster overlay.
  return <LoupeLoader full stages={LOADING_STAGES} />;
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
