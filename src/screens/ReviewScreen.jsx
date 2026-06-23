/* Loupe UI kit — the main review screen (the centerpiece).
   A near-empty canvas: a dim queue spine on the left and one focused card
   in the center. Keyboard-first: Space = Pass + advance, ←/→ = move.
   Split diff (before | after). Dragging across rows opens an inline AI thread. */

import React from 'react';
import { ProgressSpine } from '../components/ProgressSpine';
import { Thread } from '../components/Thread';
import { Button } from '../components/Button';
import { KeyHint } from '../components/KeyHint';
import ProjectMenu from '../components/ProjectMenu';
import { highlightGo } from '../data/fixtures';
import intellijLogo from '../assets/editor-intellij.svg';
import vscodeLogo from '../assets/editor-vscode.svg';

// Memoize syntax highlighting per source string: the same line text re-tokenizes
// to the same keyed span array, so windowing (mount/unmount as you scroll) never
// pays to re-highlight a line it has already seen. Bounded to avoid unbounded growth.
// Collapse a long file path to its first `head` + last `tail` segments with an
// ellipsis between (e.g. src/main/…/bus/BusRouteInfoClients.kt) so the card header
// never wraps or clips the path. The filename (last segment) is always kept.
function middlePath(p, head = 2, tail = 2) {
  const parts = String(p || '').split('/').filter(Boolean);
  if (parts.length <= head + tail + 1) return p;
  return [...parts.slice(0, head), '…', ...parts.slice(-tail)].join('/');
}

const _hlCache = new Map();
function hl(s) {
  if (_hlCache.has(s)) return _hlCache.get(s);
  const v = highlightGo(s);
  _hlCache.set(s, v);
  if (_hlCache.size > 5000) _hlCache.clear();
  return v;
}

// Small settings popover — adjust code text size. Opens upward from a subtle
// "Aa" trigger in the bottom bar. Simple + intuitive: one slider, an Auto reset.
function TextSizeMenu({ size, effective, onChange }) {
  const [open, setOpen] = React.useState(false);
  const ref = React.useRef(null);
  React.useEffect(() => {
    if (!open) return;
    const onDoc = (e) => { if (ref.current && !ref.current.contains(e.target)) setOpen(false); };
    document.addEventListener('mousedown', onDoc);
    return () => document.removeEventListener('mousedown', onDoc);
  }, [open]);
  const val = size != null ? size : effective;
  const isAuto = size == null;

  return (
    <div ref={ref} style={{ position: 'relative' }}>
      <button onClick={() => setOpen((v) => !v)} title="Text size" aria-label="Text size"
        style={{ display: 'inline-flex', alignItems: 'baseline', gap: 3, height: 26, padding: '0 8px',
          borderRadius: 'var(--radius-sm)', cursor: 'pointer',
          background: open ? 'var(--surface-overlay)' : 'transparent',
          border: `1px solid ${open ? 'var(--border-default)' : 'transparent'}`,
          color: open ? 'var(--text-secondary)' : 'var(--text-tertiary)',
          opacity: open ? 1 : 'var(--dim-rest)', transition: 'var(--t-hover)' }}>
        <span style={{ font: 'var(--weight-semibold) 15px/1 var(--font-ui)' }}>A</span>
        <span style={{ font: 'var(--weight-medium) 10px/1 var(--font-ui)' }}>A</span>
      </button>

      {open && (
        <div style={{ position: 'absolute', bottom: 'calc(100% + 10px)', left: 0, width: 224,
          background: 'var(--surface-overlay)', border: '1px solid var(--border-default)',
          borderRadius: 'var(--radius-md)', boxShadow: 'var(--shadow-pop)', padding: '14px 14px 12px', zIndex: 50 }}>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 12 }}>
            <span style={{ font: 'var(--weight-medium) var(--text-sm)/1 var(--font-ui)', color: 'var(--text-primary)' }}>Text size</span>
            <span style={{ font: 'var(--text-xs)/1 var(--font-mono)', color: 'var(--text-tertiary)',
              fontVariantNumeric: 'tabular-nums' }}>{val}px</span>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <span style={{ font: '11px/1 var(--font-ui)', color: 'var(--text-faint)' }}>A</span>
            <input type="range" min={8} max={20} step={1} value={val}
              onChange={(e) => onChange(Number(e.target.value))}
              style={{ flex: 1, accentColor: 'var(--accent)', cursor: 'pointer' }} />
            <span style={{ font: '16px/1 var(--font-ui)', color: 'var(--text-faint)' }}>A</span>
          </div>
          <button onClick={() => onChange(null)} disabled={isAuto}
            style={{ marginTop: 12, width: '100%', height: 28, borderRadius: 'var(--radius-sm)',
              background: 'transparent', border: '1px solid var(--border-default)',
              color: isAuto ? 'var(--text-faint)' : 'var(--text-secondary)',
              cursor: isAuto ? 'default' : 'pointer', opacity: isAuto ? 0.5 : 1,
              font: 'var(--weight-medium) var(--text-xs)/1 var(--font-ui)' }}>
            {isAuto ? 'Auto (fits the change)' : 'Reset to Auto'}
          </button>
        </div>
      )}
    </div>
  );
}

// Global editor SELECTOR (bottom bar). Its trigger IS the active editor's logo;
// the popover just SETS which editor ⌘-clicking a diff line opens into — it does
// NOT open anything itself. Default IntelliJ. Dimmed at rest, lights up on hover.
const EDITOR_LOGO = { idea: intellijLogo, code: vscodeLogo, auto: intellijLogo };
function OpenInEditorMenu() {
  const [open, setOpen] = React.useState(false);
  const [rect, setRect] = React.useState(null);
  // The active editor — its logo IS the trigger. Default IntelliJ.
  const [current, setCurrent] = React.useState(() => {
    try { return window.localStorage.getItem('loupe.editor') || 'idea'; } catch { return 'idea'; }
  });
  const trigRef = React.useRef(null);
  const popRef = React.useRef(null);
  React.useEffect(() => {
    if (!open) return;
    const close = (e) => {
      if (trigRef.current && trigRef.current.contains(e.target)) return;
      if (popRef.current && popRef.current.contains(e.target)) return;
      setOpen(false);
    };
    document.addEventListener('mousedown', close);
    return () => document.removeEventListener('mousedown', close);
  }, [open]);
  const toggle = () => {
    if (!open && trigRef.current) setRect(trigRef.current.getBoundingClientRect());
    setOpen((v) => !v);
  };
  // Select only — persist the choice; ⌘-click on a diff line does the opening.
  const pick = (id) => {
    setCurrent(id);
    setOpen(false);
    try { window.localStorage.setItem('loupe.editor', id); } catch { /* ignore */ }
  };
  const items = [
    { id: 'idea', label: 'IntelliJ IDEA', logo: intellijLogo },
    { id: 'code', label: 'VS Code', logo: vscodeLogo },
  ];
  return (
    <React.Fragment>
      <button ref={trigRef} onClick={toggle} type="button"
        title="열 에디터 선택 (⌘-클릭으로 diff에서 열기)" aria-label="열 에디터 선택"
        style={{ display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flex: 'none',
          width: 26, height: 26, borderRadius: 'var(--radius-sm)', cursor: 'pointer',
          background: open ? 'var(--surface-overlay)' : 'transparent',
          border: `1px solid ${open ? 'var(--border-default)' : 'transparent'}`,
          opacity: open ? 1 : 'var(--dim-rest)', transition: 'var(--t-hover)' }}
        onMouseEnter={(e) => { e.currentTarget.style.opacity = 1; }}
        onMouseLeave={(e) => { if (!open) e.currentTarget.style.opacity = 'var(--dim-rest)'; }}>
        <img src={EDITOR_LOGO[current] || intellijLogo} width="16" height="16" alt="" draggable="false" />
      </button>
      {open && rect && (
        // Opens UPWARD (translateY -100%) — this control lives in the bottom bar.
        <div ref={popRef} style={{ position: 'fixed', top: rect.top, left: rect.left,
          transform: 'translateY(calc(-100% - 6px))',
          width: 188, zIndex: 60, background: 'var(--surface-overlay)',
          border: '1px solid var(--border-default)', borderRadius: 'var(--radius-md)',
          boxShadow: 'var(--shadow-pop)', padding: 4 }}>
          <div style={{ padding: '5px 8px 6px', font: 'var(--weight-medium) 10px/1 var(--font-ui)',
            letterSpacing: 'var(--tracking-caps)', textTransform: 'uppercase', color: 'var(--text-faint)' }}>다음으로 열기</div>
          {items.map((it) => (
            <button key={it.id} type="button" onClick={() => pick(it.id)}
              style={{ display: 'flex', alignItems: 'center', gap: 9, width: '100%', padding: '8px',
                borderRadius: 'var(--radius-sm)', cursor: 'pointer', textAlign: 'left', border: 'none',
                background: 'transparent', color: 'var(--text-secondary)',
                font: 'var(--weight-medium) var(--text-sm)/1 var(--font-ui)' }}
              onMouseEnter={(e) => { e.currentTarget.style.background = 'var(--surface-inset)'; }}
              onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent'; }}>
              <img src={it.logo} width="16" height="16" alt="" draggable="false" style={{ flex: 'none' }} />
              {it.label}
            </button>
          ))}
        </div>
      )}
    </React.Fragment>
  );
}

