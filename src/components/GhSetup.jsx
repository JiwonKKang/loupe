/* GitHub CLI setup status — shown in onboarding AND settings. `gh` is what Loupe
   delegates PR approve / comment / `loupe <pr-url>` to (Loupe makes no GitHub calls
   of its own), so this surfaces whether it's installed + authenticated and hands the
   two setup commands. Optional: the core review needs only the Claude token. */

import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from './Button';

export default function GhSetup({ label = 'GitHub CLI — for PR approve · comment · loupe <pr-url>' }) {
  // null = checking, else { installed, authed }
  const [gh, setGh] = React.useState(null);
  const [copiedCmd, setCopiedCmd] = React.useState(null);

  const check = React.useCallback(() => {
    setGh(null);
    invoke('gh_status').then(setGh).catch(() => setGh({ installed: false, authed: false }));
  }, []);
  React.useEffect(() => { check(); }, [check]);

  const copyCmd = (id, text) => {
    try { navigator.clipboard && navigator.clipboard.writeText(text); } catch { /* ignore */ }
    setCopiedCmd(id); setTimeout(() => setCopiedCmd((c) => (c === id ? null : c)), 1400);
  };

  const Ico = ({ d, w = 15 }) => (
    <svg width={w} height={w} viewBox="0 0 24 24" fill="none" stroke="currentColor"
      strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d={d} /></svg>
  );
  const checkPath = 'M20 6 9 17l-5-5';
  const dot = 'M12 12m-3 0a3 3 0 1 0 6 0a3 3 0 1 0-6 0';
  const copyPath = 'M8 4v12a2 2 0 0 0 2 2h8M8 4a2 2 0 0 1 2-2h6.5L20 5.5V14a2 2 0 0 1-2 2H10a2 2 0 0 1-2-2z M8 4H6a2 2 0 0 0-2 2v12';

  const labelStyle = { font: 'var(--weight-medium) var(--text-xs)/1 var(--font-ui)',
    letterSpacing: 'var(--tracking-wide)', textTransform: 'uppercase',
    color: 'var(--text-tertiary)', marginBottom: 9, display: 'block' };

  const cmdBox = (id, cmd) => (
    <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 8,
      background: 'var(--surface-inset)', border: '1px solid var(--border-default)',
      borderRadius: 'var(--radius-md)', padding: '8px 8px 8px 12px' }}>
      <code style={{ flex: 1, font: 'var(--code-sm)/1.4 var(--font-mono)', color: 'var(--syn-plain)' }}>
        <span style={{ color: 'var(--text-faint)' }}>$ </span>{cmd}
      </code>
      <Button size="sm" variant="secondary"
        icon={copiedCmd === id ? <span style={{ color: 'var(--pass)' }}><Ico d={checkPath} w={13} /></span> : <Ico d={copyPath} w={13} />}
        onClick={() => copyCmd(id, cmd)}>{copiedCmd === id ? 'Copied' : 'Copy'}</Button>
    </div>
  );

  const ok = gh && gh.installed && gh.authed;
  return (
    <div>
      <label style={labelStyle}>{label}</label>
      <div style={{ display: 'flex', alignItems: 'center', gap: 9,
        padding: '10px 12px', borderRadius: 'var(--radius-md)',
        background: 'var(--surface-inset)', border: '1px solid var(--border-default)' }}>
        <span style={{ display: 'inline-flex', flex: 'none',
          color: gh === null ? 'var(--text-faint)' : (ok ? 'var(--pass)' : 'var(--flag)') }}>
          <Ico d={ok ? checkPath : dot} w={15} />
        </span>
        <span style={{ flex: 1, minWidth: 0, font: 'var(--weight-medium) var(--text-sm)/1.35 var(--font-ui)',
          color: gh === null ? 'var(--text-tertiary)' : 'var(--text-secondary)' }}>
          {gh === null ? 'gh 확인 중…'
            : ok ? 'GitHub CLI 연결됨 — PR 기능을 쓸 수 있어요.'
            : gh.installed ? 'gh가 설치돼 있지만 로그인이 필요해요.'
            : 'GitHub CLI(gh)가 없어요 — PR 기능에만 필요해요 (선택).'}
        </span>
        <button onClick={check} title="다시 확인" style={{
          flex: 'none', height: 26, padding: '0 10px', borderRadius: 'var(--radius-sm)',
          background: 'transparent', border: '1px solid var(--border-default)', cursor: 'pointer',
          color: 'var(--text-secondary)', font: 'var(--weight-medium) var(--text-xs)/1 var(--font-ui)' }}>
          다시 확인
        </button>
      </div>
      {gh && !ok && (
        <div>
          {!gh.installed && cmdBox('gh-install', 'brew install gh')}
          {cmdBox('gh-auth', 'gh auth login')}
        </div>
      )}
    </div>
  );
}
