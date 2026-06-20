import React from 'react';

// Cluster-kind accent dot (planning §4.3 kinds + the Unclustered bucket §3.1). Falls back to
// a neutral tone when the kind is unknown (pre-analysis, Stage-1 file grouping).
const KIND_COLOR = {
  flow: 'var(--accent)',
  contract: 'var(--syn-keyword, #c792ea)',
  'domain-concept': 'var(--syn-type, #82aaff)',
  'shared-foundation': 'var(--syn-func, #7fdbca)',
  infra: 'var(--text-tertiary)',
  unclustered: 'var(--text-faint)',
};

/**
 * ProgressSpine — the queue rail.
 * At rest it is a thin "spine" of dots (presence ≈ 0). On hover it expands to reveal the
 * cards in cluster flow order, grouped by cluster (⑧). Each group header carries the cluster
 * title, a kind color dot, and (on expand) its summary; the Unclustered bucket trails last in
 * a dimmed tone. Before the AI analysis arrives the group key is the Stage-1 chapter (file).
 * This is the narrative of the change; it must never compete with the focused card.
 */
export function ProgressSpine({ items = [], activeId, onSelect, defaultExpanded = false }) {
  const [hover, setHover] = React.useState(false);
  const expanded = defaultExpanded || hover;

  const statusColor = {
    pass: 'var(--pass)', flag: 'var(--flag)',
    active: 'var(--accent)', pending: 'var(--text-faint)',
  };

  // group consecutive items by cluster (clusterId), preserving order. Each group remembers
  // its title/kind/summary + whether it is the Unclustered bucket (rendered dimmed, last).
  const groups = [];
  items.forEach((it) => {
    const key = it.clusterId != null ? it.clusterId : it.chapter;
    const last = groups[groups.length - 1];
    if (last && last.key === key) last.items.push(it);
    else groups.push({
      key,
      title: it.clusterTitle || it.chapter,
      kind: it.clusterKind || null,
      summary: it.clusterSummary || '',
      isUnclustered: it.clusterId === '__unclustered',
      items: [it],
    });
  });

  const passed = items.filter((i) => i.status === 'pass' || i.status === 'flag').length;

  return (
    <div
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        height: '100%',
        width: expanded ? 'var(--rail-open)' : 'var(--rail-width)',
        transition: 'var(--t-rail)',
        background: expanded ? 'var(--bg-raised)' : 'transparent',
        borderRight: `1px solid ${expanded ? 'var(--border-subtle)' : 'transparent'}`,
        overflow: 'hidden', flex: 'none',
        opacity: expanded ? 1 : 'var(--dim-rest)',
        display: 'flex', flexDirection: 'column',
      }}
    >
      <div style={{
        flex: 1, overflowY: 'auto', overflowX: 'hidden',
        padding: expanded ? '28px 20px' : '28px 0',
        display: 'flex', flexDirection: 'column',
        alignItems: expanded ? 'stretch' : 'center',
        gap: expanded ? 22 : 0,
      }}>
        {groups.map((g, gi) => (
          <div key={g.key + ':' + gi} style={{ display: 'flex', flexDirection: 'column',
            gap: expanded ? 7 : 9, alignItems: expanded ? 'stretch' : 'center',
            // a touch more separation between clusters than between cards
            marginTop: gi > 0 ? (expanded ? 6 : 14) : 0,
            opacity: g.isUnclustered ? 0.62 : 1 }}>
            {expanded && (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 4,
                marginBottom: 3, paddingLeft: 2 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <span style={{ width: 6, height: 6, borderRadius: 999, flex: 'none',
                    background: KIND_COLOR[g.kind] || 'var(--text-faint)' }} />
                  <span style={{
                    font: `var(--weight-semibold) var(--text-xs)/1.2 var(--font-ui)`,
                    letterSpacing: g.kind ? 'var(--tracking-snug)' : 'var(--tracking-caps)',
                    textTransform: g.kind ? 'none' : 'uppercase',
                    color: g.isUnclustered ? 'var(--text-faint)' : 'var(--text-secondary)',
                    minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                  }}>{g.title}</span>
                </div>
              </div>
            )}
            {/* Collapsed rail: ONE dot per cluster (not per card). The spine is quiet — every
                dot is the same faint tone (no per-kind colors competing); only the cluster
                holding the focused card lights up (accent + glow). Click jumps to it. */}
            {!expanded && (() => {
              const groupActive = g.items.some((it) => it.id === activeId);
              const col = groupActive ? statusColor.active : 'var(--text-faint)';
              const firstId = g.items[0] && g.items[0].id;
              return (
                <div title={g.title} onClick={() => firstId && onSelect && onSelect(firstId)} style={{
                  width: groupActive ? 7 : 5, height: groupActive ? 7 : 5, borderRadius: 999,
                  background: col, cursor: onSelect ? 'pointer' : 'default',
                  boxShadow: groupActive ? '0 0 0 4px var(--accent-dim)' : 'none',
                  transition: 'var(--t-hover)',
                }} />
              );
            })()}
            {expanded && g.items.map((it) => {
              const active = it.id === activeId;
              const col = active ? statusColor.active : statusColor[it.status] || statusColor.pending;
              return (
                <button key={it.id} onClick={() => onSelect && onSelect(it.id)} style={{
                  display: 'flex', alignItems: 'center', gap: 11, width: '100%',
                  textAlign: 'left', padding: '7px 9px', borderRadius: 'var(--radius-md)',
                  border: '1px solid transparent', cursor: 'pointer',
                  background: active ? 'var(--accent-dim)' : 'transparent',
                  transition: 'var(--t-hover)',
                }}
                  onMouseEnter={(e) => { if (!active) e.currentTarget.style.background = 'var(--surface-overlay)'; }}
                  onMouseLeave={(e) => { if (!active) e.currentTarget.style.background = 'transparent'; }}
                >
                  <span style={{ width: 7, height: 7, borderRadius: 999, background: col, flex: 'none',
                    boxShadow: active ? '0 0 0 3px var(--accent-dim)' : 'none' }} />
                  <span style={{ minWidth: 0, flex: 1, display: 'flex', flexDirection: 'column', gap: 2 }}>
                    <span style={{
                      font: `var(--weight-${active ? 'medium' : 'regular'}) var(--text-sm)/1.2 var(--font-mono)`,
                      color: active ? 'var(--text-primary)' : 'var(--text-secondary)',
                      whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
                    }}>{it.symbol || it.label}</span>
                    {it.file && (
                      <span style={{ font: `var(--text-xs)/1 var(--font-mono)`, color: 'var(--text-faint)',
                        whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{it.file}</span>
                    )}
                  </span>
                  {it.threads > 0 && (
                    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 3, flex: 'none',
                      color: active ? 'var(--accent)' : 'var(--text-tertiary)',
                      font: `var(--weight-medium) var(--text-xs)/1 var(--font-ui)` }}>
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor"
                        strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
                        <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
                      </svg>{it.threads}
                    </span>
                  )}
                </button>
              );
            })}
          </div>
        ))}
      </div>
      {expanded && (
        <div style={{
          padding: '14px 22px', borderTop: '1px solid var(--border-subtle)',
          font: `var(--weight-medium) var(--text-xs)/1 var(--font-ui)`,
          color: 'var(--text-tertiary)', letterSpacing: 'var(--tracking-wide)',
        }}>{passed} of {items.length} reviewed</div>
      )}
    </div>
  );
}
