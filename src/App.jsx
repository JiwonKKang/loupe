/* Loupe UI kit — app shell: screen state, verdicts, threads, keyboard nav.
   Flow: boot → load_token. With a saved token we land on `pickProject` (the
   review shell with the ProjectMenu auto-opened); without one we run `onboarding`
   (token only). Choosing a project from the menu fires analyze_review and moves
   to `review`. Cards + clusters come from the Rust engine in one shot via
   invoke('analyze_review', …): pickProject → loading screen → cluster review.
   There is no flat intermediate stage — the loading screen holds until the
   analysis lands, then the two-tier cluster view renders directly.
   The token is set once (saved on this Mac) and reused for every project;
   repoPath/base/target are chosen per-project from the top-left menu. Everything
   else (verdicts/threads/spineItems/unresolved) is derived on the front-end. */

import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import Onboarding from './screens/Onboarding';
import Settings from './screens/Settings';
import ReviewScreen from './screens/ReviewScreen';
import SummaryScreen from './screens/SummaryScreen';
import ProjectMenu from './components/ProjectMenu';
import { AnalyzeScreen } from './components/AnalyzeScreen';
import { FileTree } from './components/FileTree';
import { buildTree } from './data/fixtures';
// Note: syntax color is a front-end concern — ReviewScreen imports highlightGo
// from ./data/fixtures directly. App.jsx only wires data + screen state.

// The Unclustered bucket id (engine §3.1) and its display label/kind.
const UNCLUSTERED = '__unclustered';

