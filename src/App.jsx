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
import { onOpenUrl, getCurrent as getCurrentDeepLink } from '@tauri-apps/plugin-deep-link';
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
  // Globally-unique thread id. A bare counter collided across sessions — it resets
  // to 1 on every load while threads restored from disk keep their old t1.. ids, so
  // a freshly-created thread could reuse an existing id. Then `threads.map(x.id===id)`
  // wrote the SAME AI answer onto every thread sharing that id (and the context was
  // built from the first match). The random suffix guarantees uniqueness regardless
  // of the counter's state.
  const genThreadId = () => 't' + (tid.current++) + Math.random().toString(36).slice(2, 6);
  // #8/#9 — unread AI answers. `unreadThreads` holds the ids of threads whose AI
  // reply has landed but the user has NOT yet expanded that thread to read it.
  //   • marked unread  = sendThread resolves while the thread is not visibly open
  //   • cleared (read) = openLine makes that thread `open` (NOT mere card nav).
  // `flashCluster` is the cluster id that should briefly flash on the collapsed
  // spine when a fresh answer arrives off-screen (transient, auto-cleared ~2.4s).
  const [unreadThreads, setUnreadThreads] = React.useState(() => new Set());
  const [flashCluster, setFlashCluster] = React.useState(null);
  const flashTimer = React.useRef(null);
  // Guards a stale analyze_review response from clobbering a newer project's state.
  const analyzeSeq = React.useRef(0);

  // Thread persistence (load_threads/save_threads) is keyed by (repoPath, base, target).
  // `keyOf` builds that composite key; `loadedKeyRef` records WHICH key's threads have
  // finished loading. The debounced save only fires when the loaded key matches the
  // current project — so an empty/in-flight `threads` can never overwrite saved threads
  // for a project whose load hasn't completed yet (race guard).
  const keyOf = (rp, b, t) => rp + '' + b + '' + t;
  const loadedKeyRef = React.useRef(null);
  // Holds the pending debounced save timer so it can be re-armed / cleared.
  const saveTimer = React.useRef(null);

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
    // Block the debounced save until this project's threads have actually loaded —
    // otherwise the empty reset above could overwrite saved threads. Cleared here,
    // re-set in the load_threads success/failure path below.
    loadedKeyRef.current = null;
    // Fresh project → drop any pending unread/flash from the previous one.
    setUnreadThreads(new Set());
    setFlashCluster(null);
    if (flashTimer.current) { clearTimeout(flashTimer.current); flashTimer.current = null; }
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
        // Restore saved threads for this exact (repoPath, base, target). This is
        // independent of analyze_review's cache (hit or miss) — it reads the separate
        // loupe_dir/threads.json store. Sanitize transient UI fields: pending (AI
        // thinking) is never persisted-true, and threads always reopen collapsed.
        invoke('load_threads', { repoPath: r.repoPath, base: r.base, target: r.target })
          .then((json) => {
            if (seq !== analyzeSeq.current) return; // a newer project started — drop this
            let arr;
            try { arr = JSON.parse(json || '[]'); } catch { arr = []; }
            if (!Array.isArray(arr)) arr = [];
            // Re-id every restored thread so any duplicate/colliding ids saved by an
            // older build are repaired on load (distinct content is preserved).
            setThreads(arr.map((t) => ({ ...t, id: genThreadId(), pending: false, open: false })));
            // Mark this key as loaded so the debounced save may now run for it.
            loadedKeyRef.current = keyOf(r.repoPath, r.base, r.target);
          })
          .catch(() => {
            // A read failure is non-fatal: keep the empty (reset) thread list but still
            // mark the key loaded so new threads on this project can be saved.
            if (seq !== analyzeSeq.current) return;
            loadedKeyRef.current = keyOf(r.repoPath, r.base, r.target);
          });
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

  // Debounced thread persistence. Whenever `threads` changes we schedule a save 600ms
  // later (re-armed on each change, so a burst of edits writes once). Race guard: the
  // save only runs when `loadedKeyRef` matches the CURRENT project key — i.e. this
  // project's threads have finished loading. Before that (fresh project, threads reset
  // to []), loadedKeyRef is null and the save is skipped, so the empty list can never
  // overwrite saved threads. pending is stripped (never persist "AI thinking"); every
  // other field (model/resolved/messages/lineN/cardId/side/symbol/open/…) is preserved.
  React.useEffect(() => {
    if (saveTimer.current) clearTimeout(saveTimer.current);
    const key = keyOf(repoPath, base, target);
    saveTimer.current = setTimeout(() => {
      saveTimer.current = null;
      if (loadedKeyRef.current !== key) return; // not loaded for this project yet — skip
      invoke('save_threads', {
        repoPath, base, target,
        threads: JSON.stringify(threads.map((t) => ({ ...t, pending: false }))),
      }).catch(() => {}); // a save failure is non-fatal (in-memory threads stay intact)
    }, 600);
    return () => { if (saveTimer.current) { clearTimeout(saveTimer.current); saveTimer.current = null; } };
  }, [threads, repoPath, base, target]);

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

  // Clear the pending flash timeout if the component ever unmounts mid-flash.
  React.useEffect(() => () => { if (flashTimer.current) clearTimeout(flashTimer.current); }, []);

  // Choose a project from the ProjectMenu → store the range + run analysis + show
  // the review. Same trigger the menu's "Open / Re-run review" hands up.
  const changeProject = React.useCallback(({ repoPath: rp, base: b, target: t }) => {
    setRepoPath(rp);
    setBase(b);
    setTarget(t);
    setScreen('review');
    load({ repoPath: rp, base: b, target: t });
  }, [load]);

  // ---- `loupe://review?repoPath=&base=&target=` deep links (the `loupe` CLI) ----
  // The CLI opens this URL; the OS routes it here (launching or focusing the app).
  // We stash the parsed range and apply it once a token exists.
  const [pendingCli, setPendingCli] = React.useState(null);
  const handleDeepLink = React.useCallback((urls) => {
    const raw = Array.isArray(urls) ? urls.find(Boolean) : urls;
    if (!raw) return;
    try {
      const u = new URL(raw);
      const action = (u.host || u.pathname.replace(/\//g, '')) || '';
      if (action !== 'review') return;
      const repoPath = u.searchParams.get('repoPath');
      const base = u.searchParams.get('base');
      const target = u.searchParams.get('target');
      if (repoPath && base && target) setPendingCli({ repoPath, base, target });
    } catch { /* malformed URL — ignore */ }
  }, []);
  React.useEffect(() => {
    let un;
    onOpenUrl((urls) => handleDeepLink(urls)).then((u) => { un = u; }).catch(() => {});
    // cold start: the app may have been launched BY the url.
    getCurrentDeepLink().then((urls) => { if (urls && urls.length) handleDeepLink(urls); }).catch(() => {});
    return () => { if (un) un(); };
  }, [handleDeepLink]);
  // Apply a pending CLI review once a token is available; without one, send the
  // user to onboarding (the pending review survives until the token is set).
  React.useEffect(() => {
    if (!pendingCli) return;
    if (!token) { setScreen('onboarding'); return; }
    const { repoPath: rp, base: b, target: t } = pendingCli;
    // record in recents (same shape the ProjectMenu uses) so it shows in the menu.
    try {
      const raw = window.localStorage.getItem('loupe.recents');
      const arr = raw ? JSON.parse(raw) : [];
      const next = [rp, ...(Array.isArray(arr) ? arr : []).filter((r) => r !== rp)].slice(0, 4);
      window.localStorage.setItem('loupe.recents', JSON.stringify(next));
    } catch { /* storage unavailable — non-fatal */ }
    setPendingCli(null);
    changeProject({ repoPath: rp, base: b, target: t });
  }, [pendingCli, token, changeProject]);

  // Open the analyzed project in an external editor (IntelliJ / VS Code) at
  // file:line — cmd/ctrl-click on a diff line. `file` is repo-relative; the Rust
  // side joins it onto repoPath and opens the whole project, then navigates.
  // `loupe.editor` (localStorage): 'auto' | 'idea' | 'code'.
  const [editorMsg, setEditorMsg] = React.useState(null);
  const editorMsgTimer = React.useRef(null);
  const openInEditor = React.useCallback((file, line, editorOverride) => {
    if (!repoPath || !file) return;
    let editor = editorOverride || null;
    if (editor) {
      // explicit pick from the header chooser → remember as the new default.
      try { window.localStorage.setItem('loupe.editor', editor); } catch { /* ignore */ }
    } else {
      try { editor = window.localStorage.getItem('loupe.editor') || 'idea'; } catch { editor = 'idea'; }
    }
    const ln = Math.max(1, Math.floor(line) || 1);
    // Silent on success; only surface a toast if the editor couldn't be launched.
    invoke('open_in_editor', { editor, repoPath, file, line: ln })
      .catch((e) => {
        setEditorMsg(String(e));
        if (editorMsgTimer.current) clearTimeout(editorMsgTimer.current);
        editorMsgTimer.current = setTimeout(() => setEditorMsg(null), 6000);
      });
  }, [repoPath]);

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

  // --- GitHub PR approval (summary screen, all-pass) ---------------------------
  // Delegated to the user's `gh` CLI (Loupe makes no GitHub calls of its own). The
  // PR is queried ONLY when the review actually ended all-pass — and approval only
  // ever fires from an explicit click in SummaryScreen (never automatically here).
  const allPass = list.length > 0
    && list.every((c) => verdicts[c.id] === 'pass')
    && threads.every((t) => t.resolved);
  // prStatus.state: 'unknown' | 'loading' | 'open' | 'none' | 'error' (+ PrInfo fields when 'open')
  const [prStatus, setPrStatus] = React.useState({ state: 'unknown' });
  const [approveState, setApproveState] = React.useState('idle'); // idle | approving | approved | error
  React.useEffect(() => {
    setApproveState('idle');
    if (screen !== 'summary' || !allPass || !repoPath || !target) { setPrStatus({ state: 'unknown' }); return; }
    let alive = true;
    setPrStatus({ state: 'loading' });
    invoke('pr_status', { repoPath, target })
      // The PR info is NESTED under `pr` so gh's own uppercase `state` field
      // (OPEN/MERGED/CLOSED) never overwrites our wrapper `state`. Only an OPEN PR is
      // approvable; a merged/closed one shows as 'closed' (no button).
      .then((info) => {
        if (!alive) return;
        if (!info) { setPrStatus({ state: 'none' }); return; }
        setPrStatus({ state: info.state === 'OPEN' ? 'open' : 'closed', pr: info });
      })
      .catch(() => { if (alive) setPrStatus({ state: 'error' }); });
    return () => { alive = false; };
  }, [screen, allPass, repoPath, target]);
  const approvePr = React.useCallback((body) => {
    setApproveState('approving');
    invoke('approve_pr', { repoPath, target, body: body || null })
      .then((info) => { setApproveState('approved'); setPrStatus({ state: 'open', pr: info }); })
      .catch((e) => {
        setApproveState('error');
        setEditorMsg(String(e));
        if (editorMsgTimer.current) clearTimeout(editorMsgTimer.current);
        editorMsgTimer.current = setTimeout(() => setEditorMsg(null), 6000);
      });
  }, [repoPath, target]);

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

  // Sidebar count = total messages across the card's threads (question = 1,
  // +AI reply = 2). An empty (drag-only, no message) thread contributes 0, so
  // it doesn't show a phantom "1". Updates live as replies land.
  const threadCount = (cid) => threads
    .filter((t) => t.cardId === cid)
    .reduce((n, t) => n + (t.messages ? t.messages.length : 0), 0);
  // A card with unresolved threads IS the "flag" (Needs attention).
  const hasUnresolved = (cid) => threads.some((t) => t.cardId === cid && !t.resolved);
  // #8 — a card has an unread answer if any of its threads is in unreadThreads.
  const hasUnread = (cid) => threads.some((t) => t.cardId === cid && unreadThreads.has(t.id));
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
      // #8 spine signals: a card carrying an unread answer (`unread`), whether it
      // has any thread at all (`hasThread` → cluster reads as yellow/needs-eyes),
      // and whether its cluster is mid-flash because an answer landed off-screen.
      unread: hasUnread(c.id),
      // Any thread on the card (a sent message OR a just-created open one) marks the
      // cluster — so a comment shows immediately, even before a reply lands.
      hasThread: threads.some((t) => t.cardId === c.id && ((t.messages && t.messages.length > 0) || t.open)),
      flash: flashCluster != null && clusterId === flashCluster,
    };
  });
  const unresolved = threads.filter((t) => !t.resolved).length;
  // Changed-files tree for the right-hand FileTree sidebar (built from the
  // real cards returned by the engine).
  const tree = React.useMemo(() => buildTree(list), [list]);

  // Navigation back-stack: every time we land on a DIFFERENT card (paging, the
  // spine, or a card-link jump in a thread), remember where we came from so cmd+E
  // can return to it (IntelliJ "Back" style). Capped so it can't grow unbounded.
  const backStack = React.useRef([]);
  const goTo = (i, d) => {
    const clamped = Math.max(0, Math.min(list.length - 1, i));
    if (clamped !== index) {
      backStack.current.push(index);
      if (backStack.current.length > 100) backStack.current.shift();
    }
    setDir(d);
    setIndex(clamped);
  };
  // cmd+E — jump back to the previously-visited card. Pops the history WITHOUT
  // re-recording, so repeated presses keep stepping further back.
  const goBack = () => {
    if (backStack.current.length === 0) return;
    const prevIdx = Math.max(0, Math.min(list.length - 1, backStack.current.pop()));
    setDir(prevIdx > index ? 1 : -1);
    setIndex(prevIdx);
  };

  const advance = () => {
    if (index >= list.length - 1) { setScreen('summary'); return; }
    goTo(index + 1, 1);
  };
  // Pass marks the card 'pass' and advances. We remember the order of passes so a
  // mis-pressed Space can be undone (⌘Z) — pop the last pass, clear its verdict, and
  // jump back to that card. `unpass` clears a specific card's pass without navigating
  // (used by the clickable "Passed" badge when you're already on the card).
  const passHistory = React.useRef([]);
  const pass = () => {
    if (!card) return;
    passHistory.current.push(card.id);
    setVerdicts((p) => ({ ...p, [card.id]: 'pass' }));
    advance();
  };
  const unpass = (id) => {
    const target = id || (card && card.id);
    if (!target) return;
    setVerdicts((p) => { if (p[target] == null) return p; const n = { ...p }; delete n[target]; return n; });
    passHistory.current = passHistory.current.filter((x) => x !== target);
  };
  const undoPass = () => {
    const id = passHistory.current.pop();
    if (!id) return;
    setVerdicts((p) => { const n = { ...p }; delete n[id]; return n; });
    const i = list.findIndex((c) => c.id === id);
    if (i >= 0) goTo(i, i <= index ? -1 : 1);
  };
  const next = () => { if (index < list.length - 1) goTo(index + 1, 1); else setScreen('summary'); };
  const prev = () => { if (index > 0) goTo(index - 1, -1); };

  // The active card id, mirrored into a ref so an in-flight ask_thread that
  // resolves later can tell whether ITS thread is still the one on screen — the
  // render-scoped `card` would be stale by the time the promise settles.
  const activeCardIdRef = React.useRef(null);
  React.useEffect(() => { activeCardIdRef.current = card ? card.id : null; }, [card]);

  // Leaving a card auto-closes its threads: drop drag-only empties (no message)
  // and collapse any open messaged thread back to its badge — you shouldn't
  // return to a card with a thread still hanging open.
  React.useEffect(() => {
    setThreads((p) => {
      const next = p
        .filter((t) => t.messages && t.messages.length > 0)
        .map((t) => (t.open ? { ...t, open: false } : t));
      if (next.length === p.length && next.every((t, i) => t === p[i])) return p; // unchanged → keep ref
      return next;
    });
  }, [card ? card.id : null]);

  // Mark a thread as read (#8): remove it from unreadThreads. Identity-stable
  // (no-op when the id wasn't unread) so it never forces a needless re-render.
  const clearUnread = (id) => setUnreadThreads((prev) => {
    if (!prev.has(id)) return prev;
    const next = new Set(prev);
    next.delete(id);
    return next;
  });

  // Mark a thread unread (#8) and flash its cluster on the collapsed spine.
  // The flash is transient: the cluster id is held in `flashCluster` for ~2.4s,
  // then cleared. Re-arming the timer on each new answer keeps the latest flash
  // alive for its full window rather than being cut short by an earlier one.
  const markUnread = (id, cardId) => {
    setUnreadThreads((prev) => {
      if (prev.has(id)) return prev;
      const next = new Set(prev);
      next.add(id);
      return next;
    });
    const c = (cards || []).find((x) => x.id === cardId);
    const cl = clusterOf(c);
    // Group key matches spineItems' (cluster id when analysed, else the chapter/file).
    const flashKey = cl ? cl.id : (c ? c.chapter : null);
    if (flashKey == null) return;
    setFlashCluster(flashKey);
    if (flashTimer.current) clearTimeout(flashTimer.current);
    flashTimer.current = setTimeout(() => { flashTimer.current = null; setFlashCluster(null); }, 2400);
  };

  const openLine = (side, lineN, anchorRange) => {
    if (!card) return;
    // Threads are keyed by ROW now (before+after highlight together, GitHub-style);
    // `side` is only remembered so a collapsed badge sits in that column's gutter.
    const existing = threads.find((t) => t.cardId === card.id && t.lineN === lineN);
    if (existing) {
      // #2 — a thread the user opened but never spoke into is transient: toggling
      // it shut (it's currently open with NO user message) discards it entirely
      // rather than leaving an empty collapsed badge. A thread that carries any
      // user message (question OR command) toggles open/closed as before, so
      // resolve/command history is always preserved.
      const hasUserMsg = existing.messages.some((m) => m.author === 'you');
      if (existing.open && !hasUserMsg) {
        setThreads((p) => p.filter((t) => t.id !== existing.id));
        // A transient thread can't be unread (it never received an answer), but
        // drop it from the set defensively so the id can't linger.
        clearUnread(existing.id);
        return;
      }
      const willOpen = !existing.open;
      setThreads((p) => p.map((t) => t.id === existing.id ? { ...t, open: willOpen } : t));
      // #8 read model: reading == expanding the thread. Opening it clears unread;
      // collapsing it does NOT (and merely navigating cards never touches unread).
      if (willOpen) clearUnread(existing.id);
      return;
    }
    const id = genThreadId();
    // No fabricated AI opener — a fresh thread is just the composer (empty
    // messages). The first real AI text arrives only after the user asks.
    setThreads((p) => [...p, {
      id, cardId: card.id, side: side || 'old', lineN, symbol: card.symbol, open: true, resolved: false,
      messages: [], pending: false, model: 'sonnet',
      // #3 — the dragged ROW range this thread was created over. Lets the review
      // screen faintly highlight exactly that region while the thread is open, so
      // the reviewer always sees what the question is about. Absent for a single-
      // line (plus-button) thread → falls back to just the anchor row.
      from: anchorRange && anchorRange.from != null ? anchorRange.from : lineN,
      to: anchorRange && anchorRange.to != null ? anchorRange.to : lineN,
      // The literal selected code (#3 region), so the AI prompt can quote exactly
      // what the user dragged — a vague "이거" then resolves to this region.
      selection: anchorRange && anchorRange.text ? anchorRange.text : '',
    }]);
  };

  // #2 — delete a thread outright (distinct from resolve, which keeps history).
  // Removes it from state (the debounced save effect then persists the removal)
  // and drops any lingering unread mark.
  const deleteThread = (id) => {
    setThreads((p) => p.filter((t) => t.id !== id));
    clearUnread(id);
  };

  // Per-thread model choice (#model): which CLI model answers this thread's
  // questions. 'sonnet' (default, accurate) or 'haiku' (faster). Updates only
  // the named thread; identity-stable when unchanged.
  const setThreadModel = (id, model) => setThreads((p) => {
    let changed = false;
    const next = p.map((t) => {
      if (t.id !== id || t.model === model) return t;
      changed = true;
      return { ...t, model };
    });
    return changed ? next : p;
  });

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
    // The exact region the user dragged over — the referent of a vague "이거".
    // Prefer the stored selection; fall back to the row index when absent (older
    // threads created before selection capture, or a programmatic open).
    const sel = t && t.selection
      ? `\n\n--- 사용자가 선택(드래그)한 영역 ---\n${t.selection}`
      : `\n\n질문 대상: line ${t && t.lineN != null ? t.lineN : '?'} (${side})`;
    return `${header}${span}\n\n${body}${sel}`;
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
    // Jump targets: every review card in flow order, numbered. The agent links a
    // reference to one of these as `[text](loupe-card:N)` (N = this list's number),
    // which Thread renders as a click-to-jump (see onNavigateCard). Same `list`
    // the spine/navigation use, so N maps back 1:1.
    const jump = (list || [])
      .map((cc, i) => `${i + 1}. ${cc.symbol || cc.path.split('/').pop()} — ${cc.path}`)
      .join('\n');
    const context = buildThreadContext(c, t)
      + (jump ? `\n\n--- 점프 가능한 리뷰 카드 (이 변경에 포함된 카드 목록) ---\n${jump}` : '');
    const history = t.messages.map((m) => ({ author: m.author, text: m.text }));

    // Mark the thread as thinking (spinner row in Thread.jsx).
    setThreads((p) => p.map((x) => x.id === id ? { ...x, pending: true } : x));

    // Per-thread model (#model): the chosen CLI model for this thread, defaulting
    // to 'sonnet' for any thread created before the field existed.
    const model = (t && t.model) || 'sonnet';
    invoke('ask_thread', { token, repoPath, context, question: text, history, model })
      .then((answer) => {
        let landedThread = null;
        setThreads((p) => p.map((x) => {
          if (x.id !== id) return x;
          landedThread = x; // freshest snapshot of the thread the answer is for
          return { ...x, pending: false, messages: [...x.messages, { author: 'ai', text: String(answer), time: 'now' }] };
        }));
        // #8 — if the user isn't actually looking at this answer right now, mark it
        // unread and flash its cluster on the collapsed spine. "Visibly open" means
        // the thread is expanded AND sitting on the active card; navigating cards or
        // collapsing the thread (without reading the new reply) leaves it unread.
        if (!landedThread) return; // thread was removed mid-flight — nothing to mark
        const visible = landedThread.open && activeCardIdRef.current === landedThread.cardId;
        if (visible) return;
        markUnread(landedThread.id, landedThread.cardId);
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
      // cmd/ctrl+E — Back to the previous card. Handled before the input guard so
      // it works even while a thread composer is focused (it's a deliberate combo).
      if ((e.metaKey || e.ctrlKey) && (e.key === 'e' || e.key === 'E')) { e.preventDefault(); goBack(); return; }
      const tag = (e.target.tagName || '').toLowerCase();
      if (tag === 'input' || tag === 'textarea' || tag === 'select') return;
      // ⌘/Ctrl+Z — undo the last pass (after the input guard, so it stays text-undo
      // inside a thread composer). Mis-pressed Space → ⌘Z puts the card back.
      if ((e.metaKey || e.ctrlKey) && (e.key === 'z' || e.key === 'Z')) { e.preventDefault(); undoPass(); return; }
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
        <div data-tauri-drag-region style={{ position: 'absolute', inset: 0, background: 'var(--bg-base)', overflow: 'hidden' }}>
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
          unreadThreads={unreadThreads}
          verdict={verdicts[card.id]} flagged={hasUnresolved(card.id)}
          hasPrev={index > 0} hasNext={index < list.length - 1}
          onPass={pass} onUnpass={() => unpass(card.id)} onPrev={prev} onNext={next}
          threads={cardThreads}
          onOpenLine={openLine} onResolve={resolveThread} onSend={sendThread}
          onSetThreadModel={setThreadModel} onDeleteThread={deleteThread}
          onNavigateCard={(n) => { const i = Number(n) - 1; if (i >= 0 && i < list.length) goTo(i, i > index ? 1 : -1); }}
          onOpenInEditor={openInEditor}
        />
      )}
      {screen === 'summary' && (
        <SummaryScreen cards={list} verdicts={verdicts} threads={threads}
          project={repoPath} base={base} target={target} onChangeProject={changeProject}
          prStatus={prStatus} approveState={approveState} onApprovePr={approvePr}
          onRestart={() => { setIndex(0); setDir(1); setScreen('review'); }} />
      )}

      <ScreenSwitcher screen={screen} setScreen={setScreen} onSettings={openSettings} />

      {/* transient toast — e.g. when the editor CLI launcher isn't installed */}
      {editorMsg && (
        <div style={{ position: 'fixed', bottom: 24, left: '50%', transform: 'translateX(-50%)',
          zIndex: 200, maxWidth: 520, padding: '10px 14px', borderRadius: 'var(--radius-md)',
          background: 'var(--surface-overlay)', border: '1px solid var(--flag-line)',
          boxShadow: 'var(--shadow-pop)', color: 'var(--text-secondary)',
          font: 'var(--text-sm)/1.4 var(--font-ui)' }}>
          {editorMsg}
        </div>
      )}
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
