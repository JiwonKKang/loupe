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

// Memoize syntax highlighting per source string: the same line text re-tokenizes
// to the same keyed span array, so windowing (mount/unmount as you scroll) never
// pays to re-highlight a line it has already seen. Bounded to avoid unbounded growth.
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

export default function ReviewScreen(props) {
  const {
    card, index, total, dir, project, base, target, onChangeProject, unresolved,
    cluster, clusterIndex, analysisState,
    spineItems, onSelect,
    verdict, flagged, hasPrev, hasNext,
    onPass, onPrev, onNext, onJumpUnresolved,
    threads, onOpenLine, onResolve, onSend,
  } = props;

  // Is this a JIT definition card (⑨ fills the overview; ⑧ leaves the branch placeholder)?
  const isDefinition = card.kind === 'definition';

  // ---- Build aligned split rows (before | after) from the unified line list.
  const rows = React.useMemo(() => {
    const start = (card.lines[0] && card.lines[0].n) || 1;
    let oldNo = start, newNo = start;
    const out = [];
    let pendDel = [], pendAdd = [];
    const flush = () => {
      const n = Math.max(pendDel.length, pendAdd.length);
      for (let i = 0; i < n; i++) out.push({ kind: 'change', left: pendDel[i] || null, right: pendAdd[i] || null });
      pendDel = []; pendAdd = [];
    };
    card.lines.forEach((ln) => {
      if (ln.t === 'ctx') { flush(); out.push({ kind: 'ctx', left: { n: oldNo++, c: ln.c }, right: { n: newNo++, c: ln.c } }); }
      else if (ln.t === 'del') pendDel.push({ n: oldNo++, c: ln.c });
      else if (ln.t === 'add') pendAdd.push({ n: newNo++, c: ln.c });
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
  const inRange = (i) => dragging &&
    i >= Math.min(dragFrom, dragTo) && i <= Math.max(dragFrom, dragTo);
  const endDrag = () => {
    if (dragFrom == null) return;
    const side = dragSide, f = dragFrom;
    setDragSide(null); setDragFrom(null); setDragTo(null);
    onOpenLine(side, f);
  };

  // thread lookup keyed by row
  const threadByRow = React.useMemo(() => {
    const m = {};
    threads.forEach((t) => { m[t.lineN] = t; });
    return m;
  }, [threads]);

  // ---- Fold unchanged context so big functions fit at a glance ----
  // Keep CONTEXT lines of context around each change; collapse the rest into
  // a thin "⋯ N unchanged lines" divider the reviewer can expand. Font stays
  // large; only the changes (the point of the card) compete for the eye.
  const [expanded, setExpanded] = React.useState(() => new Set());
  React.useEffect(() => { setExpanded(new Set()); }, [card.id]);
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

  // ---- No-wrap horizontal scroll ----
  // Lines no longer wrap; the diff scrolls left/right. To keep both columns
  // equal (centered divider) we give each side a min-width that fits the
  // longest line, measured in real monospace px for the current font size.
  const maxChars = React.useMemo(() => {
    let m = 0;
    rows.forEach((r) => {
      if (r.left && r.left.c) m = Math.max(m, r.left.c.length);
      if (r.right && r.right.c) m = Math.max(m, r.right.c.length);
    });
    return Math.max(m, 24);
  }, [card.id]);
  const charRef = React.useRef(null);
  const [chPx, setChPx] = React.useState(8.4);
  React.useLayoutEffect(() => {
    if (charRef.current) setChPx(charRef.current.getBoundingClientRect().width / 50);
  }, [codeFs]);
  const GUTTER = 73; // line-number gutter + sign + code padding
  // Each side's code area is its OWN horizontal scroller; short lines get a
  // min-width so every line on a side scrolls by the same amount, and a
  // capture-scroll handler keeps all lines of one side in lock-step (so the
  // BEFORE column and the AFTER column scroll independently of each other).
  const codeContentW = Math.ceil(maxChars * chPx + 28);
  // Responsive card width: grows with the actual code length up to a max, so
  // short changes get a compact card and long ones widen toward the cap.
  const cardW = Math.max(560, Math.min(1140, (62 + codeContentW) * 2 + 6));
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
  // rAF-throttled vertical scroll: only the OUTER container scrolls vertically
  // (horizontal scrolling is the bottom hbars writing a CSS variable).
  const onDiffScroll = (e) => {
    if (e.currentTarget !== diffRef.current) return;
    const top = e.currentTarget.scrollTop;
    if (scrollRaf.current) return;
    scrollRaf.current = requestAnimationFrame(() => {
      scrollRaf.current = 0;
      setScrollTop(top);
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
    // card.id (not just isDefinition): the <div key={card.id}> wrapper remounts
    // the diff container on every card change, so diffRef points at a fresh node
    // each time — we must re-observe it (and re-measure clientHeight) per card.
  }, [card.id, isDefinition]);
  // Native non-passive wheel listener (see note on hbarOldRef above): horizontal
  // wheel → push the hovered side's bottom hbar, whose onScroll sets the CSS var.
  // Re-bind per card (the diff node is fresh after each key={card.id} remount).
  React.useEffect(() => {
    const el = diffRef.current;
    if (!el) return;
    const onWheel = (e) => {
      if (Math.abs(e.deltaX) <= Math.abs(e.deltaY)) return; // vertical → container scrolls
      const half = e.target.closest && e.target.closest('[data-side]');
      const side = half && half.getAttribute('data-side');
      const bar = side === 'new' ? hbarNewRef.current : hbarOldRef.current;
      if (bar) { bar.scrollLeft += e.deltaX; e.preventDefault(); }
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => el.removeEventListener('wheel', onWheel);
  }, [card.id, isDefinition]);
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

  const OVERPX = 600; // overscan in px above/below the viewport
  const startIdx = N === 0 ? 0 : firstBelow(scrollTop - OVERPX);
  // At least one item past startIdx so a single tall row (e.g. an open thread)
  // taller than the viewport still renders instead of collapsing to an empty slice.
  const endIdx = N === 0 ? 0 : Math.max(startIdx + 1, firstAtOrAfter(scrollTop + viewportH + OVERPX));
  const topPad = offsets[startIdx] || 0;
  const bottomPad = Math.max(0, totalH - (offsets[endIdx] || totalH));

  // Measuring ref factory: stores the item's offsetHeight at its global index and
  // bumps measureTick only when the height actually changed (> 0.5px). This guard
  // is what makes measurement converge instead of looping: once a row's stored
  // height matches its rendered height (within 0.5px) the callback is a no-op, so
  // it triggers no further render — measurement reaches a fixed point.
  const measure = (gi) => (el) => {
    if (!el) return;
    const h = el.offsetHeight;
    if (Math.abs((rowH.current[gi] || 0) - h) > 0.5) {
      rowH.current[gi] = h;
      setMeasureTick((t) => t + 1);
    }
  };

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

  // bare thin chevron on the card's side, vertically centered (no box).
  const NavArrow = ({ side, d, disabled, onClick, label, offset }) => (
    <button onClick={disabled ? undefined : onClick} aria-label={label} title={label} disabled={disabled}
      style={{ position: 'absolute', zIndex: 3, top: '50%', transform: 'translateY(-50%)',
        [side]: offset, width: 30, height: 60, padding: 0,
        display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
        background: 'transparent', border: 'none',
        color: 'var(--text-secondary)', cursor: disabled ? 'default' : 'pointer',
        opacity: disabled ? 0.12 : 'var(--dim-rest)', transition: 'var(--t-dim), var(--t-hover)' }}
      onMouseEnter={(e) => { if (!disabled) { e.currentTarget.style.opacity = 1; e.currentTarget.style.color = 'var(--text-primary)'; } }}
      onMouseLeave={(e) => { if (!disabled) { e.currentTarget.style.opacity = 'var(--dim-rest)'; e.currentTarget.style.color = 'var(--text-secondary)'; } }}>
      <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor"
        strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round"><path d={d} /></svg>
    </button>
  );

  // one side (old/new) of a split row. Selection highlight is row-wide; the
  // + affordance and a collapsed thread's badge are per-side (this column).
  const Half = ({ cell, kind, side, r, active, isFirst, isLast, thread }) => {
    const isChange = kind === 'change';
    const tone = side === 'old' ? 'del' : 'add';
    const filled = !!cell;
    const showPlus = filled && !thread && ((!dragging && hover && hover.side === side && hover.r === r)
      || (dragging && dragSide === side && dragTo === r));
    const bg = active ? 'rgba(110, 139, 255, 0.20)'
      : (isChange && filled ? `var(--diff-${tone}-bg)`
        : (isChange && !filled ? 'rgba(255,255,255,0.014)' : 'transparent'));
    const edge = active ? 'var(--accent-line)' : (isChange && filled ? `var(--diff-${tone}-edge)` : 'transparent');
    let boxShadow;
    if (active) {
      const parts = [];
      if (side === 'old') parts.push('inset 0.5px 0 0 var(--accent)');
      else parts.push('inset -0.5px 0 0 var(--accent)');
      if (isFirst) parts.push('inset 0 0.5px 0 var(--accent)');
      if (isLast) parts.push('inset 0 -0.5px 0 var(--accent)');
      boxShadow = parts.join(', ');
    } else {
      boxShadow = edge !== 'transparent' ? `inset 3px 0 0 ${edge}` : 'none';
    }
    const sign = isChange && filled ? (side === 'old' ? '−' : '+') : '';
    const handlers = filled ? {
      onMouseEnter: () => { if (dragging) { setDragTo(r); } else { setHover({ side, r }); } },
      onMouseLeave: () => { if (!dragging) setHover((h) => (h && h.side === side && h.r === r ? null : h)); },
      onMouseDown: (e) => { if (!thread) { e.preventDefault(); setDragSide(side); setDragFrom(r); setDragTo(r); } },
    } : {};
    return (
      <div {...handlers} style={{ position: 'relative', display: 'grid', gridTemplateColumns: '20px 30px 12px 1fr',
        alignItems: 'start', background: bg, cursor: 'default',
        boxShadow, flex: '1 1 0', minWidth: 0,
        borderLeft: side === 'new' ? '1px solid var(--border-subtle)' : 'none',
        transition: 'background var(--dur-fast) var(--ease-soft)' }}>
        {showPlus && (
          <button
            onMouseDown={(e) => { e.stopPropagation(); e.preventDefault(); setDragSide(side); setDragFrom(r); setDragTo(r); }}
            title="Comment on this line (drag to select a range)"
            style={{ position: 'absolute', zIndex: 4, left: 1, top: Math.max(0, (codeFs * 1.72 - 20) / 2),
              width: 20, height: 20, borderRadius: 'var(--radius-sm)',
              cursor: dragging ? 'grabbing' : 'pointer',
              display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
              background: 'var(--accent)', border: 'none', color: '#fff',
              pointerEvents: dragging ? 'none' : 'auto',
              boxShadow: '0 1px 3px rgba(0,0,0,0.4)' }}>
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor"
              strokeWidth="3.8" strokeLinecap="round"><path d="M12 4v16M4 12h16" /></svg>
          </button>
        )}
        <span></span>
        <span style={{ textAlign: 'right', paddingRight: 8, color: 'var(--text-faint)',
          font: codeFs + 'px/var(--leading-code) var(--font-mono)', userSelect: 'none' }}>{cell ? cell.n : ''}</span>
        <span style={{ color: side === 'old' ? 'var(--diff-del-edge)' : 'var(--diff-add-edge)',
          userSelect: 'none', fontWeight: 600, font: codeFs + 'px/var(--leading-code) var(--font-mono)',
          boxShadow: 'inset -1px 0 0 var(--border-subtle)' }}>{sign}</span>
        <div data-side={side} style={{ overflow: 'hidden', minWidth: 0,
          font: codeFs + 'px/var(--leading-code) var(--font-mono)' }}>
          <span style={{ display: 'inline-block', minWidth: codeContentW, whiteSpace: 'pre',
            paddingLeft: 11, paddingRight: 16, tabSize: 2,
            transform: 'translateX(calc(var(--hs-' + side + ') * -1))' }}>
            {cell ? hl(cell.c) : ''}
          </span>
        </div>
      </div>
    );
  };

  return (
    <div onMouseUp={endDrag}
      style={{ position: 'absolute', inset: 0, display: 'flex',
        background: 'var(--bg-base)', overflow: 'hidden' }}>

      {/* Queue spine — dim until hovered */}
      <ProgressSpine items={spineItems} activeId={card.id} onSelect={onSelect} />

      {/* Stage */}
      <div style={{ flex: 1, position: 'relative', display: 'flex',
        flexDirection: 'column', minWidth: 0 }}>

        {/* Top-left project / branch menu — switch projects anytime */}
        <ProjectMenu project={project} base={base} target={target} onChangeProject={onChangeProject} />

        {/* Minimal top bar — progress · chapter */}
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center',
          gap: 12, padding: '26px 0 0', opacity: 'var(--dim-rest)',
          font: 'var(--weight-medium) var(--text-sm)/1 var(--font-ui)',
          color: 'var(--text-secondary)', letterSpacing: 'var(--tracking-wide)' }}>
          <span style={{ color: 'var(--text-tertiary)', fontVariantNumeric: 'tabular-nums' }}>
            {String(index + 1).padStart(2, '0')} / {String(total).padStart(2, '0')}
          </span>
          <span style={{ width: 3, height: 3, borderRadius: 999, background: 'var(--text-faint)' }} />
          <span style={{ maxWidth: 320, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {cluster ? cluster.title : card.chapter}
          </span>
          {analysisState === 'clustering' && (
            <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5,
              font: 'var(--text-xs)/1 var(--font-ui)', color: 'var(--text-faint)' }}>
              <span style={{ width: 5, height: 5, borderRadius: 999, background: 'var(--accent)',
                animation: 'loupe-core-glow 2s var(--ease-soft) infinite' }} />
              clustering…
            </span>
          )}
        </div>

        {/* Centered card — responsive width, deep shadow (no deck) */}
        <div style={{ flex: 1, display: 'flex', alignItems: 'center',
          justifyContent: 'center', padding: '24px var(--canvas-pad)', minHeight: 0 }}>
          <div style={{ position: 'relative', width: cardW, maxWidth: '100%',
            maxHeight: '100%', display: 'flex' }}>

            {/* side navigation arrows — prev (left) / next (right), vertically centered on the card */}
            <NavArrow side="left" d={chevL} disabled={!hasPrev} onClick={onPrev} label="Previous card" offset={-56} />
            <NavArrow side="right" d={chevR} disabled={false} onClick={onNext} label="Next card" offset={-56} />

          <div key={card.id} style={{
            position: 'relative', zIndex: 3,
            width: '100%', maxHeight: '100%', display: 'flex', flexDirection: 'column',
            background: 'var(--surface-card)', border: '1px solid var(--border-subtle)',
            borderRadius: 'var(--radius-card)',
            boxShadow: '0 2px 5px rgba(0,0,0,0.45), 0 20px 50px rgba(0,0,0,0.5), 0 46px 100px rgba(0,0,0,0.45), inset 0 1px 0 rgba(255,255,255,0.05), inset 0 0 0 1px var(--border-subtle)',
            animation: `loupe-card-in var(--dur-slow) var(--ease-out)`,
            ['--enter-x']: `${dir * 36}px`, overflow: 'hidden',
          }}>
            {/* Card header */}
            <div style={{ padding: '22px var(--gutter-card) 18px',
              borderBottom: '1px solid var(--border-subtle)' }}>
              {/* Cluster band — cluster title + position in this cluster (⑧). The kind
                  badge ("Flow/Contract/…") was removed — it read as clutter on the card. */}
              {cluster && (() => {
                const muted = cluster.id === '__unclustered';
                return (
                  <div style={{ display: 'flex', alignItems: 'center', gap: 9, marginBottom: 12,
                    opacity: muted ? 0.7 : 1 }}>
                    <span style={{ minWidth: 0, font: 'var(--weight-medium) var(--text-sm)/1.2 var(--font-ui)',
                      color: 'var(--text-secondary)', overflow: 'hidden', textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap' }}>{cluster.title}</span>
                    {clusterIndex && clusterIndex.of > 1 && (
                      <span style={{ marginLeft: 'auto', flex: 'none', fontVariantNumeric: 'tabular-nums',
                        font: 'var(--text-xs)/1 var(--font-ui)', color: 'var(--text-faint)',
                        letterSpacing: 'var(--tracking-wide)' }}>
                        {clusterIndex.pos} / {clusterIndex.of} in this cluster
                      </span>
                    )}
                  </div>
                );
              })()}
              <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 12 }}>
                <span style={{ font: 'var(--weight-semibold) var(--text-base)/1 var(--font-mono)',
                  color: 'var(--text-primary)', letterSpacing: 'var(--tracking-snug)' }}>{card.symbol}</span>
                {verdict === 'pass' && (
                  <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5, height: 22,
                    padding: '0 9px', borderRadius: 'var(--radius-pill)', background: 'var(--pass-dim)',
                    border: '1px solid var(--pass-line)', color: 'var(--pass)',
                    font: 'var(--weight-medium) var(--text-xs)/1 var(--font-ui)' }}>
                    <Ico d={check} w={13} />Passed</span>)}
                <span style={{ marginLeft: 'auto', font: 'var(--text-sm)/1 var(--font-mono)',
                  color: 'var(--text-tertiary)' }}>{card.path}</span>
              </div>
              <div style={{ font: 'var(--text-sm)/var(--leading-normal) var(--font-ui)',
                color: 'var(--text-secondary)', textWrap: 'pretty' }}>{card.aiSummary || card.summary}</div>
            </div>

            {/* JIT definition (⑨) — an overview panel instead of a diff. ⑧ leaves the
                placeholder; the engine's DefinitionOverview fills it in ⑨. */}
            {isDefinition ? (
              <div style={{ flex: 1, overflowY: 'auto', padding: '20px var(--gutter-card)',
                background: 'var(--surface-inset)', display: 'flex', flexDirection: 'column', gap: 12 }}>
                <div style={{ font: 'var(--weight-medium) var(--text-xs)/1 var(--font-ui)',
                  letterSpacing: 'var(--tracking-caps)', textTransform: 'uppercase',
                  color: 'var(--text-tertiary)' }}>Definition overview</div>
                <div style={{ font: 'var(--text-sm)/1.5 var(--font-ui)', color: 'var(--text-faint)' }}>
                  Overview of {card.symbol} — shown here just before it is first used.
                </div>
              </div>
            ) : (
            <React.Fragment>
            {/* hidden monospace sizer — measures one char at the current size */}
            <span ref={charRef} aria-hidden="true" style={{ position: 'absolute', visibility: 'hidden',
              pointerEvents: 'none', whiteSpace: 'pre', font: codeFs + 'px/1 var(--font-mono)' }}>{'0'.repeat(50)}</span>

            {/* Split diff — each side (BEFORE / AFTER) scrolls left/right via the
                bottom hbar, which writes --hs-old / --hs-new (read by each line). */}
            <div ref={diffRef} onScroll={onDiffScroll} onMouseLeave={() => { if (!dragging) setHover(null); }}
              style={{ overflowY: 'auto', overflowX: 'hidden', flex: 1, userSelect: 'none',
              background: 'var(--surface-inset)', '--hs-old': '0px', '--hs-new': '0px' }}>

              {/* sticky column headers */}
              <div style={{ display: 'flex', position: 'sticky', top: 0, zIndex: 2,
                background: 'var(--surface-inset)',
                borderBottom: '1px solid var(--border-subtle)' }}>
                <div style={{ flex: '1 1 0', minWidth: 0, padding: '7px 0 7px 73px',
                  font: 'var(--weight-medium) var(--text-xs)/1 var(--font-ui)', letterSpacing: 'var(--tracking-wide)',
                  textTransform: 'uppercase', color: 'var(--text-tertiary)' }}>Before</div>
                <div style={{ flex: '1 1 0', minWidth: 0, padding: '7px 0 7px 73px',
                  font: 'var(--weight-medium) var(--text-xs)/1 var(--font-ui)', letterSpacing: 'var(--tracking-wide)',
                  textTransform: 'uppercase', color: 'var(--text-tertiary)', borderLeft: '1px solid var(--border-subtle)' }}>After</div>
              </div>

              <div style={{ padding: '8px 0' }}>
              <div style={{ height: topPad }} />
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
                // A row's measured height must include any open/collapsed thread that
                // renders directly beneath it, so the measuring ref wraps both. The
                // wrapper is a plain block (no flex/grid/height) — it does not alter
                // layout, it only gives offsetHeight something to read.
                return (
                  <div key={r} ref={measure(gi)}>
                    <div data-line={r} style={{ display: 'flex', minWidth: 0 }}>
                      <Half cell={row.left} kind={row.kind} side="old" r={r}
                        active={active} isFirst={isFirst} isLast={isLast} thread={thread} />
                      <Half cell={row.right} kind={row.kind} side="new" r={r}
                        active={active} isFirst={isFirst} isLast={isLast} thread={thread} />
                    </div>
                    {thread && thread.open && (
                      <div style={{ padding: '8px 28px 12px 52px' }}>
                        <Thread messages={thread.messages} resolved={thread.resolved}
                          collapsed={false} onToggle={() => onOpenLine(thread.side || 'old', r)}
                          onResolve={() => onResolve(thread.id)} onSend={(t, kind) => onSend(thread.id, t, kind)} />
                      </div>
                    )}
                    {thread && !thread.open && (
                      <div style={{ display: 'flex', minWidth: 0 }}>
                        {(thread.side || 'old') === 'old'
                          ? <div style={{ flex: '1 1 0', minWidth: 0, padding: '6px 0 8px 52px' }}>
                              <Thread messages={thread.messages} resolved={thread.resolved}
                                collapsed onToggle={() => onOpenLine('old', r)} />
                            </div>
                          : <div style={{ flex: '1 1 0', minWidth: 0 }} />}
                        {(thread.side || 'old') === 'new'
                          ? <div style={{ flex: '1 1 0', minWidth: 0, padding: '6px 0 8px 52px' }}>
                              <Thread messages={thread.messages} resolved={thread.resolved}
                                collapsed onToggle={() => onOpenLine('new', r)} />
                            </div>
                          : <div style={{ flex: '1 1 0', minWidth: 0 }} />}
                      </div>
                    )}
                  </div>
                );
              })}
              <div style={{ height: bottomPad }} />
              </div>

              {/* horizontal scrollbars — one per side, pinned at the bottom. These
                  are the ONLY horizontal controls: each onScroll writes its side's
                  CSS variable, which every code line of that side translates by. */}
              <div style={{ display: 'flex', position: 'sticky', bottom: 0, zIndex: 2,
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
            </React.Fragment>
            )}
          </div>
          </div>
        </div>

        {/* Bottom: hints + unresolved on the left, verdict actions on the RIGHT */}
        <div style={{ display: 'flex', alignItems: 'center',
          padding: '0 var(--canvas-pad) 30px' }}>

          <div style={{ display: 'flex', alignItems: 'center', gap: 16 }}>
            <TextSizeMenu size={codeFontSize} effective={autoFs} onChange={setFont} />
            <span style={{ width: 1, height: 16, background: 'var(--border-subtle)' }} />
            <div style={{ display: 'flex', alignItems: 'center', gap: 18, opacity: 'var(--dim-rest)' }}>
              <KeyHint keys={['←', '→']} label="Move" size="sm" />
              <KeyHint keys="+" label="to comment" size="sm" tone="accent" />
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
