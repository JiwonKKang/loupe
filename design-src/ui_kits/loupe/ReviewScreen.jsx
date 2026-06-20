/* Loupe UI kit — the main review screen (the centerpiece).
   A near-empty canvas: a dim queue spine on the left and one focused card
   in the center. Keyboard-first: Space = Pass + advance, F = Flag, J/K = move.
   Split diff (before | after). Dragging across rows opens an inline AI thread. */

function BranchSelect({ value, options, onChange, fieldStyle }) {
  const [open, setOpen] = React.useState(false);
  const [rect, setRect] = React.useState(null);
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
  return (
    <React.Fragment>
      <button ref={trigRef} onClick={toggle} type="button"
        style={{ ...fieldStyle, display: 'flex', alignItems: 'center', gap: 6, textAlign: 'left',
          color: open ? 'var(--text-primary)' : fieldStyle.color,
          borderColor: open ? 'var(--border-strong)' : 'var(--border-default)' }}>
        <span style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{value}</span>
        <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4"
          strokeLinecap="round" strokeLinejoin="round" style={{ color: 'var(--text-faint)', flex: 'none',
            transform: open ? 'rotate(180deg)' : 'none', transition: 'transform var(--dur-base) var(--ease-soft)' }}><path d="M6 9l6 6 6-6" /></svg>
      </button>
      {open && rect && (
        <div ref={popRef} style={{ position: 'fixed', top: rect.bottom + 5, left: rect.left,
          minWidth: rect.width, maxWidth: 230, zIndex: 60, background: 'var(--surface-overlay)',
          border: '1px solid var(--border-default)', borderRadius: 'var(--radius-md)',
          boxShadow: 'var(--shadow-pop)', padding: 4 }}>
          {options.map((o) => {
            const sel = o === value;
            return (
              <button key={o} type="button" onClick={() => { onChange(o); setOpen(false); }}
                style={{ display: 'flex', alignItems: 'center', gap: 8, width: '100%', padding: '7px 8px',
                  borderRadius: 'var(--radius-sm)', cursor: 'pointer', textAlign: 'left', border: 'none',
                  whiteSpace: 'nowrap', background: sel ? 'var(--accent-dim)' : 'transparent',
                  color: sel ? 'var(--text-primary)' : 'var(--text-secondary)',
                  font: '12px/1 var(--font-mono)', transition: 'background var(--dur-fast) var(--ease-soft)' }}
                onMouseEnter={(e) => { if (!sel) e.currentTarget.style.background = 'var(--surface-inset)'; }}
                onMouseLeave={(e) => { if (!sel) e.currentTarget.style.background = 'transparent'; }}>
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.8"
                  strokeLinecap="round" strokeLinejoin="round" style={{ color: sel ? 'var(--accent)' : 'transparent', flex: 'none' }}><path d="M20 6 9 17l-5-5" /></svg>
                <span style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis' }}>{o}</span>
              </button>
            );
          })}
        </div>
      )}
    </React.Fragment>
  );
}