// Live analysis-progress state for the AnalyzeScreen loader, reduced from the engine's
// `analyze://progress` events. `reviewed` maps a cluster id → its real AI title once its
// per-cluster review lands (the rail flips from provisional/spinner to title + check).
const INITIAL_PROGRESS = { phase: 'static', files: 0, clusters: [], reviewed: {} };

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
  const [screen, setScreen] = React.useState('onboarding'); // onboarding | pickProject | review | summary | settings
  // Where to return from Settings (so the gear can be reached from any screen).
  const [prevScreen, setPrevScreen] = React.useState('pickProject');

  // The token is set once (onboarding / settings) and persisted on this Mac.
  const [token, setToken] = React.useState('');
  // The current project + range, chosen from the top-left ProjectMenu (per-project).
  const [repoPath, setRepoPath] = React.useState('');
  const [base, setBase] = React.useState('');
  const [target, setTarget] = React.useState('');

  const [cards, setCards] = React.useState(null);    // null = loading, [] = empty diff
  const [loadError, setLoadError] = React.useState(null);

  // ⑧ — cluster two-tier overlay (filled by analyze_review).
  // analysisState: 'idle' | 'clustering' (AI running) | 'done' | 'fallback'.
  const [clusters, setClusters] = React.useState([]);
  const [clusterOrder, setClusterOrder] = React.useState([]);
  const [orderedCardIds, setOrderedCardIds] = React.useState([]);
  const [unclustered, setUnclustered] = React.useState([]);
  const [analysisState, setAnalysisState] = React.useState('idle');
  // Live loader progress (AnalyzeScreen), driven by `analyze://progress` events.
  const [progress, setProgress] = React.useState(INITIAL_PROGRESS);

  const [index, setIndex] = React.useState(0);
  const [dir, setDir] = React.useState(1);
  const [verdicts, setVerdicts] = React.useState({});
  const [threads, setThreads] = React.useState([]);
  const [treeOpen, setTreeOpen] = React.useState(false);
  const tid = React.useRef(1);
  // Guards a stale analyze_review response from clobbering a newer project's state.
  const analyzeSeq = React.useRef(0);

  // Boot: read the saved model token. With one we go straight to project-picking
  // (the review shell with the menu open); without one we run onboarding. A read
  // failure is non-fatal — fall back to onboarding so the user can (re)connect.
  React.useEffect(() => {
    let alive = true;
    invoke('load_token')
      .then((t) => {
        if (!alive) return;
        if (t) { setToken(t); setScreen('pickProject'); }
        else { setScreen('onboarding'); }
      })
      .catch(() => { if (alive) setScreen('onboarding'); });
    return () => { alive = false; };
  }, []);

  // Run the analysis for a project/range. Single-phase: analyze_review returns the
  // cards AND the cluster two-tier (flow order + spine groups) together. While it
  // runs the loading screen holds; when it lands the cluster review renders
  // directly — there is NO flat intermediate view. (A cache hit returns
  // immediately → straight to clusters; a miss can take minutes, during which the
  // loading screen's staged labels run.) `r` = { repoPath, base, target }; the
  // token comes from app state (set once in onboarding/settings).
  const load = React.useCallback((r) => {
    if (!r || !r.repoPath || !r.base || !r.target) return;
    setCards(null);
    setLoadError(null);
    setIndex(0);
    setVerdicts({});
    setThreads([]);
    // Reset the cluster overlay for the new project.
    setClusters([]); setClusterOrder([]); setOrderedCardIds([]); setUnclustered([]);
    setAnalysisState('clustering');
    setProgress(INITIAL_PROGRESS); // fresh loader for this project
    const seq = ++analyzeSeq.current;

    // analyze_review needs the model token. It returns cards + clusters in one shot.
    // A failure is fatal here (no flat fallback to fall back to) → surface the error.
    invoke('analyze_review', {
      repoPath: r.repoPath, base: r.base, target: r.target, token,
    })
      .then((data) => {
        if (seq !== analyzeSeq.current) return;
        applyAnalysis(data, seq);
      })
      .catch((err) => { if (seq === analyzeSeq.current) { setLoadError(String(err)); setCards([]); } });
  // applyAnalysis is stable (defined below via useCallback); token is read fresh.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [token]);

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

  // Subscribe once to the engine's pipeline progress events and reduce them into `progress`
  // for the AnalyzeScreen loader. Events are cosmetic — a missed one only affects the loader,
  // never the result (which arrives via the analyze_review promise).
  React.useEffect(() => {
    let alive = true;
    let unlisten = null;
    listen('analyze://progress', (ev) => {
      const p = ev && ev.payload;
      if (!p || !p.kind) return;
      setProgress((prev) => {
        switch (p.kind) {
          case 'static': return { ...prev, phase: 'static', files: p.files != null ? p.files : prev.files };
          case 'clustering': return { ...prev, phase: 'clustering' };
          case 'clusters': return { ...prev, phase: 'review', clusters: p.clusters || [] };
          case 'reviewed': return { ...prev, reviewed: { ...prev.reviewed, [p.id]: p.chapter || '' } };
          case 'final': return { ...prev, phase: 'final' };
          default: return prev;
        }
      });
    }).then((un) => { if (alive) unlisten = un; else un(); });
    return () => { alive = false; if (unlisten) unlisten(); };
  }, []);

  // Choose a project from the ProjectMenu → store the range + run analysis + show
  // the review. Same trigger the menu's "Open / Re-run review" hands up.
  const changeProject = React.useCallback(({ repoPath: rp, base: b, target: t }) => {
    setRepoPath(rp);
    setBase(b);
    setTarget(t);
    setScreen('review');
    load({ repoPath: rp, base: b, target: t });
  }, [load]);

  // Onboarding finished → persist the token, then go pick a project. A save
  // failure keeps the user on onboarding with the error surfaced.
  const [onboardError, setOnboardError] = React.useState(null);
  const finishOnboarding = React.useCallback((t) => {
    invoke('save_token', { token: t })
      .then(() => { setOnboardError(null); setToken(t); setScreen('pickProject'); })
      .catch((err) => setOnboardError(String(err)));
  }, []);

  // Open Settings, remembering where to return.
  const openSettings = React.useCallback(() => {
    setPrevScreen((s) => (s === 'settings' ? 'pickProject' : screen));
    setScreen('settings');
  }, [screen]);

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
    // No fabricated AI opener — a fresh thread is just the composer (empty
    // messages). The first real AI text arrives only after the user asks.
    setThreads((p) => [...p, {
      id, cardId: card.id, side: side || 'old', lineN, symbol: card.symbol, open: true, resolved: false,
      messages: [], pending: false,
    }]);
  };

  // Build the `context` string the backend sees: which symbol/file, a diff
  // excerpt (windowed around the asked row for big cards), and which line the
  // question targets. `t` is the thread (for lineN/side); `c` is its card.
  const buildThreadContext = React.useCallback((c, t) => {
    if (!c) return '';
    const lines = c.lines || [];
    // Render each diff line as `+`/`-`/` ` + code (unified-diff style).
    const sign = (ln) => (ln.t === 'add' ? '+' : ln.t === 'del' ? '-' : ' ');
    const total = lines.length;
    // Windowing: for big cards, only send ±WIN lines around the asked row so
    // the prompt stays bounded. `lineN` is a ROW index into the diff list.
    const WIN = 60;
    let from = 0, to = total, windowed = false;
    if (total > WIN * 2 + 1) {
      const anchor = Math.max(0, Math.min(total - 1, t && t.lineN != null ? t.lineN : 0));
      from = Math.max(0, anchor - WIN);
      to = Math.min(total, anchor + WIN + 1);
      windowed = from > 0 || to < total;
    }
    const body = lines.slice(from, to).map((ln) => sign(ln) + ln.c).join('\n');
    const header = `심볼: ${c.symbol || '(이름 없음)'}\n파일: ${c.path || '(경로 없음)'}`;
    const span = windowed
      ? `\n(diff 발췌: 전체 ${total}줄 중 ${from + 1}–${to}줄 발췌)`
      : `\n(diff 전체 ${total}줄)`;
    const side = t && t.side ? t.side : 'old';
    const target = `\n\n질문 대상: line ${t && t.lineN != null ? t.lineN : '?'} (${side})`;
    return `${header}${span}\n\n${body}${target}`;
  }, []);

  const sendThread = (id, text, kind) => {
    // Append the user's message (kept for both questions and commands).
    setThreads((p) => p.map((t) => t.id === id
      ? { ...t, messages: [...t.messages, { author: 'you', text, time: 'now', kind: kind || 'question' }] } : t));
    // Commands are change requests captured for the summary — no AI reply.
    if (kind === 'command') return;

    // Snapshot the thread + its card BEFORE the async call so context/history
    // reflect the state at ask-time. `history` is every message up to (and
    // excluding) this question.
    const t = threads.find((x) => x.id === id);
    if (!t) return;
    const c = (cards || []).find((x) => x.id === t.cardId);
    const context = buildThreadContext(c, t);
    const history = t.messages.map((m) => ({ author: m.author, text: m.text }));

    // Mark the thread as thinking (spinner row in Thread.jsx).
    setThreads((p) => p.map((x) => x.id === id ? { ...x, pending: true } : x));

    invoke('ask_thread', { token, repoPath, context, question: text, history })
      .then((answer) => {
        setThreads((p) => p.map((x) => x.id === id
          ? { ...x, pending: false, messages: [...x.messages, { author: 'ai', text: String(answer), time: 'now' }] }
          : x));
      })
      .catch((err) => {
        setThreads((p) => p.map((x) => x.id === id
          ? { ...x, pending: false, messages: [...x.messages, { author: 'ai', text: '⚠️ 답변을 못 받았어요: ' + String(err), time: 'now' }] }
          : x));
      });
  };
  const resolveThread = (id) => setThreads((p) => p.map((t) => t.id === id ? { ...t, resolved: !t.resolved, open: false } : t));

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

  // Settings stands on its own (reachable from any screen via the gear).
  if (screen === 'settings') {
    return (
      <React.Fragment>
        <Settings connected={token.length > 0}
          onBack={() => setScreen(prevScreen)}
          onSaved={(t) => setToken(t)}
          onCleared={() => setToken('')} />
        <ScreenSwitcher screen={screen} setScreen={setScreen} onSettings={openSettings} />
      </React.Fragment>
    );
  }

  // Onboarding stands on its own (token only — no cards needed).
  if (screen === 'onboarding') {
    return (
      <React.Fragment>
        <Onboarding onFinish={finishOnboarding} />
        {onboardError && <OnboardErrorToast message={onboardError} />}
        <ScreenSwitcher screen={screen} setScreen={setScreen} onSettings={openSettings} />
      </React.Fragment>
    );
  }

  // Project-picking shell: the review-screen layout with no card — the ProjectMenu
  // auto-opens (defaultOpen) at top-left and a center hint invites the user to pick
  // a project. Choosing one runs analyze_review and moves to `review`.
  if (screen === 'pickProject') {
    return (
      <React.Fragment>
        <div style={{ position: 'absolute', inset: 0, background: 'var(--bg-base)', overflow: 'hidden' }}>
          <ProjectMenu
            project={repoPath} base={base} target={target}
            branches={undefined} recents={undefined}
            onChangeProject={changeProject} defaultOpen pinned />
          <div style={{ position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column',
            alignItems: 'center', justifyContent: 'center', gap: 10, padding: 24, textAlign: 'center',
            pointerEvents: 'none' }}>
            <div style={{ font: 'var(--weight-semibold) var(--text-lg)/1.2 var(--font-ui)',
              color: 'var(--text-secondary)', letterSpacing: 'var(--tracking-snug)' }}>
              프로젝트를 선택해 리뷰를 시작하세요
            </div>
            <div style={{ font: 'var(--text-sm)/1.5 var(--font-ui)', color: 'var(--text-faint)' }}>
              왼쪽 위 메뉴에서 폴더와 비교할 브랜치를 고르면 분석이 시작됩니다.
            </div>
          </div>
        </div>
        <ScreenSwitcher screen={screen} setScreen={setScreen} onSettings={openSettings} />
      </React.Fragment>
    );
  }

  // Review/Summary need a chosen project. If we somehow got here without one
  // (e.g. via the dev switcher), bounce back to project-picking.
  if (!repoPath) {
    return (
      <React.Fragment>
        <EmptyDiffScreen range={null} onBack={() => setScreen('pickProject')} />
        <ScreenSwitcher screen={screen} setScreen={setScreen} onSettings={openSettings} />
      </React.Fragment>
    );
  }

  // Review/Summary need data: show loading / error / empty guards.
  if (loadError) {
    return (
      <React.Fragment>
        <LoadErrorScreen message={loadError} onRetry={() => load({ repoPath, base, target })}
          onBack={() => setScreen('pickProject')} />
        <ScreenSwitcher screen={screen} setScreen={setScreen} onSettings={openSettings} />
      </React.Fragment>
    );
  }
  if (cards === null) {
    return (
      <React.Fragment>
        <AnalyzeScreen progress={progress} />
        <ScreenSwitcher screen={screen} setScreen={setScreen} onSettings={openSettings} />
      </React.Fragment>
    );
  }
  if (cards.length === 0) {
    return (
      <React.Fragment>
        <EmptyDiffScreen range={{ base, target }} onBack={() => setScreen('pickProject')} />
        <ScreenSwitcher screen={screen} setScreen={setScreen} onSettings={openSettings} />
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
          project={repoPath} base={base} target={target} onChangeProject={changeProject}
          unresolved={unresolved}
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

      <ScreenSwitcher screen={screen} setScreen={setScreen} onSettings={openSettings} />
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
      <button onClick={onBack} style={pillButtonStyle}>Pick another project</button>
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

// A small inline toast for an onboarding save_token failure (the only place the
// token persists from onboarding).
function OnboardErrorToast({ message }) {
  return (
    <div style={{ position: 'fixed', bottom: 18, left: '50%', transform: 'translateX(-50%)',
      zIndex: 70, maxWidth: 520, padding: '10px 14px', borderRadius: 'var(--radius-md)',
      background: 'var(--surface-inset)', border: '1px solid var(--flag)',
      color: 'var(--flag)', font: 'var(--text-sm)/1.4 var(--font-ui)' }}>
      {message}
    </div>
  );
}

const pillButtonStyle = {
  height: 34, padding: '0 16px', borderRadius: 'var(--radius-pill)', cursor: 'pointer',
  background: 'var(--accent-dim)', border: '1px solid var(--accent-line)', color: 'var(--accent)',
  font: 'var(--weight-medium) var(--text-sm)/1 var(--font-ui)',
};

function ScreenSwitcher({ screen, setScreen, onSettings }) {
  const [hover, setHover] = React.useState(false);
  // 'Pick' (pickProject) intentionally omitted — the top-left project menu already covers it.
  const tabs = [['onboarding', 'Onboarding'], ['review', 'Review'], ['summary', 'Summary']];
  const gear = 'M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z';
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
      {/* Settings gear — always reachable; sits at the end of the switcher. */}
      {hover && onSettings && (
        <React.Fragment>
          <span style={{ width: 1, height: 12, background: 'var(--border-default)', margin: '0 2px' }} />
          <button onClick={onSettings} aria-label="Settings" title="Settings" style={{
            display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
            padding: '4px', borderRadius: 'var(--radius-pill)', cursor: 'pointer', border: 'none',
            background: screen === 'settings' ? 'var(--surface-overlay)' : 'transparent',
            color: screen === 'settings' ? 'var(--text-primary)' : 'var(--text-tertiary)',
            transition: 'var(--t-hover)' }}>
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor"
              strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d={gear} /></svg>
          </button>
        </React.Fragment>
      )}
    </div>
  );
}
