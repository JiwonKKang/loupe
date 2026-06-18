/* Loupe UI kit — onboarding (3 quiet steps on an empty canvas). */

import React from 'react';
import { Button } from '../components/Button';
import { KeyHint } from '../components/KeyHint';

export default function Onboarding(props) {
  const [step, setStep] = React.useState(0);
  const [repo, setRepo] = React.useState('');
  const [token, setToken] = React.useState('');
  const [tested, setTested] = React.useState(false);
  const [copied, setCopied] = React.useState(false);
  const [base, setBase] = React.useState('main');
  const [target, setTarget] = React.useState('agent/refactor-auth');
  const [advanced, setAdvanced] = React.useState(false);
  const [apiKey, setApiKey] = React.useState('');

  const recents = ['monorepo / api', 'edge-proxy', 'billing-worker'];

  const Ico = ({ d, w = 16 }) => (
    <svg width={w} height={w} viewBox="0 0 24 24" fill="none" stroke="currentColor"
      strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d={d} /></svg>
  );
  const folder = 'M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2z';
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

  const steps = [
    { k: 'Open repository', sub: 'Point Loupe at the working tree you want to review.' },
    { k: 'Connect your model', sub: 'Loupe reads the diff through your own Claude token.' },
    { k: 'Choose the range', sub: 'Compare a target branch against its base.' },
  ];
  const cur = steps[step];
  const canNext = step === 0 ? repo.trim().length > 0 : step === 1 ? tested : true;

  return (
    <div style={{ position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column',
      alignItems: 'center', justifyContent: 'center', background: 'var(--bg-base)', padding: 24 }}>

      {/* wordmark */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 9, marginBottom: 38 }}>
        <span style={{ width: 11, height: 11, borderRadius: 999, background: 'var(--accent)',
          boxShadow: '0 0 0 5px var(--accent-dim)' }} />
        <span style={{ font: 'var(--weight-semibold) var(--text-md)/1 var(--font-ui)',
          letterSpacing: 'var(--tracking-snug)', color: 'var(--text-primary)' }}>Loupe</span>
      </div>

      <div style={{ width: 480, background: 'var(--surface-card)',
        border: '1px solid var(--border-subtle)', borderRadius: 'var(--radius-xl)',
        boxShadow: 'var(--shadow-focus)', padding: '32px 34px 26px' }}>

        {/* stepper */}
        <div style={{ display: 'flex', gap: 6, marginBottom: 26 }}>
          {steps.map((_, i) => (
            <div key={i} style={{ flex: 1, height: 3, borderRadius: 999,
              background: i <= step ? 'var(--accent)' : 'var(--border-default)',
              transition: 'background var(--dur-base) var(--ease-soft)' }} />
          ))}
        </div>

        <div style={{ font: 'var(--weight-semibold) var(--text-xl)/1.1 var(--font-ui)',
          letterSpacing: 'var(--tracking-snug)', color: 'var(--text-primary)', marginBottom: 8 }}>{cur.k}</div>
        <div style={{ font: 'var(--text-base)/var(--leading-normal) var(--font-ui)',
          color: 'var(--text-secondary)', marginBottom: 26, textWrap: 'pretty' }}>{cur.sub}</div>

        {step === 0 && (
          <div>
            <div style={{ ...fieldStyle, height: 48, display: 'flex', alignItems: 'center',
              gap: 11, color: 'var(--text-secondary)', padding: '0 14px' }}>
              <span style={{ color: 'var(--accent)', flex: 'none' }}><Ico d={folder} /></span>
              <input
                value={repo}
                onChange={(e) => setRepo(e.target.value)}
                placeholder="/path/to/repo"
                style={{ flex: 1, background: 'transparent', border: 'none', outline: 'none',
                  color: 'var(--text-primary)', font: 'var(--text-base)/1 var(--font-mono)' }}
              />
            </div>
            <div style={{ ...labelStyle, marginTop: 22 }}>Recent</div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
              {recents.map((r) => (
                <button key={r} onClick={() => setRepo(r)} style={{
                  display: 'flex', alignItems: 'center', gap: 11, padding: '10px 12px',
                  borderRadius: 'var(--radius-md)', cursor: 'pointer', textAlign: 'left',
                  background: repo === r ? 'var(--accent-dim)' : 'transparent',
                  border: `1px solid ${repo === r ? 'var(--accent-line)' : 'transparent'}`,
                  font: 'var(--text-base)/1 var(--font-mono)',
                  color: repo === r ? 'var(--text-primary)' : 'var(--text-secondary)' }}>
                  <span style={{ color: repo === r ? 'var(--accent)' : 'var(--text-faint)' }}><Ico d={folder} w={15} /></span>
                  {r}
                </button>
              ))}
            </div>
          </div>
        )}

        {step === 1 && (
          <div>
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
              onChange={(e) => { setToken(e.target.value); setTested(false); }}
              style={{ ...fieldStyle, fontFamily: 'var(--font-mono)', marginBottom: 14 }} />
            <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
              <Button variant={tested ? 'secondary' : 'primary'} size="sm"
                disabled={token.length < 6}
                onClick={() => setTested(true)}>Test connection</Button>
              {tested && (
                <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6,
                  color: 'var(--pass)', font: 'var(--weight-medium) var(--text-sm)/1 var(--font-ui)' }}>
                  <Ico d={check} w={15} /> Connected
                </span>
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
                    onChange={(e) => setApiKey(e.target.value)}
                    style={{ ...fieldStyle, fontFamily: 'var(--font-mono)' }} />
                </div>
              )}
            </div>
          </div>
        )}

        {step === 2 && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
            <div>
              <label style={labelStyle}>Base</label>
              <input value={base} onChange={(e) => setBase(e.target.value)}
                placeholder="main"
                style={{ ...fieldStyle, fontFamily: 'var(--font-mono)' }} />
            </div>
            <div>
              <label style={labelStyle}>Target</label>
              <input value={target} onChange={(e) => setTarget(e.target.value)}
                placeholder="feature/branch"
                style={{ ...fieldStyle, fontFamily: 'var(--font-mono)' }} />
            </div>
            <div style={{ font: 'var(--text-sm)/1.5 var(--font-ui)', color: 'var(--text-tertiary)' }}>
              Loupe compares {target || 'target'} against {base || 'base'}, one symbol at a time.
            </div>
          </div>
        )}

        {/* footer */}
        <div style={{ display: 'flex', alignItems: 'center', marginTop: 30 }}>
          {step > 0
            ? <Button variant="ghost" size="sm" onClick={() => setStep(step - 1)}>Back</Button>
            : <span />}
          <div style={{ marginLeft: 'auto' }}>
            {step < 2
              ? <Button variant="primary" disabled={!canNext} onClick={() => setStep(step + 1)}>Continue</Button>
              : <Button variant="primary" onClick={() => props.onFinish({ repoPath: repo.trim(), base: base.trim(), target: target.trim() })}>Start review</Button>}
          </div>
        </div>
      </div>

      <div style={{ marginTop: 26, opacity: 'var(--dim-rest)' }}>
        <KeyHint keys="⏎" label="continue" size="sm" />
      </div>
    </div>
  );
}