function ProjectMenu({ project, base, target, onChangeProject }) {
  const { Button } = window.LoupeDesignSystem_045e3b;
  const [open, setOpen] = React.useState(false);
  const [hover, setHover] = React.useState(false);
  const [proj, setProj] = React.useState(project);
  const [b, setB] = React.useState(base);
  const [t, setT] = React.useState(target);
  const ref = React.useRef(null);

  React.useEffect(() => { setProj(project); setB(base); setT(target); }, [project, base, target, open]);
  React.useEffect(() => {
    if (!open) return;
    const onDoc = (e) => { if (ref.current && !ref.current.contains(e.target)) setOpen(false); };
    document.addEventListener('mousedown', onDoc);
    return () => document.removeEventListener('mousedown', onDoc);
  }, [open]);

  const recents = ['monorepo / api', 'edge-proxy', 'billing-worker'];
  const Ico = ({ d, w = 14 }) => (
    <svg width={w} height={w} viewBox="0 0 24 24" fill="none" stroke="currentColor"
      strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d={d} /></svg>
  );
  const folder = 'M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2z';
  const chev = 'M6 9l6 6 6-6';
  const fieldStyle = {
    width: '100%', height: 32, padding: '0 10px', borderRadius: 'var(--radius-sm)',
    background: 'var(--surface-inset)', border: '1px solid var(--border-default)',
    color: 'var(--text-primary)', font: 'var(--text-xs)/1 var(--font-mono)', outline: 'none',
    boxSizing: 'border-box', appearance: 'none', cursor: 'pointer',
  };
  const labelStyle = { font: 'var(--weight-medium) 10px/1 var(--font-ui)',
    letterSpacing: 'var(--tracking-caps)', textTransform: 'uppercase',
    color: 'var(--text-tertiary)', marginBottom: 6, display: 'block' };

  const dirty = proj !== project || b !== base || t !== target;

  return (
    <div ref={ref} style={{ position: 'absolute', top: 20, left: 24, zIndex: 40,
      width: 296 }}>
      {/* one container: the trigger row IS the top of the panel; opening grows
         the same box downward (max-height), so it reads as a single component */}
      <div
        onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)}
        style={{ borderRadius: 'var(--radius-md)', overflow: 'hidden',
          border: `1px solid ${open ? 'var(--border-default)' : 'transparent'}`,
          background: open ? 'var(--surface-overlay)' : (hover ? 'var(--surface-overlay)' : 'transparent'),
          boxShadow: open ? 'var(--shadow-pop)' : 'none',
          opacity: open || hover ? 1 : 'var(--dim-rest)',
          transition: 'background var(--dur-fast) var(--ease-soft), border-color var(--dur-fast) var(--ease-soft), box-shadow var(--dur-base) var(--ease-soft), opacity var(--dur-fast) var(--ease-soft)' }}>

        {/* trigger row */}
        <button
          onClick={() => setOpen((v) => !v)}
          style={{ display: 'flex', alignItems: 'center', gap: 7, width: '100%', height: 32, padding: '0 10px',
            background: 'transparent', border: 'none', cursor: 'pointer',
            color: open ? 'var(--text-primary)' : 'var(--text-secondary)' }}>
          <span style={{ color: 'currentColor', display: 'inline-flex', opacity: 0.7 }}><Ico d={folder} w={13} /></span>
          <span style={{ font: 'var(--weight-medium) var(--text-sm)/1 var(--font-ui)', color: 'currentColor', whiteSpace: 'nowrap' }}>{project}</span>
          <span style={{ width: 1, height: 12, background: 'var(--border-default)' }} />
          <span style={{ font: 'var(--text-xs)/1 var(--font-mono)', color: 'var(--text-tertiary)', whiteSpace: 'nowrap' }}>{target}</span>
          <span style={{ flex: 1 }} />
          <span style={{ color: 'var(--text-faint)', display: 'inline-flex',
            transform: open ? 'rotate(180deg)' : 'none', transition: 'transform var(--dur-base) var(--ease-soft)' }}><Ico d={chev} w={12} /></span>
        </button>

        {/* panel that expands straight down inside the same box */}
        <div style={{ maxHeight: open ? 360 : 0, opacity: open ? 1 : 0,
          transition: 'max-height var(--dur-slow) var(--ease-out), opacity var(--dur-base) var(--ease-soft)',
          overflow: 'hidden' }}>
          <div style={{ padding: '4px 10px 10px', borderTop: '1px solid var(--border-subtle)' }}>
            <label style={{ ...labelStyle, marginTop: 8 }}>Project</label>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 1, marginBottom: 6 }}>
              {recents.map((r) => (
                <button key={r} onClick={() => setProj(r)} style={{
                  display: 'flex', alignItems: 'center', gap: 8, padding: '6px 8px',
                  borderRadius: 'var(--radius-sm)', cursor: 'pointer', textAlign: 'left',
                  background: proj === r ? 'var(--accent-dim)' : 'transparent',
                  border: `1px solid ${proj === r ? 'var(--accent-line)' : 'transparent'}`,
                  font: '12px/1 var(--font-mono)',
                  color: proj === r ? 'var(--text-primary)' : 'var(--text-secondary)' }}>
                  <span style={{ color: proj === r ? 'var(--accent)' : 'var(--text-faint)', display: 'inline-flex' }}><Ico d={folder} w={12} /></span>
                  {r}
                </button>
              ))}
              <button style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '6px 8px',
                borderRadius: 'var(--radius-sm)', cursor: 'pointer', textAlign: 'left', background: 'transparent',
                border: '1px solid transparent', font: '12px/1 var(--font-ui)', color: 'var(--text-tertiary)' }}>
                <span style={{ display: 'inline-flex' }}><Ico d="M12 5v14M5 12h14" w={12} /></span>
                Browse…
              </button>
            </div>

            <div style={{ display: 'flex', gap: 8, marginTop: 8 }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <label style={labelStyle}>Base</label>
                <BranchSelect value={b} options={['main', 'release/24.2', 'develop']} onChange={setB} fieldStyle={fieldStyle} />
              </div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <label style={labelStyle}>Target</label>
                <BranchSelect value={t} options={['agent/refactor-auth', 'agent/add-redaction', 'feature/lease-expiry']} onChange={setT} fieldStyle={fieldStyle} />
              </div>
            </div>

            <div style={{ marginTop: 10 }}>
              <Button variant="primary" size="sm" fullWidth
                onClick={() => { onChangeProject({ project: proj, base: b, target: t }); setOpen(false); }}>
                {dirty ? 'Open review' : 'Re-run review'}
              </Button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
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

function ReviewScreen(props) {
  const DS = window.LoupeDesignSystem_045e3b;
  const { ProgressSpine, Thread, Button, KeyHint } = DS;
  const {
    card, index, total, dir, project, base, target, onChangeProject, unresolved,
    spineItems, onSelect,
    verdict, flagged, hasPrev, hasNext,
    onPass, onPrev, onNext, onJumpUnresolved,
    threads, onOpenLine, onResolve, onSend,
  } = props;

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
  const threadByRow = {};
  threads.forEach((t) => { threadByRow[t.lineN] = t; });

  // ---- Fold unchanged context so big functions fit at a glance ----
  // Keep CONTEXT lines of context around each change; collapse the rest into
  // a thin "⋯ N unchanged lines" divider the reviewer can expand. Font stays
  // large; only the changes (the point of the card) compete for the eye.
  const [expanded, setExpanded] = React.useState(() => new Set());
  React.useEffect(() => { setExpanded(new Set()); }, [card.id]);
  const CONTEXT = 2;
  const display = (() => {
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
  })();

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
  const halfMin = Math.ceil(GUTTER + maxChars * chPx + 14);
  // Each side's code area is its OWN horizontal scroller; short lines get a
  // min-width so every line on a side scrolls by the same amount, and a
  // capture-scroll handler keeps all lines of one side in lock-step (so the
  // BEFORE column and the AFTER column scroll independently of each other).
  const codeContentW = Math.ceil(maxChars * chPx + 28);
  // Responsive card width: grows with the actual code length up to a max, so
  // short changes get a compact card and long ones widen toward the cap.
  const cardW = Math.max(560, Math.min(1140, (62 + codeContentW) * 2 + 6));
  const diffRef = React.useRef(null);
  const syncScroll = (e) => {
    const t = e.target;
    if (!t || !t.getAttribute) return;
    const side = t.getAttribute('data-codescroll');
    if (!side || !diffRef.current) return;
    const sl = t.scrollLeft;
    diffRef.current.querySelectorAll('[data-codescroll="' + side + '"]').forEach((el) => {
      if (el !== t && Math.abs(el.scrollLeft - sl) > 0.5) el.scrollLeft = sl;
    });
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
        <div className="loupe-codescroll" data-codescroll={side} style={{ overflowX: 'auto', overflowY: 'hidden', minWidth: 0,
          font: codeFs + 'px/var(--leading-code) var(--font-mono)' }}>
          <span style={{ display: 'inline-block', minWidth: codeContentW, whiteSpace: 'pre',
            paddingLeft: 11, paddingRight: 16, tabSize: 2 }}>
            {cell ? window.LoupeData.highlightGo(cell.c) : ''}
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
          <span>{card.chapter}</span>
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
                color: 'var(--text-secondary)', textWrap: 'pretty' }}>{card.summary}</div>
            </div>

            {/* hidden monospace sizer — measures one char at the current size */}
            <span ref={charRef} aria-hidden="true" style={{ position: 'absolute', visibility: 'hidden',
              pointerEvents: 'none', whiteSpace: 'pre', font: codeFs + 'px/1 var(--font-mono)' }}>{'0'.repeat(50)}</span>

            {/* Split diff — each side (BEFORE / AFTER) scrolls left/right on its own */}
            <div ref={diffRef} onScrollCapture={syncScroll} onMouseLeave={() => { if (!dragging) setHover(null); }}
              style={{ overflowY: 'auto', overflowX: 'hidden', flex: 1, userSelect: 'none',
              background: 'var(--surface-inset)' }}>

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
              {display.map((item) => {
                if (item.type === 'fold') {
                  return (
                    <div key={'fold-' + item.key} onClick={() => setExpanded((s) => new Set(s).add(item.key))}
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
                return (
                  <React.Fragment key={r}>
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
                          onResolve={() => onResolve(thread.id)} onSend={(t) => onSend(thread.id, t)} />
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
                  </React.Fragment>
                );
              })}
              </div>

              {/* synced horizontal scrollbars — one per side, pinned at the bottom */}
              <div style={{ display: 'flex', position: 'sticky', bottom: 0, zIndex: 2,
                background: 'var(--surface-inset)' }}>
                <div style={{ flex: '1 1 0', minWidth: 0, display: 'flex' }}>
                  <div style={{ width: GUTTER, flex: 'none' }} />
                  <div className="loupe-hbar" data-codescroll="old" style={{ flex: '1 1 0', minWidth: 0, overflowX: 'auto', overflowY: 'hidden' }}>
                    <div style={{ width: codeContentW, height: 1 }} />
                  </div>
                </div>
                <div style={{ flex: '1 1 0', minWidth: 0, display: 'flex' }}>
                  <div style={{ width: GUTTER, flex: 'none' }} />
                  <div className="loupe-hbar" data-codescroll="new" style={{ flex: '1 1 0', minWidth: 0, overflowX: 'auto', overflowY: 'hidden' }}>
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

window.ReviewScreen = ReviewScreen;
