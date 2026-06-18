/* Loupe UI kit — the main review screen (the centerpiece).
   A near-empty canvas: a dim queue spine on the left and one focused card
   in the center. Keyboard-first: Space = Pass + advance, F = Flag, J/K = move.
   Split diff (before | after). Dragging across rows opens an inline AI thread. */

function ReviewScreen(props) {
  const DS = window.LoupeDesignSystem_045e3b;
  const { ProgressSpine, Thread, Button, KeyHint } = DS;
  const {
    card, index, total, dir, base, target, unresolved,
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

  // round prev/next button, dim until hovered, clear of the card AND the deck
  const NavArrow = ({ side, d, disabled, onClick, label, offset }) => (
    <button onClick={disabled ? undefined : onClick} aria-label={label} title={label} disabled={disabled}
      style={{ position: 'absolute', zIndex: 3, top: '50%', transform: 'translateY(-50%)',
        [side]: offset, width: 40, height: 40, borderRadius: 'var(--radius-pill)',
        display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
        background: 'var(--surface-overlay)', border: '1px solid var(--border-default)',
        color: 'var(--text-secondary)', cursor: disabled ? 'default' : 'pointer',
        opacity: disabled ? 0.14 : 'var(--dim-rest)', transition: 'var(--t-dim), var(--t-hover)' }}
      onMouseEnter={(e) => { if (!disabled) { e.currentTarget.style.opacity = 1; e.currentTarget.style.color = 'var(--text-primary)'; } }}
      onMouseLeave={(e) => { if (!disabled) { e.currentTarget.style.opacity = 'var(--dim-rest)'; e.currentTarget.style.color = 'var(--text-secondary)'; } }}>
      <Ico d={d} w={18} />
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
        alignItems: 'baseline', background: bg, cursor: 'default',
        boxShadow,
        minWidth: 0, borderLeft: side === 'new' ? '1px solid var(--border-subtle)' : 'none',
        transition: 'background var(--dur-fast) var(--ease-soft)' }}>
        {showPlus && (
          <button
            onMouseDown={(e) => { e.stopPropagation(); e.preventDefault(); setDragSide(side); setDragFrom(r); setDragTo(r); }}
            title="Comment on this line (drag to select a range)"
            style={{ position: 'absolute', zIndex: 4, left: 1, top: '50%', transform: 'translateY(-50%)',
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
          font: 'var(--code-sm)/var(--leading-code) var(--font-mono)', userSelect: 'none' }}>{cell ? cell.n : ''}</span>
        <span style={{ color: side === 'old' ? 'var(--diff-del-edge)' : 'var(--diff-add-edge)',
          userSelect: 'none', fontWeight: 600, font: 'var(--code-sm)/var(--leading-code) var(--font-mono)' }}>{sign}</span>
        <span style={{ whiteSpace: 'pre', overflow: 'hidden', textOverflow: 'ellipsis', paddingRight: 12,
          tabSize: 4, font: 'var(--code-sm)/var(--leading-code) var(--font-mono)' }}>
          {cell ? window.LoupeData.highlightGo(cell.c) : ''}
        </span>
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

        {/* Minimal top bar — progress · chapter · base→target */}
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center',
          gap: 12, padding: '26px 0 0', opacity: 'var(--dim-rest)',
          font: 'var(--weight-medium) var(--text-sm)/1 var(--font-ui)',
          color: 'var(--text-secondary)', letterSpacing: 'var(--tracking-wide)' }}>
          <span style={{ color: 'var(--text-tertiary)', fontVariantNumeric: 'tabular-nums' }}>
            {String(index + 1).padStart(2, '0')} / {String(total).padStart(2, '0')}
          </span>
          <span style={{ width: 3, height: 3, borderRadius: 999, background: 'var(--text-faint)' }} />
          <span>{card.chapter}</span>
          <span style={{ width: 3, height: 3, borderRadius: 999, background: 'var(--text-faint)' }} />
          <span style={{ display: 'inline-flex', alignItems: 'center', gap: 7,
            font: 'var(--text-xs)/1 var(--font-mono)', color: 'var(--text-tertiary)' }}>
            {base}<span style={{ color: 'var(--text-faint)' }}><Ico d={arrow} w={13} /></span>{target}
          </span>
        </div>

        {/* Centered card — with a faint deck of remaining cards peeking to the RIGHT */}
        <div style={{ flex: 1, display: 'flex', alignItems: 'center',
          justifyContent: 'center', padding: '24px var(--canvas-pad)', minHeight: 0 }}>
          <div style={{ position: 'relative', width: '100%', maxWidth: 760,
            maxHeight: '100%', display: 'flex' }}>

            {/* deck: upcoming cards peeking off the right edge */}
            {remaining >= 2 && (
              <div aria-hidden="true" style={{ position: 'absolute', zIndex: 0,
                top: 26, bottom: 26, left: 46, right: -40,
                background: 'var(--bg-raised)', border: '1px solid var(--border-default)',
                borderRadius: 'var(--radius-card)', boxShadow: 'var(--shadow-card)' }} />
            )}
            {remaining >= 1 && (
              <div aria-hidden="true" style={{ position: 'absolute', zIndex: 1,
                top: 13, bottom: 13, left: 24, right: -21,
                background: 'var(--surface-overlay)', border: '1px solid var(--border-default)',
                borderRadius: 'var(--radius-card)', boxShadow: 'var(--shadow-card)' }} />
            )}

            {/* side navigation arrows — prev / next (right one clears the deck) */}
            <NavArrow side="left" d={chevL} disabled={!hasPrev} onClick={onPrev} label="Previous card" offset={-58} />
            <NavArrow side="right" d={chevR} disabled={false} onClick={onNext} label="Next card" offset={remaining >= 1 ? -88 : -58} />

          <div key={card.id} style={{
            position: 'relative', zIndex: 2,
            width: '100%', maxHeight: '100%', display: 'flex', flexDirection: 'column',
            background: 'var(--surface-card)', border: '1px solid var(--border-subtle)',
            borderRadius: 'var(--radius-card)', boxShadow: 'var(--shadow-focus)',
            animation: `loupe-card-in var(--dur-slow) var(--ease-out)`,
            ['--enter-x']: `${dir * 36}px`, overflow: 'hidden',
          }}>
            {/* Card header */}
            <div style={{ padding: '22px var(--gutter-card) 18px',
              borderBottom: '1px solid var(--border-subtle)' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 12 }}>
                <span style={{ font: 'var(--weight-semibold) var(--text-md)/1 var(--font-mono)',
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
              <div style={{ font: 'var(--text-base)/var(--leading-normal) var(--font-ui)',
                color: 'var(--text-secondary)', textWrap: 'pretty' }}>{card.summary}</div>
            </div>

            {/* Split-diff column headers */}
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr',
              borderBottom: '1px solid var(--border-subtle)', background: 'var(--surface-inset)' }}>
              <div style={{ padding: '7px 0 7px 62px', font: 'var(--weight-medium) var(--text-xs)/1 var(--font-ui)',
                letterSpacing: 'var(--tracking-wide)', textTransform: 'uppercase', color: 'var(--text-tertiary)' }}>Before</div>
              <div style={{ padding: '7px 0 7px 62px', font: 'var(--weight-medium) var(--text-xs)/1 var(--font-ui)',
                letterSpacing: 'var(--tracking-wide)', textTransform: 'uppercase', color: 'var(--text-tertiary)',
                borderLeft: '1px solid var(--border-subtle)' }}>After</div>
            </div>

            {/* Split diff */}
            <div onMouseLeave={() => { if (!dragging) setHover(null); }}
              style={{ overflowY: 'auto', padding: '8px 0', flex: 1, userSelect: 'none',
              background: 'var(--surface-inset)' }}>
              {rows.map((row, r) => {
                const thread = threadByRow[r];
                const active = inRange(r);
                const lo = dragging ? Math.min(dragFrom, dragTo) : -1;
                const hi = dragging ? Math.max(dragFrom, dragTo) : -1;
                const isFirst = active && r === lo;
                const isLast = active && r === hi;
                return (
                  <React.Fragment key={r}>
                    <div data-line={r} style={{ display: 'grid', gridTemplateColumns: '1fr 1fr' }}>
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
                      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr' }}>
                        {(thread.side || 'old') === 'old'
                          ? <div style={{ padding: '6px 0 8px 52px' }}>
                              <Thread messages={thread.messages} resolved={thread.resolved}
                                collapsed onToggle={() => onOpenLine('old', r)} />
                            </div>
                          : <span />}
                        {(thread.side || 'old') === 'new'
                          ? <div style={{ padding: '6px 0 8px 52px' }}>
                              <Thread messages={thread.messages} resolved={thread.resolved}
                                collapsed onToggle={() => onOpenLine('new', r)} />
                            </div>
                          : <span />}
                      </div>
                    )}
                  </React.Fragment>
                );
              })}
            </div>
          </div>
          </div>
        </div>

        {/* Bottom: hints + unresolved on the left, verdict actions on the RIGHT */}
        <div style={{ display: 'flex', alignItems: 'center',
          padding: '0 var(--canvas-pad) 30px' }}>

          <div style={{ display: 'flex', alignItems: 'center', gap: 16 }}>
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
