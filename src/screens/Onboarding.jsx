/* Loupe — onboarding. One-time only: connect your Claude token.
   Loupe is a desktop app, so the token is saved once (App persists it via
   `save_token`) and reused across every project. Choosing a project folder +
   branches happens later, from the top-left project menu — not here.
   `props.onFinish(token)` hands the chosen token string up to App (the Advanced
   API key wins over the setup-token when both are present). */

import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '../components/Button';
import { KeyHint } from '../components/KeyHint';
import Logo from '../components/Logo';

export default function Onboarding(props) {
  const [token, setToken] = React.useState('');
  const [tested, setTested] = React.useState(false);
  const [testing, setTesting] = React.useState(false);
  const [testErr, setTestErr] = React.useState(null);
  const [copied, setCopied] = React.useState(false);
  const [advanced, setAdvanced] = React.useState(false);
  const [apiKey, setApiKey] = React.useState('');

  const Ico = ({ d, w = 16 }) => (
    <svg width={w} height={w} viewBox="0 0 24 24" fill="none" stroke="currentColor"
      strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d={d} /></svg>
  );
  const copy = 'M8 4v12a2 2 0 0 0 2 2h8M8 4a2 2 0 0 1 2-2h6.5L20 5.5V14a2 2 0 0 1-2 2H10a2 2 0 0 1-2-2z M8 4H6a2 2 0 0 0-2 2v12';
  const check = 'M20 6 9 17l-5-5';

  const fieldStyle = {
    width: '100%', height: 40, padding: '0 14px', borderRadius: 'var(--radius-md)',
    background: 'var(--surface-inset)', border: '1px solid var(--border-default)',
    color: 'var(--text-primary)', font: 'var(--text-base)/1 var(--font-ui)', outline: 'none',
    boxSizing: 'border-box', appearance: 'none',
  };
  const labelStyle = { font: 'var(--weight-medium) var(--text-xs)/1 var(--font-ui)',
    letterSpacing: 'var(--tracking-wide)', textTransform: 'uppercase',
    color: 'var(--text-tertiary)', marginBottom: 9, display: 'block' };

  // The value we hand up: the Advanced API key wins over the pasted setup-token.
  const chosen = (apiKey.trim() || token.trim());

  // Test connection: actually verify the chosen token with a minimal live model call
  // (backend `verify_token`). Success keeps the existing tested=true flow (enables
  // Continue); failure shows a red error and leaves tested=false.
  const test = async () => {
    setTesting(true);
    setTestErr(null);
    try {
      await invoke('verify_token', { token: chosen });
      setTested(true);
    } catch (e) {
      setTested(false);
      setTestErr(String(e));
    } finally {
      setTesting(false);
    }
  };

  return (
    <div data-tauri-drag-region style={{ position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column',
      alignItems: 'center', justifyContent: 'center', background: 'var(--bg-base)', padding: 24 }}>

      {/* wordmark */}
      <div style={{ marginBottom: 38 }}>
        <Logo size="sm" />
      </div>

      <div style={{ width: 480, background: 'var(--surface-card)',
        border: '1px solid var(--border-subtle)', borderRadius: 'var(--radius-xl)',
        boxShadow: 'var(--shadow-focus)', padding: '34px 34px 28px' }}>

        <div style={{ font: 'var(--weight-semibold) var(--text-xl)/1.1 var(--font-ui)',
          letterSpacing: 'var(--tracking-snug)', color: 'var(--text-primary)', marginBottom: 8 }}>Connect your model</div>
        <div style={{ font: 'var(--text-base)/var(--leading-normal) var(--font-ui)',
          color: 'var(--text-secondary)', marginBottom: 28, textWrap: 'pretty' }}>
          Loupe reads diffs through your own Claude token. You only need to do this once — it’s saved on this Mac and reused for every project.
        </div>

        <label style={labelStyle}>1 · Generate a token</label>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8,
          background: 'var(--surface-inset)', border: '1px solid var(--border-default)',
          borderRadius: 'var(--radius-md)', padding: '10px 10px 10px 14px', marginBottom: 20 }}>
          <code style={{ flex: 1, font: 'var(--code-base)/1.4 var(--font-mono)', color: 'var(--syn-plain)' }}>
            <span style={{ color: 'var(--text-faint)' }}>$ </span>claude setup-token
          </code>
          <Button size="sm" variant="secondary"
            icon={copied ? <span style={{ color: 'var(--pass)' }}><Ico d={check} w={14} /></span> : <Ico d={copy} w={14} />}
            onClick={() => { setCopied(true); setTimeout(() => setCopied(false), 1400); }}>
            {copied ? 'Copied' : 'Copy'}
          </Button>
        </div>

        <label style={labelStyle}>2 · Paste the token</label>
        <input type="password" value={token} placeholder="sk-ant-…"
          onChange={(e) => { setToken(e.target.value); setTested(false); setTestErr(null); }}
          style={{ ...fieldStyle, fontFamily: 'var(--font-mono)', marginBottom: 14 }} />
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          {/* Test connection makes one minimal live model call (backend `verify_token`)
             so a bad/expired token is caught here, before we persist it. Success gates
             Continue (tested=true); failure shows the error below and leaves it disabled. */}
          <Button variant={tested ? 'secondary' : 'primary'} size="sm"
            disabled={chosen.length < 6 || testing}
            onClick={test}>{testing ? 'Testing…' : 'Test connection'}</Button>
          {tested && !testing && (
            <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6,
              color: 'var(--pass)', font: 'var(--weight-medium) var(--text-sm)/1 var(--font-ui)' }}>
              <Ico d={check} w={15} /> Connected
            </span>
          )}
          {testErr && !testing && (
            <span style={{ font: 'var(--text-sm)/1.4 var(--font-ui)', color: 'var(--flag)',
              maxWidth: 280, textWrap: 'pretty' }}>{testErr}</span>
          )}
        </div>

        <div style={{ marginTop: 20, borderTop: '1px solid var(--border-subtle)', paddingTop: 16 }}>
          <button onClick={() => setAdvanced(!advanced)} style={{
            display: 'inline-flex', alignItems: 'center', gap: 6, background: 'transparent',
            border: 'none', cursor: 'pointer', padding: 0, color: 'var(--text-tertiary)',
            font: 'var(--weight-medium) var(--text-sm)/1 var(--font-ui)' }}>
            <span style={{ transform: advanced ? 'rotate(90deg)' : 'none',
              transition: 'transform var(--dur-fast) var(--ease-soft)', display: 'inline-flex' }}>
              <Ico d="M9 18l6-6-6-6" w={14} /></span>
            Advanced
          </button>
          {advanced && (
            <div style={{ marginTop: 14 }}>
              <label style={labelStyle}>API key (instead of a token)</label>
              <input type="password" value={apiKey} placeholder="sk-ant-api03-…"
                onChange={(e) => { setApiKey(e.target.value); setTested(false); setTestErr(null); }}
                style={{ ...fieldStyle, fontFamily: 'var(--font-mono)' }} />
            </div>
          )}
        </div>

        {/* footer */}
        <div style={{ display: 'flex', alignItems: 'center', marginTop: 30 }}>
          <span style={{ font: 'var(--text-sm)/1 var(--font-ui)', color: 'var(--text-faint)' }}>
            Next: pick a project from the menu
          </span>
          <div style={{ marginLeft: 'auto' }}>
            <Button variant="primary" disabled={!tested || chosen.length === 0}
              onClick={() => props.onFinish(chosen)}>Continue</Button>
          </div>
        </div>
      </div>

      <div style={{ marginTop: 26, opacity: 'var(--dim-rest)' }}>
        <KeyHint keys="⏎" label="continue" size="sm" />
      </div>
    </div>
  );
}
