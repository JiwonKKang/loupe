/* Loupe — top-left project / branch menu.
   A single box: the collapsed pill (folder · target) IS the top of the panel;
   opening grows the same box straight down (max-height), so it reads as one
   component. The token is set once in onboarding; the *project* is chosen here
   anytime — Browse… opens a native folder dialog, reads the repo's branches via
   `list_branches`, and the Base/Target selects fill from them. "Open / Re-run
   review" hands `{ repoPath, base, target }` up to App, which re-runs analysis. */

import React from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import { Button } from './Button';

// Render `text` with the first case-insensitive occurrence of `q` highlighted.
// Used by the branch search so you see exactly what matched.
function highlightMatch(text, q) {
  if (!q) return text;
  const idx = text.toLowerCase().indexOf(q);
  if (idx < 0) return text;
  return (
    <React.Fragment>
      {text.slice(0, idx)}
      <mark style={{ background: 'var(--accent-dim)', color: 'var(--accent)',
        borderRadius: 2, padding: '0 1px' }}>{text.slice(idx, idx + q.length)}</mark>
      {text.slice(idx + q.length)}
    </React.Fragment>
  );
}

// basename of a repo path (display label for a recent / the trigger).
const basename = (p) => {
  if (!p) return '';
  const parts = String(p).replace(/[/\\]+$/, '').split(/[/\\]/);
  return parts[parts.length - 1] || p;
};

// Custom branch dropdown: a trigger button (current value + chevron) and a
// fixed-positioned popover (a check per option, accent on the selected one).
// Replaces the native <select> so Base/Target match the design kit.
function BranchSelect({ value, options, onChange, fieldStyle }) {
  const [open, setOpen] = React.useState(false);
  const [rect, setRect] = React.useState(null);
  const [query, setQuery] = React.useState('');
  const trigRef = React.useRef(null);
  const popRef = React.useRef(null);
  const searchRef = React.useRef(null);
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
  // Focus the search box when the popover opens so you can type to filter at once.
  React.useEffect(() => { if (open && searchRef.current) searchRef.current.focus(); }, [open]);
  const toggle = () => {
    if (!open && trigRef.current) setRect(trigRef.current.getBoundingClientRect());
    setQuery('');
    setOpen((v) => !v);
  };
  // Live substring filter (case-insensitive). Show the search box only once the
  // list is long enough to be worth filtering.
  const q = query.trim().toLowerCase();
  const filtered = q ? options.filter((o) => o.toLowerCase().includes(q)) : options;
  const showSearch = options.length > 6;
  const pick = (o) => { onChange(o); setOpen(false); };
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
          minWidth: rect.width, maxWidth: 260, zIndex: 60, background: 'var(--surface-overlay)',
          border: '1px solid var(--border-default)', borderRadius: 'var(--radius-md)',
          boxShadow: 'var(--shadow-pop)', padding: 4, display: 'flex', flexDirection: 'column' }}>
          {showSearch && (
            <input ref={searchRef} value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Escape') { e.preventDefault(); setOpen(false); }
                else if (e.key === 'Enter' && filtered.length) { e.preventDefault(); pick(filtered[0]); }
              }}
              placeholder="브랜치 검색…" spellCheck={false}
              style={{ width: '100%', boxSizing: 'border-box', height: 28, padding: '0 8px', marginBottom: 4,
                borderRadius: 'var(--radius-sm)', background: 'var(--surface-inset)',
                border: '1px solid var(--border-default)', color: 'var(--text-primary)',
                font: '12px/1 var(--font-mono)', outline: 'none', flex: 'none' }} />
          )}
          <div style={{ overflowY: 'auto', maxHeight: 232, minWidth: 0,
            // a thin custom scrollbar so a long branch list reads as scrollable
            scrollbarWidth: 'thin' }}>
            {filtered.length === 0 ? (
              <div style={{ padding: '8px', font: '12px/1.3 var(--font-ui)', color: 'var(--text-faint)' }}>
                일치하는 브랜치 없음
              </div>
            ) : filtered.map((o) => {
              const sel = o === value;
              return (
                <button key={o} type="button" onClick={() => pick(o)}
                  style={{ display: 'flex', alignItems: 'center', gap: 8, width: '100%', padding: '7px 8px',
                    borderRadius: 'var(--radius-sm)', cursor: 'pointer', textAlign: 'left', border: 'none',
                    whiteSpace: 'nowrap', background: sel ? 'var(--accent-dim)' : 'transparent',
                    color: sel ? 'var(--text-primary)' : 'var(--text-secondary)',
                    font: '12px/1 var(--font-mono)', transition: 'background var(--dur-fast) var(--ease-soft)' }}
                  onMouseEnter={(e) => { if (!sel) e.currentTarget.style.background = 'var(--surface-inset)'; }}
                  onMouseLeave={(e) => { if (!sel) e.currentTarget.style.background = 'transparent'; }}>
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.8"
                    strokeLinecap="round" strokeLinejoin="round" style={{ color: sel ? 'var(--accent)' : 'transparent', flex: 'none' }}><path d="M20 6 9 17l-5-5" /></svg>
                  <span style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis' }}>{highlightMatch(o, q)}</span>
                </button>
              );
            })}
          </div>
        </div>
      )}
    </React.Fragment>
  );
}

