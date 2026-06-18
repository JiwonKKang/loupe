import React from 'react';

/**
 * FileTree — the right-hand changed-files sidebar (GitHub PR style).
 * Collapsed to a single button at top-right; expands to a nested folder tree.
 * Clicking a file jumps to its card. Dim until interacted with, like all chrome.
 */
export function FileTree({ tree, activeId, onSelect, open, onToggle }) {
  const Ico = ({ d, w = 15, sw = 2 }) => (
    <svg width={w} height={w} viewBox="0 0 24 24" fill="none" stroke="currentColor"
      strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round"><path d={d} /></svg>
  );
  const folderIcon = 'M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z';
  const fileIcon = 'M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z M14 2v6h6';
  const filesIcon = 'M9 3h10a2 2 0 0 1 2 2v12 M5 7v12a2 2 0 0 0 2 2h10';
  const collapseIcon = 'M9 18l6-6-6-6';

  const symbolIcon = 'M8 3H7a2 2 0 0 0-2 2v5a2 2 0 0 1-2 2 2 2 0 0 1 2 2v5a2 2 0 0 0 2 2h1 M16 3h1a2 2 0 0 1 2 2v5a2 2 0 0 1 2 2 2 2 0 0 0-2 2v5a2 2 0 0 1-2 2h-1';

  const dot = { pass: 'var(--pass)', flag: 'var(--flag)', active: 'var(--accent)', pending: 'var(--text-faint)' };
  const fileCount = (() => { let n = 0; const walk = (nd) => { n += nd.files.length; Object.values(nd.dirs).forEach(walk); }; walk(tree); return n; })();

  // a file may hold one card (file row IS the target) or several (file row +
  // indented symbol sub-rows). Defined after Row, below.

  // --- collapsed: a single dim button, top-right
  if (!open) {
    return (
      <button onClick={onToggle} title="Changed files" aria-label="Changed files"
        style={{ position: 'absolute', top: 22, right: 22, zIndex: 40,
          display: 'inline-flex', alignItems: 'center', gap: 8, height: 34, padding: '0 12px',
          borderRadius: 'var(--radius-md)', cursor: 'pointer',
          background: 'var(--bg-raised)', border: '1px solid var(--border-subtle)',
          color: 'var(--text-secondary)', font: 'var(--weight-medium) var(--text-sm)/1 var(--font-ui)',
          opacity: 'var(--dim-rest)', transition: 'var(--t-dim), var(--t-hover)' }}
        onMouseEnter={(e) => { e.currentTarget.style.opacity = 1; e.currentTarget.style.color = 'var(--text-primary)'; }}
        onMouseLeave={(e) => { e.currentTarget.style.opacity = 'var(--dim-rest)'; e.currentTarget.style.color = 'var(--text-secondary)'; }}>
        <Ico d={filesIcon} w={15} />{fileCount}
      </button>
    );
  }

  // --- expanded panel
  const Row = ({ depth, children, onClick, activeRow }) => (
    <button onClick={onClick} style={{
      display: 'flex', alignItems: 'center', gap: 7, width: '100%', textAlign: 'left',
      padding: '5px 10px', paddingLeft: 10 + depth * 14, border: 'none', cursor: 'pointer',
      borderRadius: 'var(--radius-sm)', background: activeRow ? 'var(--accent-dim)' : 'transparent',
      transition: 'var(--t-hover)' }}
      onMouseEnter={(e) => { if (!activeRow) e.currentTarget.style.background = 'var(--surface-overlay)'; }}
      onMouseLeave={(e) => { if (!activeRow) e.currentTarget.style.background = 'transparent'; }}>
      {children}
    </button>
  );

  // render one file: single card -> the file row is the jump target;
  // multiple cards -> the file row is a static header with symbol sub-rows.
  const renderFile = (f, depth) => {
    const multi = f.cards.length > 1;
    const single = f.cards[0];
    const fileActive = !multi && single.id === activeId;
    const counts = (a, d) => (
      <span style={{ flex: 'none', display: 'inline-flex', gap: 5,
        font: 'var(--weight-medium) var(--text-xs)/1 var(--font-mono)' }}>
        {a > 0 && <span style={{ color: 'var(--pass)' }}>+{a}</span>}
        {d > 0 && <span style={{ color: 'var(--diff-del-text)' }}>−{d}</span>}
      </span>
    );
    const fileGlyph = (active) => (
      <span style={{ flex: 'none', display: 'inline-flex', color: active ? 'var(--accent)' : 'var(--text-faint)' }}>
        <Ico d={fileIcon} w={13} sw={1.8} /></span>
    );
    const fileName = (active) => (
      <span style={{ flex: 1, minWidth: 0, font: `var(--weight-${active ? 'medium' : 'regular'}) var(--text-sm)/1.3 var(--font-mono)`,
        color: active ? 'var(--text-primary)' : 'var(--text-secondary)',
        whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{f.name}</span>
    );

    if (!multi) {
      return (
        <Row key={single.id} depth={depth} activeRow={fileActive} onClick={() => onSelect(single.id)}>
          {fileGlyph(fileActive)}{fileName(fileActive)}{counts(f.add, f.del)}
        </Row>
      );
    }
    return (
      <React.Fragment key={f.name + depth}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 7, padding: '5px 10px',
          paddingLeft: 10 + depth * 14 }}>
          {fileGlyph(false)}{fileName(false)}
          <span style={{ flex: 'none', font: 'var(--text-xs)/1 var(--font-mono)', color: 'var(--text-faint)' }}>{f.cards.length}</span>
        </div>
        {f.cards.map((cd) => {
          const on = cd.id === activeId;
          return (
            <Row key={cd.id} depth={depth + 1} activeRow={on} onClick={() => onSelect(cd.id)}>
              <span style={{ flex: 'none', display: 'inline-flex',
                color: on ? 'var(--accent)' : 'var(--text-faint)' }}><Ico d={symbolIcon} w={12} sw={2} /></span>
              <span style={{ flex: 1, minWidth: 0, font: `var(--weight-${on ? 'medium' : 'regular'}) var(--text-sm)/1.3 var(--font-mono)`,
                color: on ? 'var(--text-primary)' : 'var(--text-secondary)',
                whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{cd.symbol}</span>
              {counts(cd.add, cd.del)}
            </Row>
          );
        })}
      </React.Fragment>
    );
  };

  const renderDir = (node, depth) => (
    <React.Fragment key={'d' + depth + node.name}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 7, padding: '5px 10px',
        paddingLeft: 10 + depth * 14 }}>
        <span style={{ color: 'var(--text-tertiary)', display: 'inline-flex' }}><Ico d={folderIcon} w={14} sw={1.8} /></span>
        <span style={{ font: 'var(--weight-medium) var(--text-sm)/1.3 var(--font-mono)',
          color: 'var(--text-secondary)', whiteSpace: 'nowrap', overflow: 'visible' }}>{node.name}</span>
      </div>
      {Object.values(node.dirs).map((d) => renderDir(d, depth + 1))}
      {node.files.map((f) => renderFile(f, depth + 1))}
    </React.Fragment>
  );

  return (
    <div style={{ position: 'absolute', top: 0, right: 0, bottom: 0, zIndex: 40, width: 268,
      background: 'var(--bg-raised)', borderLeft: '1px solid var(--border-subtle)',
      display: 'flex', flexDirection: 'column',
      animation: 'loupe-panel-in var(--dur-base) var(--ease-out)' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 9, padding: '20px 16px 14px',
        borderBottom: '1px solid var(--border-subtle)' }}>
        <span style={{ color: 'var(--text-tertiary)', display: 'inline-flex' }}><Ico d={filesIcon} w={15} /></span>
        <span style={{ font: 'var(--weight-semibold) var(--text-sm)/1 var(--font-ui)',
          letterSpacing: 'var(--tracking-wide)', textTransform: 'uppercase', color: 'var(--text-secondary)' }}>Changed files</span>
        <span style={{ font: 'var(--text-xs)/1 var(--font-mono)', color: 'var(--text-faint)' }}>{fileCount}</span>
        <button onClick={onToggle} title="Collapse" aria-label="Collapse"
          style={{ marginLeft: 'auto', width: 28, height: 28, borderRadius: 'var(--radius-sm)',
            display: 'inline-flex', alignItems: 'center', justifyContent: 'center', cursor: 'pointer',
            background: 'transparent', border: 'none', color: 'var(--text-tertiary)' }}
          onMouseEnter={(e) => { e.currentTarget.style.background = 'var(--surface-overlay)'; e.currentTarget.style.color = 'var(--text-primary)'; }}
          onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = 'var(--text-tertiary)'; }}>
          <Ico d={collapseIcon} w={16} />
        </button>
      </div>
      <div style={{ flex: 1, overflowY: 'auto', padding: '8px 8px 16px' }}>
        {Object.values(tree.dirs).map((d) => renderDir(d, 0))}
        {tree.files.map((f) => renderFile(f, 0))}
      </div>
    </div>
  );
}
