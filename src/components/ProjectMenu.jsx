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

// basename of a repo path (display label for a recent / the trigger).
const basename = (p) => {
  if (!p) return '';
  const parts = String(p).replace(/[/\\]+$/, '').split(/[/\\]/);
  return parts[parts.length - 1] || p;
};

export default function ProjectMenu({
  project, base, target, branches: branchesProp, recents: recentsProp,
  onChangeProject, onBrowse, defaultOpen = false,
}) {
  const [open_, setOpen] = React.useState(!!defaultOpen);
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

  // Outside-click closes the panel (but never while it's the auto-opened picker
  // shell with no project chosen — App keeps that one open via defaultOpen, and
  // closing it is still fine since there's a center hint behind it).
  React.useEffect(() => {
    if (!open_) return;
    const onDoc = (e) => { if (ref.current && !ref.current.contains(e.target)) setOpen(false); };
    document.addEventListener('mousedown', onDoc);
    return () => document.removeEventListener('mousedown', onDoc);
  }, [open_]);

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
      const next = [repoPath, ...recents.filter((r) => r !== repoPath)].slice(0, 6);
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

  // <select> filled from real branches (falls back to the single staged value so
  // the box still shows something before branches load).
  const BranchSelect = ({ value, onChange }) => {
    const opts = branches.length > 0 ? branches : (value ? [value] : []);
    return (
      <select value={value} onChange={(e) => onChange(e.target.value)}
        disabled={opts.length === 0 || branchLoading}
        style={{ ...fieldStyle, opacity: opts.length === 0 || branchLoading ? 0.5 : 1 }}>
        {opts.map((x) => <option key={x} value={x}>{x}</option>)}
      </select>
    );
  };

  return (
    <div ref={ref} style={{ position: 'absolute', top: 20, left: 24, zIndex: 40, width: 296 }}>
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
            whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', maxWidth: 150 }}>{triggerLabel}</span>
          {target ? <span style={{ width: 1, height: 12, background: 'var(--border-default)' }} /> : null}
          {target ? <span style={{ font: 'var(--text-xs)/1 var(--font-mono)', color: 'var(--text-tertiary)', whiteSpace: 'nowrap' }}>{target}</span> : null}
          <span style={{ flex: 1 }} />
          <span style={{ color: 'var(--text-faint)', display: 'inline-flex',
            transform: open_ ? 'rotate(180deg)' : 'none', transition: 'transform var(--dur-base) var(--ease-soft)' }}><Ico d={chev} w={12} /></span>
        </button>

        {/* panel that expands straight down inside the same box */}
        <div style={{ maxHeight: open_ ? 420 : 0, opacity: open_ ? 1 : 0,
          transition: 'max-height var(--dur-slow) var(--ease-out), opacity var(--dur-base) var(--ease-soft)',
          overflow: 'hidden' }}>
          <div style={{ padding: '4px 10px 10px', borderTop: '1px solid var(--border-subtle)' }}>
            <label style={{ ...labelStyle, marginTop: 8 }}>Project</label>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 1, marginBottom: 6 }}>
              {recents.map((r) => (
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
                <BranchSelect value={b} onChange={setB} />
              </div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <label style={labelStyle}>Target</label>
                <BranchSelect value={t} onChange={setT} />
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
