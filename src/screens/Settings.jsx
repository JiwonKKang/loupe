/* Loupe — settings. A small screen to *replace* the saved model token: shows the
   current connection state, takes a new token (password input, never shown in
   plaintext), saves it via `save_token`, and can remove it via `clear_token`.
   `onBack` returns to wherever the user came from; `onSaved(token)` / `onCleared`
   let App keep its in-memory `token` in sync. */

import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '../components/Button';
import Logo from '../components/Logo';

export default function Settings({ connected = false, onBack, onSaved, onCleared }) {
  const [token, setToken] = React.useState('');
  const [busy, setBusy] = React.useState(false);
  // status: { tone: 'ok' | 'err', text } | null
  const [status, setStatus] = React.useState(null);
  // Preferred editor for ⌘-click "open in editor" (localStorage). 'auto' tries code then idea.
  const [editor, setEditor] = React.useState(() => {
    try { return window.localStorage.getItem('loupe.editor') || 'idea'; } catch { return 'idea'; }
  });
  const pickEditor = (v) => {
    setEditor(v);
    try { window.localStorage.setItem('loupe.editor', v); } catch { /* ignore */ }
  };

  const Ico = ({ d, w = 16 }) => (
    <svg width={w} height={w} viewBox="0 0 24 24" fill="none" stroke="currentColor"
      strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d={d} /></svg>
  );
  const check = 'M20 6 9 17l-5-5';
  const back = 'M19 12H5M12 19l-7-7 7-7';
  const dot = 'M12 12m-3 0a3 3 0 1 0 6 0a3 3 0 1 0-6 0';

  const fieldStyle = {
    width: '100%', height: 40, padding: '0 14px', borderRadius: 'var(--radius-md)',
    background: 'var(--surface-inset)', border: '1px solid var(--border-default)',
    color: 'var(--text-primary)', font: 'var(--text-base)/1 var(--font-mono)', outline: 'none',
    boxSizing: 'border-box', appearance: 'none',
  };
  const labelStyle = { font: 'var(--weight-medium) var(--text-xs)/1 var(--font-ui)',
    letterSpacing: 'var(--tracking-wide)', textTransform: 'uppercase',
    color: 'var(--text-tertiary)', marginBottom: 9, display: 'block' };

  const save = async () => {
    const t = token.trim();
    if (t.length === 0) return;
    setBusy(true);
    setStatus(null);
    try {
      // Verify the token with a minimal live model call before persisting, so a
      // bad/expired token is rejected here instead of failing mid-review.
      await invoke('verify_token', { token: t });
      await invoke('save_token', { token: t });
      setToken('');
      setStatus({ tone: 'ok', text: 'Token saved. Your model is connected.' });
      if (onSaved) onSaved(t);
    } catch (e) {
      setStatus({ tone: 'err', text: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const clear = async () => {
    setBusy(true);
    setStatus(null);
    try {
      await invoke('clear_token');
      setToken('');
      setStatus({ tone: 'ok', text: 'Token removed.' });
      if (onCleared) onCleared();
    } catch (e) {
      setStatus({ tone: 'err', text: String(e) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={{ position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column',
      alignItems: 'center', justifyContent: 'center', background: 'var(--bg-base)', padding: 24 }}>

      <div style={{ marginBottom: 38 }}>
        <Logo size="sm" />
      </div>

      <div style={{ width: 480, background: 'var(--surface-card)',
        border: '1px solid var(--border-subtle)', borderRadius: 'var(--radius-xl)',
        boxShadow: 'var(--shadow-focus)', padding: '30px 34px 26px' }}>

        {/* header row: back + title */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 8 }}>
          <button onClick={onBack} aria-label="Back" title="Back" style={{
            display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
            width: 30, height: 30, borderRadius: 'var(--radius-sm)', cursor: 'pointer',
            background: 'transparent', border: '1px solid var(--border-default)',
            color: 'var(--text-secondary)', flex: 'none' }}>
            <Ico d={back} w={15} />
          </button>
          <div style={{ font: 'var(--weight-semibold) var(--text-xl)/1.1 var(--font-ui)',
            letterSpacing: 'var(--tracking-snug)', color: 'var(--text-primary)' }}>Model token</div>
        </div>
        <div style={{ font: 'var(--text-base)/var(--leading-normal) var(--font-ui)',
          color: 'var(--text-secondary)', marginBottom: 24, textWrap: 'pretty' }}>
          Replace the Claude token Loupe uses to read your diffs. It’s stored on this Mac only.
        </div>

        {/* current connection state */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 9, marginBottom: 22,
          padding: '11px 14px', borderRadius: 'var(--radius-md)',
          background: 'var(--surface-inset)', border: '1px solid var(--border-default)' }}>
          <span style={{ display: 'inline-flex', flex: 'none',
            color: connected ? 'var(--pass)' : 'var(--text-faint)' }}>
            <Ico d={connected ? check : dot} w={15} />
          </span>
          <span style={{ font: 'var(--weight-medium) var(--text-sm)/1 var(--font-ui)',
            color: connected ? 'var(--text-primary)' : 'var(--text-tertiary)' }}>
            {connected ? 'A model token is connected.' : 'No model token connected.'}
          </span>
        </div>

        <label style={labelStyle}>New token</label>
        <input type="password" value={token} placeholder="sk-ant-…"
          onChange={(e) => { setToken(e.target.value); setStatus(null); }}
          onKeyDown={(e) => { if (e.key === 'Enter' && !busy && token.trim().length >= 6) save(); }}
          style={{ ...fieldStyle, marginBottom: 14 }} />

        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <Button variant="primary" size="sm" disabled={busy || token.trim().length < 6}
            onClick={save}>{busy ? 'Saving…' : 'Save token'}</Button>
          {connected && (
            <Button variant="ghost" size="sm" disabled={busy}
              onClick={clear} style={{ color: 'var(--flag)' }}>Remove token</Button>
          )}
          {status && (
            <span style={{ marginLeft: 'auto', font: 'var(--text-sm)/1.4 var(--font-ui)',
              color: status.tone === 'ok' ? 'var(--pass)' : 'var(--flag)',
              textAlign: 'right', maxWidth: 220 }}>{status.text}</span>
          )}
        </div>

        {/* Open-in-editor preference — ⌘-click a diff line opens the project here. */}
        <div style={{ marginTop: 26, paddingTop: 22, borderTop: '1px solid var(--border-subtle)' }}>
          <label style={labelStyle}>Open in editor — ⌘-click a diff line</label>
          <div style={{ display: 'flex', gap: 8 }}>
            {[['auto', 'Auto'], ['idea', 'IntelliJ'], ['code', 'VS Code']].map(([v, l]) => (
              <button key={v} onClick={() => pickEditor(v)} style={{
                flex: 1, height: 36, borderRadius: 'var(--radius-md)', cursor: 'pointer',
                background: editor === v ? 'var(--accent-dim)' : 'var(--surface-inset)',
                border: `1px solid ${editor === v ? 'var(--accent-line)' : 'var(--border-default)'}`,
                color: editor === v ? 'var(--accent)' : 'var(--text-secondary)',
                font: 'var(--weight-medium) var(--text-sm)/1 var(--font-ui)',
                transition: 'var(--t-hover)' }}>{l}</button>
            ))}
          </div>
          <div style={{ marginTop: 9, font: 'var(--text-xs)/1.5 var(--font-ui)', color: 'var(--text-faint)' }}>
            에디터 CLI 런처가 필요해요 — VS Code: “Shell Command: Install ‘code’”, IntelliJ: “Tools › Create Command-line Launcher”.
          </div>
        </div>
      </div>
    </div>
  );
}