export default function ProjectMenu({
  project, base, target, branches: branchesProp, recents: recentsProp,
  onChangeProject, onBrowse, defaultOpen = false, pinned = false,
}) {
  // `pinned` (used by App's pickProject shell, where there is no project yet):
  // the menu must stay open — outside-click never closes it and the trigger
  // can't toggle it shut. The empty canvas behind it has nothing to fall back
  // to, so a stray click on it shouldn't dismiss the only way to start a review.
  const [open_, setOpenRaw] = React.useState(!!defaultOpen || !!pinned);
  // When pinned, swallow any close request (false); open requests still pass.
  const setOpen = React.useCallback((next) => {
    setOpenRaw((prev) => {
      const v = typeof next === 'function' ? next(prev) : next;
      return pinned ? true : v;
    });
  }, [pinned]);
  const [hover, setHover] = React.useState(false);
  const ref = React.useRef(null);

  // Working copy of the selection, seeded from props. `repoPath` is the absolute
  // path; the trigger/recents show its basename.
  const [repoPath, setRepoPath] = React.useState(project || '');
  const [b, setB] = React.useState(base || '');
  const [t, setT] = React.useState(target || '');

  // Branches for the chosen repo. Seeded from the prop (App's current range) and
  // replaced when Browse… reads a new repo.
  const [branches, setBranches] = React.useState(branchesProp || []);
  const [branchLoading, setBranchLoading] = React.useState(false);
  const [branchError, setBranchError] = React.useState(null);
  const [picking, setPicking] = React.useState(false);

  // localStorage-backed recents (most-recent-first repoPath array). The prop
  // wins if provided; otherwise we read the store.
  const [recents, setRecents] = React.useState(() => {
    if (Array.isArray(recentsProp)) return recentsProp;
    try {
      const raw = window.localStorage.getItem('loupe.recents');
      const arr = raw ? JSON.parse(raw) : [];
      return Array.isArray(arr) ? arr : [];
    } catch { return []; }
  });

  // Re-sync the working copy whenever the upstream range changes or the panel
  // (re)opens — so an external re-run reflects here, and reopening discards an
  // un-committed edit.
  React.useEffect(() => {
    setRepoPath(project || '');
    setB(base || '');
    setT(target || '');
    if (Array.isArray(branchesProp)) setBranches(branchesProp);
  }, [project, base, target, branchesProp, open_]);

  // Keep the panel open while pinned (e.g. pickProject), even if it was never
  // toggled — covers a parent flipping `pinned` true after mount.
  React.useEffect(() => { if (pinned) setOpenRaw(true); }, [pinned]);

  // Outside-click closes the panel — EXCEPT when pinned: the pickProject shell
  // has only an empty canvas behind the menu, so a stray click there must not
  // dismiss the only entry point to a review. (When pinned, `setOpen(false)`
  // already no-ops, but we also skip binding the listener so it's truly inert.)
  React.useEffect(() => {
    if (!open_ || pinned) return;
    const onDoc = (e) => { if (ref.current && !ref.current.contains(e.target)) setOpen(false); };
    document.addEventListener('mousedown', onDoc);
    return () => document.removeEventListener('mousedown', onDoc);
  }, [open_, pinned, setOpen]);

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

  // Read the chosen repo's branches and seed base/target defaults (default for
  // base, current for target — same convention as onboarding).
  const loadBranches = async (dir) => {
    setBranchLoading(true);
    setBranchError(null);
    setBranches([]);
    try {
      const res = await invoke('list_branches', { repoPath: dir });
      const list = res.branches || [];
      setBranches(list);
      if (list.length === 0) {
        setBranchError('이 폴더에서 브랜치를 찾지 못했어요.');
        setB(''); setT('');
        return;
      }
      const nextBase = res.default && list.includes(res.default) ? res.default : list[0];
      let nextTarget = res.current && list.includes(res.current) ? res.current : null;
      if (!nextTarget || nextTarget === nextBase) {
        nextTarget = list.find((x) => x !== nextBase) || nextBase;
      }
      setB(nextBase);
      setT(nextTarget);
    } catch {
      setBranchError('git 저장소가 아니거나 브랜치를 읽을 수 없어요.');
      setBranches([]); setB(''); setT('');
    } finally {
      setBranchLoading(false);
    }
  };

  // Native folder picker → set repoPath + load its branches. Defensive: in a
  // plain browser (no Tauri) `open` throws; surface a gentle error instead of
  // crashing. `onBrowse` is an optional hook for the host (e.g. analytics).
  const browse = async () => {
    if (onBrowse) onBrowse();
    setPicking(true);
    try {
      const dir = await open({ directory: true, multiple: false, title: 'Select a git repository' });
      if (typeof dir === 'string' && dir.length > 0) {
        setRepoPath(dir);
        await loadBranches(dir);
      }
      // null (cancelled) or array → ignore.
    } catch {
      setBranchError('폴더를 열 수 없어요. Tauri 앱에서 실행 중인지 확인해 주세요.');
    } finally {
      setPicking(false);
    }
  };

  // Pick a recent repo path → load its branches.
  const pickRecent = async (dir) => {
    setRepoPath(dir);
    await loadBranches(dir);
  };

  // Commit the selection. Persists the repo into recents (most-recent-first,
  // de-duped, capped) and hands the range up to App to re-run analysis.
  const commit = () => {
    if (!repoPath || !b || !t) return;
    try {
      const next = [repoPath, ...recents.filter((r) => r !== repoPath)].slice(0, 4);
      setRecents(next);
      window.localStorage.setItem('loupe.recents', JSON.stringify(next));
    } catch { /* storage unavailable — non-fatal */ }
    onChangeProject({ repoPath, base: b, target: t });
    setOpen(false);
  };

  const triggerLabel = basename(repoPath) || 'Pick a project';
  // "Open" when nothing/a different repo is staged vs the active project; else
  // "Re-run" (same repo+range). Disabled until a repo + both branches exist.
  const dirty = repoPath !== project || b !== base || t !== target;
  const canCommit = !!repoPath && !!b && !!t && !branchLoading;

  // Options for the custom dropdown: real branches, falling back to the field's
  // own staged value so the box still shows something before branches load.
  const optsFor = (value) => (branches.length > 0 ? branches : (value ? [value] : []));

  return (
    <div ref={ref} style={{ position: 'absolute', top: 20, left: 24, zIndex: 40,
      // Size to the trigger so the bar grows with the folder name (open OR closed).
      // Open keeps a 296 floor for the panel. Capped so the bar never reaches the
      // centered "02 / 08 · cluster" title (≈ stage center).
      width: 'max-content',
      minWidth: open_ ? 296 : 0,
      maxWidth: 'min(420px, calc(50vw - 300px))' }}>
      {/* one container: the trigger row IS the top of the panel; opening grows
         the same box downward (max-height), so it reads as a single component */}
      <div
        onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)}
        style={{ borderRadius: 'var(--radius-md)', overflow: 'hidden',
          border: `1px solid ${open_ ? 'var(--border-default)' : 'transparent'}`,
          background: open_ ? 'var(--surface-overlay)' : (hover ? 'var(--surface-overlay)' : 'transparent'),
          boxShadow: open_ ? 'var(--shadow-pop)' : 'none',
          opacity: open_ || hover ? 1 : 'var(--dim-rest)',
          transition: 'background var(--dur-fast) var(--ease-soft), border-color var(--dur-fast) var(--ease-soft), box-shadow var(--dur-base) var(--ease-soft), opacity var(--dur-fast) var(--ease-soft)' }}>

        {/* trigger row */}
        <button
          onClick={() => setOpen((v) => !v)}
          style={{ display: 'flex', alignItems: 'center', gap: 7, width: '100%', height: 32, padding: '0 10px',
            background: 'transparent', border: 'none', cursor: 'pointer',
            color: open_ ? 'var(--text-primary)' : 'var(--text-secondary)' }}>
          <span style={{ color: 'currentColor', display: 'inline-flex', opacity: 0.7 }}><Ico d={folder} w={13} /></span>
          <span style={{ font: 'var(--weight-medium) var(--text-sm)/1 var(--font-ui)', color: 'currentColor',
            whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
            minWidth: 0, flexShrink: 1, flexGrow: 0 }}>{triggerLabel}</span>
          {target ? <span style={{ width: 1, height: 12, background: 'var(--border-default)', flexShrink: 0 }} /> : null}
          {target ? <span style={{ font: 'var(--text-xs)/1 var(--font-mono)', color: 'var(--text-tertiary)',
            whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', minWidth: 0, flexShrink: 2 }}>{target}</span> : null}
          <span style={{ flex: 1 }} />
          <span style={{ color: 'var(--text-faint)', display: 'inline-flex',
            transform: open_ ? 'rotate(180deg)' : 'none', transition: 'transform var(--dur-base) var(--ease-soft)' }}><Ico d={chev} w={12} /></span>
        </button>

        {/* panel that expands straight down inside the same box. Fixed 296 (collapsed
           → 0) so its content never stretches the max-content trigger bar; when the
           folder name makes the bar wider than 296, the panel just sits left-aligned. */}
        <div style={{ maxHeight: open_ ? 420 : 0, opacity: open_ ? 1 : 0,
          width: open_ ? 296 : 0,
          transition: 'max-height var(--dur-slow) var(--ease-out), opacity var(--dur-base) var(--ease-soft)',
          overflow: 'hidden' }}>
          <div style={{ padding: '4px 10px 10px', borderTop: '1px solid var(--border-subtle)',
            width: 296, boxSizing: 'border-box' }}>
            <label style={{ ...labelStyle, marginTop: 8 }}>Project</label>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 1, marginBottom: 6 }}>
              {recents.slice(0, 4).map((r) => (
                <button key={r} onClick={() => pickRecent(r)} title={r} style={{
                  display: 'flex', alignItems: 'center', gap: 8, padding: '6px 8px',
                  borderRadius: 'var(--radius-sm)', cursor: 'pointer', textAlign: 'left',
                  background: repoPath === r ? 'var(--accent-dim)' : 'transparent',
                  border: `1px solid ${repoPath === r ? 'var(--accent-line)' : 'transparent'}`,
                  font: '12px/1 var(--font-mono)',
                  overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                  color: repoPath === r ? 'var(--text-primary)' : 'var(--text-secondary)' }}>
                  <span style={{ color: repoPath === r ? 'var(--accent)' : 'var(--text-faint)', display: 'inline-flex', flex: 'none' }}><Ico d={folder} w={12} /></span>
                  <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{basename(r)}</span>
                </button>
              ))}
              <button onClick={browse} disabled={picking} style={{
                display: 'flex', alignItems: 'center', gap: 8, padding: '6px 8px',
                borderRadius: 'var(--radius-sm)', cursor: picking ? 'default' : 'pointer', textAlign: 'left',
                background: 'transparent', border: '1px solid transparent',
                font: '12px/1 var(--font-ui)', color: 'var(--text-tertiary)' }}>
                <span style={{ display: 'inline-flex', flex: 'none' }}><Ico d="M12 5v14M5 12h14" w={12} /></span>
                {picking ? 'Opening…' : 'Browse…'}
              </button>
            </div>

            {/* selected path (monospace, ellipsised from the left) */}
            {repoPath && (
              <div style={{ font: 'var(--text-xs)/1.4 var(--font-mono)', color: 'var(--text-faint)',
                padding: '0 2px 4px', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                direction: 'rtl', textAlign: 'left' }} title={repoPath}>{repoPath}</div>
            )}

            {/* branch read feedback */}
            {branchLoading && (
              <div style={{ padding: '2px 2px 4px', font: 'var(--text-xs)/1.5 var(--font-ui)',
                color: 'var(--text-tertiary)' }}>Reading branches…</div>
            )}
            {!branchLoading && branchError && (
              <div style={{ padding: '2px 2px 4px', font: 'var(--text-xs)/1.5 var(--font-ui)',
                color: 'var(--flag)' }}>{branchError}</div>
            )}

            <div style={{ display: 'flex', gap: 8, marginTop: 8 }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <label style={labelStyle}>Base</label>
                <BranchSelect value={b} options={optsFor(b)} onChange={setB} fieldStyle={fieldStyle} />
              </div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <label style={labelStyle}>Target</label>
                <BranchSelect value={t} options={optsFor(t)} onChange={setT} fieldStyle={fieldStyle} />
              </div>
            </div>

            <div style={{ marginTop: 10 }}>
              <Button variant="primary" size="sm" fullWidth disabled={!canCommit} onClick={commit}>
                {dirty ? 'Open review' : 'Re-run review'}
              </Button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