export default function ReviewScreen(props) {
  const {
    card, index, total, dir, project, base, target, onChangeProject, unresolved,
    cluster, clusterIndex, analysisState,
    spineItems, onSelect,
    verdict, flagged, hasPrev, hasNext,
    onPass, onUnpass, onPrev, onNext, onJumpUnresolved,
    threads, onOpenLine, onResolve, onSend, onSetThreadModel, onDeleteThread, onNavigateCard, onOpenInEditor,
    // #8/#9 shared contract — set of threadId with an arrived AI reply that has
    // not yet been read (read = expanding that thread). App owns the set; when it
    // doesn't pass one yet, default to an empty Set so `.has(...)` stays safe.
    unreadThreads = new Set(),
  } = props;

  // Per-thread element refs for the open-thread wrappers, so a newly-opened
  // thread can be scrolled clear of the sticky column header (see the layout
  // effect below). Keyed by thread id.
  const threadEls = React.useRef({});
  // The set of thread ids that were open on the previous render — diffed against
  // the current open set to detect which thread was JUST opened this commit.
  const prevOpenIds = React.useRef(new Set());
  // The opaque sticky column header. We measure its REAL height (it is ~25px:
  // 7+7 padding + 11px text + 1px border, not the 44 that was hard-coded) so the
  // scroll-correction below reveals a thread by exactly the header it must clear,
  // and so the open-thread wrapper's scrollMarginTop matches it.
  const headerRef = React.useRef(null);
  const [headerH, setHeaderH] = React.useState(26);

  // ---- Build aligned split rows (before | after) from the unified line list.
  const rows = React.useMemo(() => {
    // Use the engine's per-line number (`ln.n` — the real new-file gutter) AS IS.
    // Re-deriving it from a single start drifted from the actual file line numbers
    // (e.g. when a card starts with deleted lines), which also threw off ⌘-click.
    const out = [];
    let pendDel = [], pendAdd = [];
    const flush = () => {
      const n = Math.max(pendDel.length, pendAdd.length);
      for (let i = 0; i < n; i++) out.push({ kind: 'change', left: pendDel[i] || null, right: pendAdd[i] || null });
      pendDel = []; pendAdd = [];
    };
    card.lines.forEach((ln) => {
      if (ln.t === 'ctx') { flush(); out.push({ kind: 'ctx', left: { n: ln.n, c: ln.c }, right: { n: ln.n, c: ln.c } }); }
      else if (ln.t === 'del') pendDel.push({ n: ln.n, c: ln.c });
      else if (ln.t === 'add') pendAdd.push({ n: ln.n, c: ln.c });
    });
    flush();
    return out;
  }, [card.id]);

  // drag-to-create-thread state. The selection is by ROW (before+after
  // highlight together, like GitHub), but we remember which SIDE the drag
  // began on so a collapsed thread's badge sits in that column's gutter.
  const [dragSide, setDragSide] = React.useState(null);
  const [dragFrom, setDragFrom] = React.useState(null);
  const [dragTo, setDragTo] = React.useState(null);
  const [hover, setHover] = React.useState(null); // { side, r }
  const dragging = dragFrom != null;
  // While selecting text, lock selection to ONE side (before OR after) so a copy
  // drag doesn't grab both columns. Set on mousedown over a side, cleared on mouseup.
  const [selectSide, setSelectSide] = React.useState(null);
  // Brief "pressed" flash on the ⌘-clicked cell — only the side (before/after)
  // you actually clicked, not both. Holds { r, side } | null.
  const [flashRow, setFlashRow] = React.useState(null);
  const flashTimer = React.useRef(0);
  const pressRow = React.useCallback((r, side) => {
    setFlashRow({ r, side });
    clearTimeout(flashTimer.current);
    flashTimer.current = setTimeout(() => setFlashRow(null), 220);
  }, []);
  const inRange = (i) => dragging &&
    i >= Math.min(dragFrom, dragTo) && i <= Math.max(dragFrom, dragTo);
  const endDrag = () => {
    if (dragFrom == null) return;
    // Anchor the thread (and its collapsed badge) on the row where the drag
    // ENDED, not where it began.
    const side = dragSide, f = dragTo != null ? dragTo : dragFrom;
    const from = Math.min(dragFrom, dragTo), to = Math.max(dragFrom, dragTo);
    // Capture the literal code the user dragged over, so the thread prompt can
    // anchor a vague "이거 / this" to the EXACT selected region (not a row index).
    // Change rows render as -old / +new; context rows as a plain (unchanged) line.
    const text = rows.slice(from, to + 1).map((row) => {
      if (row.kind === 'ctx') return '  ' + ((row.right && row.right.c) || (row.left && row.left.c) || '');
      const parts = [];
      if (row.left && row.left.c != null) parts.push('- ' + row.left.c);
      if (row.right && row.right.c != null) parts.push('+ ' + row.right.c);
      return parts.join('\n');
    }).join('\n');
    setDragSide(null); setDragFrom(null); setDragTo(null);
    // Pass the dragged ROW range + selected text so the thread remembers its
    // region (#3) and the AI prompt can quote exactly what was selected.
    onOpenLine(side, f, { from, to, text });
  };

  // thread lookup keyed by row
  const threadByRow = React.useMemo(() => {
    const m = {};
    threads.forEach((t) => { m[t.lineN] = t; });
    return m;
  }, [threads]);
  // A collapsed thread's badge under the cursor → preview its region too.
  const [hoverThreadId, setHoverThreadId] = React.useState(null);

  // #3 — rows covered by an OPEN thread's dragged region (or the collapsed thread
  // whose badge is hovered). We faintly tint exactly those rows so the reviewer
  // sees what the thread is about. A Set (not a single lo..hi span) so two disjoint
  // threads don't bleed a highlight across the gap between them.
  // Side-keyed (`old:row` / `new:row`) so the tint lands ONLY on the thread's own
  // side (before OR after), matching how the drag selection is one-sided.
  const openRegionRows = React.useMemo(() => {
    const s = new Set();
    threads.forEach((t) => {
      if ((t.open || t.id === hoverThreadId) && t.from != null && t.to != null) {
        const sd = t.side || 'old';
        for (let k = t.from; k <= t.to; k++) s.add(sd + ':' + k);
      }
    });
    return s;
  }, [threads, hoverThreadId]);

  // ---- Fold unchanged context so big functions fit at a glance ----
  // Keep CONTEXT lines of context around each change; collapse the rest into
  // a thin "⋯ N unchanged lines" divider the reviewer can expand. Font stays
  // large; only the changes (the point of the card) compete for the eye.
  const [expanded, setExpanded] = React.useState(() => new Set());
  // Header path starts collapsed (first/last segments) on every card; click expands.
  const [pathFull, setPathFull] = React.useState(false);
  const [passHover, setPassHover] = React.useState(false); // "Passed" badge → "패스 취소" on hover
  React.useEffect(() => { setExpanded(new Set()); setPathFull(false); }, [card.id]);
  const CONTEXT = 2;
  const display = React.useMemo(() => {
    const items = []; const N = rows.length; let i = 0;
    const hasThread = (a, b) => { for (let k = a; k < b; k++) if (threadByRow[k]) return true; return false; };
    while (i < N) {
      if (rows[i].kind !== 'ctx') { items.push({ type: 'row', r: i, row: rows[i] }); i++; continue; }
      let j = i; while (j < N && rows[j].kind === 'ctx') j++;
      const runLen = j - i;
      const keepTop = i === 0 ? 0 : CONTEXT;
      const keepBottom = j === N ? 0 : CONTEXT;
      if (expanded.has(i) || hasThread(i, j) || runLen <= keepTop + keepBottom + 1) {
        for (let k = i; k < j; k++) items.push({ type: 'row', r: k, row: rows[k] });
      } else {
        for (let k = i; k < i + keepTop; k++) items.push({ type: 'row', r: k, row: rows[k] });
        items.push({ type: 'fold', key: i, count: runLen - keepTop - keepBottom });
        for (let k = j - keepBottom; k < j; k++) items.push({ type: 'row', r: k, row: rows[k] });
      }
      i = j;
    }
    return items;
  }, [rows, expanded, threadByRow]);

  // Adaptive code size: a brand-new function is all additions (nothing to
  // fold), so scale the font to how many rows actually show — fewer rows get
  // a comfortable size, long ones shrink toward a readable floor (never tiny)
  // so the card needs little or no scrolling.
  const rowCount = display.reduce((n, it) => n + (it.type === 'row' ? 1 : 0), 0);
  const autoFs = rowCount <= 16 ? 14 : rowCount <= 24 ? 13 : rowCount <= 34 ? 12 : 11;
  const [codeFontSize, setCodeFontSize] = React.useState(() => {
    const v = localStorage.getItem('loupe.codeFontSize');
    return v != null ? Number(v) : null;
  });
  const setFont = (v) => {
    setCodeFontSize(v);
    if (v == null) localStorage.removeItem('loupe.codeFontSize');
    else localStorage.setItem('loupe.codeFontSize', String(v));
  };
  const codeFs = codeFontSize != null ? codeFontSize : autoFs;
  // ⌘/Ctrl + wheel over the code view zooms the font. The wheel listener is bound
  // per card, so it reads the live size via a ref; the accumulator smooths trackpad
  // deltas so one notch ≈ one step.
  const effFontRef = React.useRef(codeFs);
  effFontRef.current = codeFs;
  const fontWheelAccum = React.useRef(0);

  // ---- No-wrap horizontal scroll ----
  // Lines no longer wrap; the diff scrolls left/right. To keep both columns
  // equal (centered divider) we give each side a min-width that fits the
  // longest line, measured in real monospace px for the current font size.
  // Rendered column width of a line — NOT .length. A CJK/full-width glyph (e.g.
  // Korean in comments) occupies ~2 monospace cells but counts as 1 char, and a
  // tab advances up to tabSize(2). Using .length under-measured these, so the
  // h-scroll spacer was too short and you couldn't scroll to the end of a line
  // with Korean text or tabs. Count them at their real cell width.
  const cellWidth = (s) => {
    let w = 0;
    for (let i = 0; i < s.length; i++) {
      const c = s.charCodeAt(i);
      if (c === 9) { w += 2; continue; } // tab
      w += (c >= 0x1100 && (
        c <= 0x115f ||                     // Hangul Jamo
        (c >= 0x2e80 && c <= 0xa4cf) ||    // CJK radicals … Yi
        (c >= 0xac00 && c <= 0xd7a3) ||    // Hangul syllables
        (c >= 0xf900 && c <= 0xfaff) ||    // CJK compatibility
        (c >= 0xfe30 && c <= 0xfe4f) ||    // CJK compat forms
        (c >= 0xff00 && c <= 0xff60) ||    // full-width forms
        (c >= 0xffe0 && c <= 0xffe6)
      )) ? 2 : 1;
    }
    return w;
  };
  const maxChars = React.useMemo(() => {
    let m = 0;
    rows.forEach((r) => {
      if (r.left && r.left.c) m = Math.max(m, cellWidth(r.left.c));
      if (r.right && r.right.c) m = Math.max(m, cellWidth(r.right.c));
    });
    return Math.max(m, 24);
  }, [card.id]);
  const charRef = React.useRef(null);
  const [chPx, setChPx] = React.useState(8.4);
  React.useLayoutEffect(() => {
    if (charRef.current) setChPx(charRef.current.getBoundingClientRect().width / 50);
  }, [codeFs]);
  const GUTTER = 52; // hbar left spacer = Half grid fixed cols (10+30+12), so the
                     // scroll track mirrors the code column 1:1 (no phantom / no clip)
  // Each side's code area is its OWN horizontal scroller; short lines get a
  // min-width so every line on a side scrolls by the same amount, and a
  // capture-scroll handler keeps all lines of one side in lock-step (so the
  // BEFORE column and the AFTER column scroll independently of each other).
  // Span width (border-box, so padding is INCLUDED): text width + L/R padding +
  // a few px slack to absorb sub-pixel chPx error. The h-bar scroller uses this
  // SAME width and a gutter equal to the grid's fixed columns, so the scrollbar
  // mirrors the span exactly — it scrolls iff the span overflows the code column,
  // by exactly the overflow (no phantom scroll, no un-scrollable clipped tail).
  const codeContentW = Math.ceil(maxChars * chPx + 28);
  // Responsive card width: grows with the actual code length up to a max, so
  // short changes get a compact card and long ones widen toward the cap.
  // No hard px cap: grow with the code, and let the card wrapper's maxWidth:100%
  // (the stage's 76px side padding, which clears the edge nav arrows) cap it to the
  // window. So the card is as wide as it needs, up to just inside the arrows.
  const cardW = Math.max(760, (52 + codeContentW) * 2 + 6);
  const diffRef = React.useRef(null);
  // The two bottom horizontal scrollbars (one per side) are the ONLY horizontal
  // controls. Their onScroll writes a CSS variable (--hs-old / --hs-new) on the
  // diff container; every code line of that side reads the variable via a CSS
  // translateX, so a horizontal scroll is O(1) (one setProperty) and rows that
  // mount during windowing inherit the offset automatically (no JS per row).
  const hbarOldRef = React.useRef(null);
  const hbarNewRef = React.useRef(null);
  // Wheel-driven horizontal scroll: route a horizontal wheel onto the bar of the
  // side under the cursor; its onScroll then updates the CSS variable for us.
  // NOTE: this MUST be a native non-passive listener. React delegates `onWheel`
  // as a PASSIVE root listener, so a React onWheel handler cannot preventDefault
  // (it no-ops + warns, and the browser keeps the native gesture — e.g. macOS
  // swipe-back). We bind it ourselves with { passive: false } so preventDefault
  // actually consumes the horizontal gesture.

  // ---- Vertical windowing (virtualization) for the split diff ----
  // Default row height from the code line-height; used only for not-yet-measured
  // rows. Actual heights are measured per row (see rowH/offsets below) so threads,
  // collapsed badges and fold dividers contribute their real height with no drift.
  const RH = Math.max(1, Math.round(codeFs * 1.72));
  const [scrollTop, setScrollTop] = React.useState(0);
  const [viewportH, setViewportH] = React.useState(800);
  const scrollRaf = React.useRef(0);
  // Latest scrollTop seen since the last commit. #7: the rAF reads THIS (trailing
  // edge) rather than the value captured when it was scheduled, so a fast scroll
  // that fires many events inside one frame still commits the FINAL position — not
  // a stale leading-edge sample. Committing a stale top is what blanks the window
  // (the slice we render no longer covers the real viewport) and reads as flicker.
  const pendingTop = React.useRef(0);
  // rAF-throttled vertical scroll: only the OUTER container scrolls vertically
  // (horizontal scrolling is the bottom hbars writing a CSS variable). We coalesce
  // a burst of scroll events into one state commit per frame, but always with the
  // most recent scrollTop (trailing rAF) so virtualization stays in lock-step.
  const onDiffScroll = (e) => {
    if (e.currentTarget !== diffRef.current) return;
    pendingTop.current = e.currentTarget.scrollTop;
    if (scrollRaf.current) return;
    scrollRaf.current = requestAnimationFrame(() => {
      scrollRaf.current = 0;
      setScrollTop(pendingTop.current);
    });
  };
  React.useEffect(() => () => { if (scrollRaf.current) cancelAnimationFrame(scrollRaf.current); }, []);
  // Track the viewport height of the scroll container (mount + on resize).
  React.useEffect(() => {
    const el = diffRef.current;
    if (!el) return;
    setViewportH(el.clientHeight || 800);
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => { if (diffRef.current) setViewportH(diffRef.current.clientHeight || 800); });
    ro.observe(el);
    return () => ro.disconnect();
    // card.id: the <div key={card.id}> wrapper remounts the diff container on
    // every card change, so diffRef points at a fresh node each time — we must
    // re-observe it (and re-measure clientHeight) per card.
  }, [card.id]);
  // Native non-passive wheel listener (see note on hbarOldRef above): horizontal
  // wheel → push the hovered side's bottom hbar, whose onScroll sets the CSS var.
  // Re-bind per card (the diff node is fresh after each key={card.id} remount).
  React.useEffect(() => {
    const el = diffRef.current;
    if (!el) return;
    const onWheel = (e) => {
      // ⌘/Ctrl + wheel → zoom the code font (up = larger), not scroll.
      if (e.metaKey || e.ctrlKey) {
        e.preventDefault();
        fontWheelAccum.current += e.deltaY;
        const STEP = 30;
        if (Math.abs(fontWheelAccum.current) >= STEP) {
          const dir = fontWheelAccum.current < 0 ? 1 : -1;
          fontWheelAccum.current = 0;
          const cur = Math.round(effFontRef.current);
          const next = Math.max(8, Math.min(20, cur + dir));
          if (next !== cur) setFont(next);
        }
        return;
      }
      if (Math.abs(e.deltaX) <= Math.abs(e.deltaY)) return; // vertical → container scrolls
      const half = e.target.closest && e.target.closest('[data-side]');
      const side = half && half.getAttribute('data-side');
      const bar = side === 'new' ? hbarNewRef.current : hbarOldRef.current;
      if (bar) { bar.scrollLeft += e.deltaX; e.preventDefault(); }
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => el.removeEventListener('wheel', onWheel);
  }, [card.id]);
  // ---- Measured variable-height windowing ----
  // Each displayed item's real pixel height, indexed by its position in `display`.
  // Not-yet-measured items fall back to RH. A measuring ref on each item's top
  // element reports offsetHeight; we store it and bump measureTick to recompute
  // offsets. Reset per card so a reused container never carries stale heights.
  const rowH = React.useRef([]);
  const [measureTick, setMeasureTick] = React.useState(0);

  // Reset scroll + measured heights when switching cards (container is reused).
  React.useEffect(() => {
    rowH.current = [];
    if (diffRef.current) diffRef.current.scrollTop = 0;
    setScrollTop(0);
  }, [card.id]);

  const N = display.length;

  // Measure the sticky column header's real height (mount + per card remount +
  // when the code font changes its line metrics). Used by the scroll-correction
  // effect and the open-thread wrapper's scrollMarginTop. Layout effect so the
  // value is correct before the first paint that could need it.
  React.useLayoutEffect(() => {
    if (headerRef.current) {
      const h = headerRef.current.offsetHeight;
      if (h > 0) setHeaderH((prev) => (Math.abs(prev - h) > 0.5 ? h : prev));
    }
  }, [card.id, codeFs]);

  // Prefix-sum offsets of every item top (offsets[i]) and the total height.
  // (totalH, not `total` — that prop is the card count.) Recomputed when the
  // item list, font (RH), or a fresh measurement changes.
  const { offsets, totalH } = React.useMemo(() => {
    const o = new Array(N + 1);
    o[0] = 0;
    for (let i = 0; i < N; i++) o[i + 1] = o[i] + (rowH.current[i] || RH);
    return { offsets: o, totalH: o[N] };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [N, RH, measureTick, display]);

  // Binary search: first index whose BOTTOM edge passes `y` (offsets[i+1] > y).
  const firstBelow = (y) => {
    let lo = 0, hi = N;
    while (lo < hi) { const m = (lo + hi) >> 1; if (offsets[m + 1] > y) hi = m; else lo = m + 1; }
    return lo;
  };
  // Binary search: first index whose TOP edge is at/after `y` (offsets[i] >= y).
  const firstAtOrAfter = (y) => {
    let lo = 0, hi = N;
    while (lo < hi) { const m = (lo + hi) >> 1; if (offsets[m] >= y) hi = m; else lo = m + 1; }
    return lo;
  };

  // #7: generous overscan (px above/below the viewport). Rows render in a band
  // this much taller than the viewport, so a fast flick stays inside the already-
  // mounted band for several frames before the next commit lands — eliminating the
  // blank gap (flicker) you'd see when the scroll outruns a tighter band. Paired
  // with the trailing-rAF commit above and per-row measured heights, the window
  // stays accurate and deterministic; the only cost is a few extra mounted rows.
  const OVERPX = 1600; // overscan in px above/below the viewport
  const startIdx = N === 0 ? 0 : firstBelow(scrollTop - OVERPX);
  // At least one item past startIdx so a single tall row (e.g. an open thread)
  // taller than the viewport still renders instead of collapsing to an empty slice.
  const endIdx = N === 0 ? 0 : Math.max(startIdx + 1, firstAtOrAfter(scrollTop + viewportH + OVERPX));
  const topPad = offsets[startIdx] || 0;
  const bottomPad = Math.max(0, totalH - (offsets[endIdx] || totalH));

  // Measuring ref, stabilized per global index. A NEW ref callback identity each
  // render makes React detach (call with null) then re-attach (call with the node)
  // every render — during a thread's enter animation that's a churn of ref calls,
  // each re-measuring and (when the height shifts) bumping measureTick: a render
  // burst. Caching one callback per `gi` keeps ref identity stable across renders,
  // so React leaves the ref attached and the callback fires only on real mount/
  // unmount or resize — not on every re-render.
  //
  // The body still bumps measureTick only when the height actually changed (> 0.5px).
  // That guard is what makes measurement converge instead of looping: once a row's
  // stored height matches its rendered height (within 0.5px) the callback is a
  // no-op, so it triggers no further render — measurement reaches a fixed point.
  // `rowH` (a ref) and `setMeasureTick` (a state setter) are stable, so a cached
  // closure stays correct for the life of the component.
  const measureCbs = React.useRef(new Map());
  const measure = React.useCallback((gi) => {
    let cb = measureCbs.current.get(gi);
    if (!cb) {
      cb = (el) => {
        if (!el) return;
        const h = el.offsetHeight;
        if (Math.abs((rowH.current[gi] || 0) - h) > 0.5) {
          rowH.current[gi] = h;
          setMeasureTick((t) => t + 1);
        }
      };
      measureCbs.current.set(gi, cb);
    }
    return cb;
  }, []);

  // When a thread is opened, its wrapper (which holds the resolve/collapse
  // buttons at its top) can sit underneath the opaque sticky column header
  // (top:0, zIndex:2) and be hidden. We scroll the diff container so the thread's
  // top clears the sticky header. Runs before paint (useLayoutEffect) so the user
  // never sees the clipped state. Top-level hook (no early return).
  //
  // The thread that was just opened and still needs its scroll settled. The
  // measure ref re-renders (setMeasureTick) AFTER this thread's height grows,
  // which re-lays-out the virtualization spacers and slides the corrected thread
  // back under the header. So we don't correct only on the open commit: we latch
  // the id here and keep re-correcting on every subsequent commit (incl. the
  // measureTick re-renders) until the position is stable, then clear the latch.
  const pendingReveal = React.useRef(null);
  const prevThreadsRef = React.useRef(threads);
  React.useLayoutEffect(() => {
    const openIds = new Set(threads.filter((t) => t.open).map((t) => t.id));
    // Find an id that is open now but was NOT open on the previous render.
    let justOpened = null;
    openIds.forEach((id) => { if (!prevOpenIds.current.has(id)) justOpened = id; });
    // Did the threads array itself change (a message sent / AI reply landed),
    // vs. this effect re-running only because of a measureTick re-layout?
    const threadsChanged = prevThreadsRef.current !== threads;
    prevThreadsRef.current = threads;
    prevOpenIds.current = openIds;
    if (justOpened) {
      // Latch a freshly-opened thread to scroll its top clear of the header.
      pendingReveal.current = justOpened;
    } else if (threadsChanged) {
      // Content changed in an already-open thread (you typed a follow-up, or the
      // AI replied) — do NOT re-scroll. Re-revealing here is what yanked the view
      // back to the top mid-conversation. Drop the latch and leave scroll alone.
      pendingReveal.current = null;
    }
    // (A measureTick-only re-run keeps the latch so the open-reveal can settle.)
    const target = pendingReveal.current;
    if (!target) return;
    // If the latched thread is no longer open (collapsed/resolved), drop it.
    if (!openIds.has(target)) { pendingReveal.current = null; return; }
    const el = threadEls.current[target];
    const cont = diffRef.current;
    if (!el || !cont) { pendingReveal.current = null; return; }
    // Bring the whole opened thread into view between the sticky column header
    // (top) and the sticky h-scrollbar (bottom). Handles BOTH cases: top hidden
    // under the header → scroll up; bottom spilling past the viewport → scroll
    // down. (The earlier version only handled the top, so a thread opening low
    // in the viewport stayed clipped below its action buttons.)
    const HEADER = headerH; // measured sticky column-header height (~25px, not 44)
    const FOOTER = 12;      // sticky bottom h-scrollbar (~8px bar + slack)
    const cR = cont.getBoundingClientRect();
    const eR = el.getBoundingClientRect();
    const viewTop = cR.top + HEADER;
    const viewBot = cR.bottom - FOOTER;
    let delta = 0;
    if (eR.height >= viewBot - viewTop) {
      delta = eR.top - viewTop - 8;                  // taller than viewport → align its top (clear header)
    } else if (eR.top < viewTop + 8) {
      delta = eR.top - viewTop - 8;                  // top hidden/touching header → reveal top
    } else if (eR.bottom > viewBot) {
      delta = eR.bottom - viewBot + 8;               // bottom spills below → scroll down to reveal
    }
    if (Math.abs(delta) > 0.5) {
      cont.scrollTop += delta;                       // (browser clamps to [0, max]; zIndex:3 covers the clamp case)
      // Position not yet settled this commit — keep the latch so the next commit
      // (e.g. the measureTick re-render) re-checks and finishes the job.
    } else {
      pendingReveal.current = null;                  // settled within tolerance → stop.
    }
    // threads: re-run when a thread opens/closes. card.id: a card switch remounts
    // the diff container. measureTick: the measure ref's re-layout moves the
    // thread, so re-settle the scroll AFTER measurement lands (this is the deps
    // entry that fixes the clip — the old effect ran once and was invalidated by
    // the very next measure render).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [threads, card.id, measureTick, headerH]);

  const Ico = ({ d, w = 15 }) => (
    <svg width={w} height={w} viewBox="0 0 24 24" fill="none" stroke="currentColor"
      strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round"><path d={d} /></svg>
  );
  const check = 'M20 6 9 17l-5-5';
  const flagPath = 'M4 15s1-1 4-1 5 2 8 2 4-1 4-1V3s-1 1-4 1-5-2-8-2-4 1-4 1z M4 22V4';
  const arrow = 'M5 12h14M13 6l6 6-6 6';
  const chevL = 'M15 18l-6-6 6-6';
  const chevR = 'M9 18l6-6-6-6';
  const ordinal = (n) => {
    const s = ['th', 'st', 'nd', 'rd'], v = n % 100;
    return n + (s[(v - 20) % 10] || s[v] || s[0]);
  };
  const remaining = total - index - 1;

  // #4: bare chevron anchored to the CANVAS edge (the main stage minus the sidebar),
  // vertically centered — independent of card width. `side` is 'left'/'right' and the
  // arrow pins ~16px from that edge of the stage container (which is position:relative).
  // Enlarged hit/visual area (~44×96) with a bigger chevron so the controls read clearly
  // and never overlap the card center (the stage has horizontal padding reserving room).
  const NavArrow = ({ side, d, disabled, onClick, label }) => (
    <button onClick={disabled ? undefined : onClick} aria-label={label} title={label} disabled={disabled}
      style={{ position: 'absolute', zIndex: 5, top: '50%', transform: 'translateY(-50%)',
        [side]: 16, width: 44, height: 96, padding: 0,
        display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
        background: 'transparent', border: 'none',
        color: 'var(--text-secondary)', cursor: disabled ? 'default' : 'pointer',
        opacity: disabled ? 0.12 : 'var(--dim-rest)', transition: 'var(--t-dim), var(--t-hover)' }}
      onMouseEnter={(e) => { if (!disabled) { e.currentTarget.style.opacity = 1; e.currentTarget.style.color = 'var(--text-primary)'; } }}
      onMouseLeave={(e) => { if (!disabled) { e.currentTarget.style.opacity = 'var(--dim-rest)'; e.currentTarget.style.color = 'var(--text-secondary)'; } }}>
      <svg width="34" height="34" viewBox="0 0 24 24" fill="none" stroke="currentColor"
        strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d={d} /></svg>
    </button>
  );

  // one side (old/new) of a split row. Selection highlight is row-wide; the
  // + affordance and a collapsed thread's badge are per-side (this column).
  const Half = ({ cell, kind, side, r, active, isFirst, isLast, thread, region, regFirst, regLast, navLine, flash }) => {
    const isChange = kind === 'change';
    const tone = side === 'old' ? 'del' : 'add';
    const filled = !!cell;
    // The "+" shows on any thread-less line whose gutter you hover (or the drag's
    // current row) — even while another thread is open, so you can start a second
    // thread. (`!thread` already keeps it off the line that has the open thread.)
    const showPlus = filled && !thread && ((!dragging && hover && hover.side === side && hover.r === r)
      || (dragging && dragSide === side && dragTo === r));
    const bg = flash ? 'rgba(110, 139, 255, 0.22)'
      : active ? 'rgba(110, 139, 255, 0.20)'
      : (region ? 'rgba(110, 139, 255, 0.09)'
        : (isChange && filled ? `var(--diff-${tone}-bg)`
          : (isChange && !filled ? 'rgba(255,255,255,0.014)' : 'transparent')));
    const edge = active ? 'var(--accent-line)' : (isChange && filled ? `var(--diff-${tone}-edge)` : 'transparent');
    let boxShadow;
    if (active) {
      const parts = [];
      if (side === 'old') parts.push('inset 0.5px 0 0 var(--accent)');
      else parts.push('inset -0.5px 0 0 var(--accent)');
      if (isFirst) parts.push('inset 0 0.5px 0 var(--accent)');
      if (isLast) parts.push('inset 0 -0.5px 0 var(--accent)');
      boxShadow = parts.join(', ');
    } else if (region) {
      // Faint accent frame echoing the live-drag box, one notch dimmer.
      const parts = [];
      if (side === 'old') parts.push('inset 0.5px 0 0 var(--accent-line)');
      else parts.push('inset -0.5px 0 0 var(--accent-line)');
      if (regFirst) parts.push('inset 0 0.5px 0 var(--accent-line)');
      if (regLast) parts.push('inset 0 -0.5px 0 var(--accent-line)');
      boxShadow = parts.join(', ');
    } else {
      boxShadow = edge !== 'transparent' ? `inset 3px 0 0 ${edge}` : 'none';
    }
    const sign = isChange && filled ? (side === 'old' ? '−' : '+') : '';
    const handlers = filled ? {
      // While dragging the + → extend the selection on row entry.
      onMouseEnter: () => { if (dragging) setDragTo(r); },
      // Show the "+" only when the cursor is near the LEFT gutter (not over the
      // code text) — so it doesn't pop up everywhere, and hovering the code to
      // select text doesn't churn hover state (which caused the flicker).
      onMouseMove: (e) => {
        if (dragging) return;
        const near = (e.clientX - e.currentTarget.getBoundingClientRect().left) < 58;
        setHover((h) => {
          const isThis = h && h.side === side && h.r === r;
          if (near && !isThis) return { side, r };
          if (!near && isThis) return null;
          return h; // unchanged → React bails, no re-render
        });
      },
      onMouseLeave: () => { if (!dragging) setHover((h) => (h && h.side === side && h.r === r ? null : h)); },
      // ⌘/Ctrl held → open in editor immediately (on mousedown, the most reliable
      // signal in the webview), NOT a drag-to-comment.
      onMouseDown: (e) => {
        if (e.metaKey || e.ctrlKey) {
          e.preventDefault(); e.stopPropagation();
          pressRow(r, side);
          // Open at the line the user actually clicked (this cell's own new-file
          // gutter number) — NOT the row's navLine (right.n||left.n), which on a
          // change row gave the AFTER line even when you clicked the BEFORE side.
          if (onOpenInEditor && cell && cell.n != null) onOpenInEditor(card.path, cell.n);
          return;
        }
        // Plain mousedown starts a native text selection — lock it to THIS side so
        // the copy drag never spills into the other column. Threads are +-only.
        setSelectSide(side);
      },
    } : {};
    return (
      <div {...handlers} style={{ position: 'relative', display: 'grid', gridTemplateColumns: '10px 30px 12px 1fr',
        alignItems: 'start', background: bg, cursor: 'default',
        boxShadow, flex: '1 1 0', minWidth: 0,
        borderLeft: side === 'new' ? '1px solid var(--border-subtle)' : 'none',
        transition: 'background var(--dur-fast) var(--ease-soft)' }}>
        {showPlus && (() => {
          // Scale the + with the code font so it never dwarfs small text.
          const sz = Math.max(13, Math.min(28, Math.round(codeFs * 1.5)));
          return (
            <button
              onMouseDown={(e) => { e.stopPropagation(); e.preventDefault(); setDragSide(side); setDragFrom(r); setDragTo(r); }}
              title="Comment on this line (drag to select a range)"
              style={{ position: 'absolute', zIndex: 4, left: 1, top: Math.max(0, (codeFs * 1.72 - sz) / 2),
                width: sz, height: sz, borderRadius: 'var(--radius-sm)',
                cursor: dragging ? 'grabbing' : 'pointer',
                display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
                background: 'var(--accent)', border: 'none', color: '#fff',
                pointerEvents: dragging ? 'none' : 'auto',
                boxShadow: '0 1px 3px rgba(0,0,0,0.4)' }}>
              <svg width={sz} height={sz} viewBox="0 0 24 24" fill="none" stroke="currentColor"
                strokeWidth="3.6" strokeLinecap="round"><path d="M12 2v20M2 12h20" /></svg>
            </button>
          );
        })()}
        <span></span>
        <span style={{ textAlign: 'right', paddingRight: 5, color: 'var(--text-faint)',
          font: codeFs + 'px/var(--leading-code) var(--font-mono)', userSelect: 'none' }}>{cell ? cell.n : ''}</span>
        <span style={{ color: side === 'old' ? 'var(--diff-del-edge)' : 'var(--diff-add-edge)',
          userSelect: 'none', fontWeight: 600, font: codeFs + 'px/var(--leading-code) var(--font-mono)',
          boxShadow: 'inset -1px 0 0 var(--border-subtle)' }}>{sign}</span>
        <div data-side={side} style={{ overflow: 'hidden', minWidth: 0,
          font: codeFs + 'px/var(--leading-code) var(--font-mono)' }}>
          <span style={{ display: 'inline-block', boxSizing: 'border-box', minWidth: codeContentW, whiteSpace: 'pre',
            paddingLeft: 8, paddingRight: 16, tabSize: 2,
            userSelect: selectSide && selectSide !== side ? 'none' : 'text',
            transform: 'translateX(calc(var(--hs-' + side + ') * -1))' }}>
            {cell ? hl(cell.c) : ''}
          </span>
        </div>
      </div>
    );
  };

  return (
    <div onMouseUp={() => { endDrag(); setSelectSide(null); }}
      style={{ position: 'absolute', inset: 0, display: 'flex',
        background: 'var(--bg-base)', overflow: 'hidden' }}>

      {/* Queue spine — dim until hovered */}
      <ProgressSpine items={spineItems} activeId={card.id} onSelect={onSelect} />

      {/* Stage */}
      <div style={{ flex: 1, position: 'relative', display: 'flex',
        flexDirection: 'column', minWidth: 0 }}>

        {/* Top-left project / branch menu — inside the stage so it shifts right when
            the spine expands. left:84 puts it at x = rail-width(18) + 84 = 102, the
            same resting distance as the summary / pick-project screens (root, 102). */}
        <ProjectMenu project={project} base={base} target={target} onChangeProject={onChangeProject} left={84} />

        {/* Top bar — the cluster you're reviewing (prominent) + a subtle progress count.
            The cluster title is the orienting label, so it reads clearly (not dimmed).
            Doubles as the window's drag handle (overlay title bar has no chrome). */}
        <div data-tauri-drag-region style={{ display: 'flex', alignItems: 'center', justifyContent: 'center',
          gap: 10, padding: '34px 0 6px', userSelect: 'none', cursor: 'default' }}>
          {/* content is pointerEvents:none so a mousedown ANYWHERE on the bar lands on
              the drag-region element itself — reliable window drag (not a text selection
              on the title, which was why dragging "sometimes" did nothing). */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 12, minWidth: 0, pointerEvents: 'none' }}>
          <span style={{ color: 'var(--text-tertiary)', fontVariantNumeric: 'tabular-nums',
            font: 'var(--weight-medium) var(--text-base)/1 var(--font-ui)',
            letterSpacing: 'var(--tracking-wide)', opacity: 'var(--dim-rest)' }}>
            {String(index + 1).padStart(2, '0')} / {String(total).padStart(2, '0')}
          </span>
          <span style={{ width: 4, height: 4, borderRadius: 999, background: 'var(--text-faint)' }} />
          {cluster ? (
            <span style={{ display: 'inline-flex', alignItems: 'center', gap: 8, minWidth: 0 }}>
              <span style={{ width: 8, height: 8, borderRadius: 999, flex: 'none',
                background: cluster.id === '__unclustered' ? 'var(--text-faint)' : 'var(--accent)' }} />
              <span style={{ maxWidth: 560, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                font: 'var(--weight-semibold) var(--text-md)/1.2 var(--font-ui)',
                color: 'var(--text-primary)', letterSpacing: 'var(--tracking-snug)' }}>{cluster.title}</span>
              {clusterIndex && clusterIndex.of > 1 && (
                <span style={{ flex: 'none', fontVariantNumeric: 'tabular-nums',
                  font: 'var(--text-sm)/1 var(--font-ui)', color: 'var(--text-faint)',
                  letterSpacing: 'var(--tracking-wide)' }}>{clusterIndex.pos}/{clusterIndex.of}</span>
              )}
            </span>
          ) : (
            <span style={{ maxWidth: 360, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
              font: 'var(--weight-medium) var(--text-base)/1 var(--font-ui)', color: 'var(--text-secondary)',
              opacity: 'var(--dim-rest)' }}>{card.chapter}</span>
          )}
          {analysisState === 'clustering' && (
            <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5,
              font: 'var(--text-xs)/1 var(--font-ui)', color: 'var(--text-faint)' }}>
              <span style={{ width: 5, height: 5, borderRadius: 999, background: 'var(--accent)',
                animation: 'loupe-core-glow 2s var(--ease-soft) infinite' }} />
              clustering…
            </span>
          )}
          </div>
        </div>

        {/* Centered card — responsive width, deep shadow (no deck).
            #6: cards are wider now, and #4 moved the nav arrows to THIS stage's edges
            (absolute, ~16px in, ~44px wide → a 60px footprint each side). We replace
            --canvas-pad with an explicit 76px horizontal padding so the card center
            always clears that arrow footprint regardless of card width, while keeping
            the vertical breathing room. This stage is position:relative — the anchor
            for the edge-pinned arrows below. */}
        <div style={{ flex: 1, position: 'relative', display: 'flex', alignItems: 'center',
          justifyContent: 'center', padding: '24px 76px', minHeight: 0,
          // Isolate the card subtree's layout so the spine's width animation (which
          // re-centers the card) doesn't re-evaluate the diff virtualization against the
          // whole document each frame. `layout` only (NOT paint) so the card's big
          // ambient shadow isn't clipped.
          contain: 'layout' }}>

          {/* #4: side navigation arrows — pinned to the CANVAS edges (this stage
              container, position:relative), vertically centered, card-width-agnostic. */}
          <NavArrow side="left" d={chevL} disabled={!hasPrev} onClick={onPrev} label="Previous card" />
          <NavArrow side="right" d={chevR} disabled={false} onClick={onNext} label="Next card" />

          {/* The card grows with its CONTENT (code length), capped at maxHeight:100% (the
              card-area height minus its 24px padding). Short diffs => short card (centered);
              tall diffs => fills the available height and the inner diff scroller scrolls.
              NOT force-stretched — a 28-line diff must not become a full-height empty card. */}
          <div style={{ position: 'relative', width: cardW, maxWidth: '100%',
            maxHeight: '100%', display: 'flex' }}>

          {/* Cluster progress (design system) — a soft luminous line along the card's
              TOP edge that grows left→right with the position in the cluster (pos/of),
              feathered at both ends so it glows out of the edge instead of cutting off. */}
          {clusterIndex && clusterIndex.of >= 1 && (
            <div aria-hidden="true" style={{ position: 'absolute', top: 1,
              left: 'var(--radius-card)', right: 'var(--radius-card)', height: 1,
              zIndex: 5, pointerEvents: 'none' }}>
              <div style={{ height: '100%', borderRadius: 999,
                width: (Math.max(0, Math.min(1, clusterIndex.pos / clusterIndex.of)) * 100) + '%',
                background: 'linear-gradient(90deg, rgba(231,236,245,0) 0%, rgba(231,236,245,0.4) 35%, rgba(231,236,245,0.8) 90%, rgba(231,236,245,0) 100%)',
                boxShadow: '0 0 7px 0 rgba(200,220,255,0.46)',
                transition: 'width 0.6s var(--ease-out)' }} />
            </div>
          )}

          <div key={card.id} style={{
            position: 'relative', zIndex: 3,
            width: '100%', maxHeight: '100%', display: 'flex', flexDirection: 'column',
            // Lit-from-above gradient (subtle sheen at the top, settling into shadow
            // at the bottom) over the base surface — gives the card a premium, raised feel.
            background: 'linear-gradient(180deg, rgba(255,255,255,0.05) 0%, rgba(255,255,255,0) 15%, rgba(0,0,0,0.05) 100%), var(--surface-card)',
            border: '1px solid var(--border-subtle)',
            borderRadius: 'var(--radius-card)',
            // Layered drop shadows (tight contact → wide ambient) for float, plus a
            // beveled rim: bright top sheen + dark inner bottom edge.
            boxShadow: [
              '0 1px 2px rgba(0,0,0,0.55)',
              '0 5px 12px rgba(0,0,0,0.45)',
              '0 20px 46px rgba(0,0,0,0.5)',
              '0 50px 110px rgba(0,0,0,0.45)',
              'inset 0 1px 0 rgba(255,255,255,0.09)',
              'inset 0 -1px 0 rgba(0,0,0,0.35)',
              'inset 0 0 0 1px rgba(255,255,255,0.02)',
            ].join(', '),
            animation: `loupe-card-in var(--dur-slow) var(--ease-out)`,
            ['--enter-x']: `${dir * 36}px`, overflow: 'hidden',
          }}>
            {/* Card header — the cluster name lives in the top bar now, not on the card. */}
            <div style={{ padding: '22px var(--gutter-card) 18px',
              borderBottom: '1px solid var(--border-subtle)' }}>
              {/* fixed-height row so the 22px "Passed" badge appearing doesn't make the
                  header taller (which slightly shrank the card) — reserve its height always. */}
              <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 12, minHeight: 22 }}>
                <span style={{ font: 'var(--weight-semibold) var(--text-base)/1 var(--font-mono)',
                  color: 'var(--text-primary)', letterSpacing: 'var(--tracking-snug)' }}>{card.symbol}</span>
                {verdict === 'pass' && (
                  <button onClick={onUnpass} title="패스 취소 (⌘Z)"
                    onMouseEnter={() => setPassHover(true)} onMouseLeave={() => setPassHover(false)}
                    style={{ display: 'inline-flex', alignItems: 'center', gap: 5, flex: 'none', height: 22,
                      padding: '0 9px', borderRadius: 'var(--radius-pill)', cursor: 'pointer',
                      background: passHover ? 'var(--flag-dim)' : 'var(--pass-dim)',
                      border: `1px solid ${passHover ? 'var(--flag-line)' : 'var(--pass-line)'}`,
                      color: passHover ? 'var(--flag)' : 'var(--pass)',
                      font: 'var(--weight-medium) var(--text-xs)/1 var(--font-ui)',
                      transition: 'var(--t-hover)' }}>
                    {/* icon swaps check→× (same 13px width) but the TEXT stays "Passed" so
                        the badge never changes width — no header reflow / card-size jitter. */}
                    <Ico d={passHover ? 'M18 6 6 18M6 6l12 12' : check} w={13} />Passed</button>)}
                <span onClick={() => setPathFull((v) => !v)} title={pathFull ? '경로 접기' : card.path}
                  style={{ marginLeft: 'auto', flex: 'none', maxWidth: '55%', cursor: 'pointer',
                    font: 'var(--text-sm)/1 var(--font-mono)', color: 'var(--text-tertiary)',
                    whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
                    direction: 'rtl', textAlign: 'right' }}>
                  {pathFull ? card.path : middlePath(card.path)}</span>
              </div>
              <div style={{ font: 'var(--text-sm)/var(--leading-normal) var(--font-ui)',
                color: 'var(--text-secondary)', textWrap: 'pretty' }}>{card.aiSummary || card.summary}</div>
            </div>

            {/* hidden monospace sizer — measures one char at the current size */}
            <span ref={charRef} aria-hidden="true" style={{ position: 'absolute', visibility: 'hidden',
              pointerEvents: 'none', whiteSpace: 'pre', font: codeFs + 'px/1 var(--font-mono)' }}>{'0'.repeat(50)}</span>

            {/* Split diff — each side (BEFORE / AFTER) scrolls left/right via the
                bottom hbar, which writes --hs-old / --hs-new (read by each line). */}
            <div ref={diffRef} onScroll={onDiffScroll} onMouseLeave={() => { if (!dragging) setHover(null); }}
              style={{ overflowY: 'auto', overflowX: 'hidden', flex: 1, userSelect: 'none',
              background: 'var(--surface-inset)', '--hs-old': '0px', '--hs-new': '0px' }}>

              {/* sticky column headers */}
              <div ref={headerRef} style={{ display: 'flex', position: 'sticky', top: 0, zIndex: 2,
                background: 'var(--surface-inset)',
                borderBottom: '1px solid var(--border-subtle)' }}>
                <div style={{ flex: '1 1 0', minWidth: 0, padding: '7px 0 7px 10px',
                  font: 'var(--weight-medium) var(--text-xs)/1 var(--font-ui)', letterSpacing: 'var(--tracking-wide)',
                  textTransform: 'uppercase', color: 'var(--text-tertiary)' }}>Before</div>
                <div style={{ flex: '1 1 0', minWidth: 0, padding: '7px 0 7px 10px',
                  font: 'var(--weight-medium) var(--text-xs)/1 var(--font-ui)', letterSpacing: 'var(--tracking-wide)',
                  textTransform: 'uppercase', color: 'var(--text-tertiary)', borderLeft: '1px solid var(--border-subtle)' }}>After</div>
              </div>

              {/* #7: the rows column + its virtualization spacers all paint the
                  same --surface-inset as the scroller, so any sub-pixel gap between
                  an estimated and a measured row height (or a spacer that lands a
                  fraction off during a fast flick) shows inset, never the card
                  surface beneath — keeping the diff visually seamless while scrolling. */}
              <div style={{ padding: '8px 0', background: 'var(--surface-inset)' }}>
              <div style={{ height: topPad, background: 'var(--surface-inset)' }} />
              {display.slice(startIdx, endIdx).map((item, li) => {
                const gi = startIdx + li; // global index into `display` (for measurement)
                if (item.type === 'fold') {
                  return (
                    <div key={'fold-' + item.key} ref={measure(gi)} onClick={() => setExpanded((s) => new Set(s).add(item.key))}
                      style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '8px 18px', cursor: 'pointer',
                        color: 'var(--text-faint)',
                        transition: 'color var(--dur-fast) var(--ease-soft)' }}
                      onMouseEnter={(e) => { e.currentTarget.style.color = 'var(--text-secondary)'; }}
                      onMouseLeave={(e) => { e.currentTarget.style.color = 'var(--text-faint)'; }}>
                      <span style={{ flex: 1, height: 1, background: 'var(--border-subtle)' }} />
                      <span style={{ font: 'var(--text-xs)/1 var(--font-ui)', letterSpacing: 'var(--tracking-wide)', whiteSpace: 'nowrap' }}>
                        ⋯ {item.count} unchanged lines
                      </span>
                      <span style={{ flex: 1, height: 1, background: 'var(--border-subtle)' }} />
                    </div>
                  );
                }
                const row = item.row;
                const r = item.r;
                const thread = threadByRow[r];
                const active = inRange(r);
                const lo = dragging ? Math.min(dragFrom, dragTo) : -1;
                const hi = dragging ? Math.max(dragFrom, dragTo) : -1;
                const isFirst = active && r === lo;
                const isLast = active && r === hi;
                // #3 persistent region tint (suppressed while actively dragging so
                // the live selection reads cleanly). regFirst/regLast draw the box's
                // top/bottom edge even across disjoint regions.
                const regionOld = !dragging && openRegionRows.has('old:' + r);
                const regionNew = !dragging && openRegionRows.has('new:' + r);
                const regFirstOld = regionOld && !openRegionRows.has('old:' + (r - 1));
                const regLastOld = regionOld && !openRegionRows.has('old:' + (r + 1));
                const regFirstNew = regionNew && !openRegionRows.has('new:' + (r - 1));
                const regLastNew = regionNew && !openRegionRows.has('new:' + (r + 1));
                // New-file line for ⌘-click → open-in-editor (after side; deletion → before).
                const navLine = (row.right && row.right.n) || (row.left && row.left.n);
                // A row's measured height must include any open/collapsed thread that
                // renders directly beneath it, so the measuring ref wraps both. The
                // wrapper is a plain block (no flex/grid/height) — it does not alter
                // layout, it only gives offsetHeight something to read.
                return (
                  <div key={r} ref={measure(gi)}>
                    {/* Half is invoked as a FUNCTION (not <Half/>) so it inlines into this
                        tree. As a component its identity changes every render (defined in
                        the body), which made React remount every visible row on each scroll
                        frame — the source of the scroll stutter. Inlined, rows reconcile. */}
                    <div data-line={r} style={{ display: 'flex', minWidth: 0 }}>
                      {Half({ cell: row.left, kind: row.kind, side: 'old', r,
                        active: active && dragSide === 'old', isFirst: isFirst && dragSide === 'old', isLast: isLast && dragSide === 'old', thread,
                        region: regionOld, regFirst: regFirstOld, regLast: regLastOld, navLine,
                        flash: !!(flashRow && flashRow.r === r && flashRow.side === 'old') })}
                      {Half({ cell: row.right, kind: row.kind, side: 'new', r,
                        active: active && dragSide === 'new', isFirst: isFirst && dragSide === 'new', isLast: isLast && dragSide === 'new', thread,
                        region: regionNew, regFirst: regFirstNew, regLast: regLastNew, navLine,
                        flash: !!(flashRow && flashRow.r === r && flashRow.side === 'new') })}
                    </div>
                    {thread && thread.open && (
                      <div ref={(el) => { if (el) threadEls.current[thread.id] = el; }}
                        style={{ padding: '8px 28px 12px 52px',
                          // The sticky column header is opaque (--surface-inset) and sits at
                          // zIndex 2. Without its own stacking context an open thread paints
                          // at z-auto (0) and the header overpaints its top ~12px (rounded
                          // corner + the action buttons) whenever a re-layout slides the
                          // thread back under the header (e.g. the measure-driven re-render
                          // below, or a clamped scrollTop). Giving the wrapper its own layer
                          // ABOVE the header guarantees the header can never clip the thread,
                          // independent of scroll timing. scrollMarginTop reserves the header's
                          // height so scroll-into-view / clamp(scrollTop=0) still clear it.
                          position: 'relative', zIndex: 3,
                          scrollMarginTop: (headerH + 8) + 'px' }}>
                        <Thread messages={thread.messages} resolved={thread.resolved}
                          pending={thread.pending}
                          model={thread.model || 'sonnet'}
                          onSetModel={onSetThreadModel ? (m) => onSetThreadModel(thread.id, m) : undefined}
                          collapsed={false} onToggle={() => onOpenLine(thread.side || 'old', r)}
                          onResolve={() => onResolve(thread.id)} onSend={(t, kind) => onSend(thread.id, t, kind)}
                          onDelete={onDeleteThread ? () => onDeleteThread(thread.id) : undefined}
                          onNavigateCard={onNavigateCard} />
                      </div>
                    )}
                    {thread && !thread.open && (() => {
                      // Fixed spot: right next to the line-number gutter (the code
                      // column), the SAME for every thread. Preview region on hover.
                      const onNewSide = (thread.side || 'old') === 'new';
                      const hov = {
                        onMouseEnter: () => setHoverThreadId(thread.id),
                        onMouseLeave: () => setHoverThreadId((id) => (id === thread.id ? null : id)),
                      };
                      // Only the AFTER (new) side badge sits at 85px; the before side
                      // stays at the code column (52).
                      const badge = (sd) => (
                        <div style={{ flex: '1 1 0', minWidth: 0,
                          padding: `6px 0 8px ${sd === 'new' ? 95 : 52}px` }} {...hov}>
                          <Thread messages={thread.messages} resolved={thread.resolved}
                            unread={unreadThreads.has(thread.id)}
                            collapsed onToggle={() => onOpenLine(sd, r)} />
                        </div>
                      );
                      const onNew = onNewSide;
                      return (
                        <div style={{ display: 'flex', minWidth: 0 }}>
                          {onNew ? <div style={{ flex: '1 1 0', minWidth: 0 }} /> : badge('old')}
                          {onNew ? badge('new') : <div style={{ flex: '1 1 0', minWidth: 0 }} />}
                        </div>
                      );
                    })()}
                  </div>
                );
              })}
              <div style={{ height: bottomPad, background: 'var(--surface-inset)' }} />
              </div>

              {/* horizontal scrollbars — one per side, pinned at the bottom. These
                  are the ONLY horizontal controls: each onScroll writes its side's
                  CSS variable, which every code line of that side translates by. */}
              {/* zIndex 4: stays ABOVE open threads (now zIndex 3 so the header
                  can't clip them). Threads must paint UNDER this bottom scrollbar,
                  same as before the z bump, so the h-bars remain visible/usable. */}
              <div style={{ display: 'flex', position: 'sticky', bottom: 0, zIndex: 4,
                background: 'var(--surface-inset)' }}>
                <div style={{ flex: '1 1 0', minWidth: 0, display: 'flex' }}>
                  <div style={{ width: GUTTER, flex: 'none' }} />
                  <div className="loupe-hbar" ref={hbarOldRef}
                    onScroll={(e) => { if (diffRef.current) diffRef.current.style.setProperty('--hs-old', e.target.scrollLeft + 'px'); }}
                    style={{ flex: '1 1 0', minWidth: 0, overflowX: 'auto', overflowY: 'hidden' }}>
                    <div style={{ width: codeContentW, height: 1 }} />
                  </div>
                </div>
                <div style={{ flex: '1 1 0', minWidth: 0, display: 'flex' }}>
                  <div style={{ width: GUTTER, flex: 'none' }} />
                  <div className="loupe-hbar" ref={hbarNewRef}
                    onScroll={(e) => { if (diffRef.current) diffRef.current.style.setProperty('--hs-new', e.target.scrollLeft + 'px'); }}
                    style={{ flex: '1 1 0', minWidth: 0, overflowX: 'auto', overflowY: 'hidden' }}>
                    <div style={{ width: codeContentW, height: 1 }} />
                  </div>
                </div>
              </div>
            </div>
          </div>
          </div>
        </div>

        {/* Bottom: hints + unresolved on the left, verdict actions on the RIGHT */}
        <div style={{ display: 'flex', alignItems: 'center',
          padding: '0 var(--canvas-pad) 30px' }}>

          <div style={{ display: 'flex', alignItems: 'center', gap: 16 }}>
            <TextSizeMenu size={codeFontSize} effective={autoFs} onChange={setFont} />
            <OpenInEditorMenu />
            <span style={{ width: 1, height: 16, background: 'var(--border-subtle)' }} />
            <div style={{ display: 'flex', alignItems: 'center', gap: 18, opacity: 'var(--dim-rest)' }}>
              <KeyHint keys={['←', '→']} label="Move" size="sm" />
              <KeyHint keys="+" label="to comment" size="sm" tone="accent" />
              <KeyHint keys="⌘Z" label="undo pass" size="sm" />
            </div>
          </div>

          <div style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 14 }}>
            <Button variant="pass" kbd="Space" icon={<Ico d={check} />} onClick={onPass}>Pass</Button>
          </div>
        </div>
      </div>
    </div>
  );
}
