/* Loupe UI kit — onboarding (3 quiet steps on an empty canvas). */

import React from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '../components/Button';
import { KeyHint } from '../components/KeyHint';
import Logo from '../components/Logo';

export default function Onboarding(props) {
  const [step, setStep] = React.useState(0);
  const [repo, setRepo] = React.useState('');
  const [token, setToken] = React.useState('');
  const [tested, setTested] = React.useState(false);
  const [copied, setCopied] = React.useState(false);
  const [base, setBase] = React.useState('');
  const [target, setTarget] = React.useState('');
  const [advanced, setAdvanced] = React.useState(false);
  const [apiKey, setApiKey] = React.useState('');

  // Branch loading state for the chosen repo.
  const [branches, setBranches] = React.useState([]);   // string[]
  const [branchLoading, setBranchLoading] = React.useState(false);
  const [branchError, setBranchError] = React.useState(null);
  const [picking, setPicking] = React.useState(false);  // native dialog open

  const Ico = ({ d, w = 16 }) => (
    <svg width={w} height={w} viewBox="0 0 24 24" fill="none" stroke="currentColor"
      strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d={d} /></svg>
  );
  const folder = 'M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2z';
  const copy = 'M8 4v12a2 2 0 0 0 2 2h8M8 4a2 2 0 0 1 2-2h6.5L20 5.5V14a2 2 0 0 1-2 2H10a2 2 0 0 1-2-2z M8 4H6a2 2 0 0 0-2 2v12';
  const check = 'M20 6 9 17l-5-5';
  const branch = 'M6 3v12M18 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6zm12-9a9 9 0 0 1-9 9';
  const chevron = 'M6 9l6 6 6-6';
  const alert = 'M12 9v4M12 17h.01M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z';

  const fieldStyle = {
    width: '100%', height: 40, padding: '0 14px', borderRadius: 'var(--radius-md)',
    background: 'var(--surface-inset)', border: '1px solid var(--border-default)',
    color: 'var(--text-primary)', font: 'var(--text-base)/1 var(--font-ui)', outline: 'none',
    boxSizing: 'border-box', appearance: 'none',
  };
  const labelStyle = { font: 'var(--weight-medium) var(--text-xs)/1 var(--font-ui)',
    letterSpacing: 'var(--tracking-wide)', textTransform: 'uppercase',
    color: 'var(--text-tertiary)', marginBottom: 9, display: 'block' };

  // Native folder picker → set repo + load its branches. Defensive: in a plain
  // browser (no Tauri) `open` throws; we surface a gentle error instead of crashing.
  const pickFolder = async () => {
    setPicking(true);
    try {
      const dir = await open({ directory: true, multiple: false, title: 'Select a git repository' });
      if (typeof dir === 'string' && dir.length > 0) {
        setRepo(dir);
        await loadBranches(dir);
      }
      // null (cancelled) or array → ignore.
    } catch (e) {
      setBranchError('폴더를 열 수 없어요. Tauri 앱에서 실행 중인지 확인해 주세요.');
    } finally {
      setPicking(false);
    }
  };

  // Read the chosen repo's branches and seed base/target defaults.
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
        setBase('');
        setTarget('');
        return;
      }
      // base default = response.default (else first branch).
      const nextBase = res.default && list.includes(res.default) ? res.default : list[0];
      // target default = response.current (else a branch different from base).
      let nextTarget = res.current && list.includes(res.current) ? res.current : null;
      if (!nextTarget || nextTarget === nextBase) {
        nextTarget = list.find((b) => b !== nextBase) || nextBase;
      }
      setBase(nextBase);
      setTarget(nextTarget);
    } catch (e) {
      setBranchError('git 저장소가 아니거나 브랜치를 읽을 수 없어요.');
      setBranches([]);
      setBase('');
      setTarget('');
    } finally {
      setBranchLoading(false);
    }
  };

  const steps = [
    { k: 'Open repository', sub: 'Point Loupe at the working tree you want to review.' },
    { k: 'Connect your model', sub: 'Loupe reads the diff through your own Claude token.' },
    { k: 'Choose the range', sub: 'Compare a target branch against its base.' },
  ];
  const cur = steps[step];
  // Step 0 needs a repo with at least one readable branch; step 1 needs a tested
  // token; step 2 (range) needs both dropdowns populated.
  const canNext = step === 0
    ? repo.trim().length > 0 && branches.length > 0 && !branchLoading
    : step === 1
      ? tested
      : base.length > 0 && target.length > 0;

  // Shared <select> styling — same tokens/feel as fieldStyle, with a chevron.
  const selectWrap = { position: 'relative' };
  const selectStyle = {
    ...fieldStyle, fontFamily: 'var(--font-mono)', paddingRight: 38, cursor: 'pointer',
  };
  const chevronStyle = {
    position: 'absolute', right: 12, top: '50%', transform: 'translateY(-50%)',
    pointerEvents: 'none', color: 'var(--text-tertiary)', display: 'inline-flex',
  };
  const BranchSelect = ({ value, onChange, disabled }) => (
    <div style={selectWrap}>
      <select value={value} onChange={(e) => onChange(e.target.value)}
        disabled={disabled} style={{ ...selectStyle, opacity: disabled ? 0.5 : 1 }}>
        {branches.map((b) => <option key={b} value={b}>{b}</option>)}
      </select>
      <span style={chevronStyle}><Ico d={chevron} w={15} /></span>
    </div>
  );

  return (
    <div style={{ position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column',
      alignItems: 'center', justifyContent: 'center', background: 'var(--bg-base)', padding: 24 }}>

      {/* wordmark */}
      <div style={{ marginBottom: 38 }}>
        <Logo size="sm" />
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
            {/* Native folder picker — the only way to set the repo path. */}
            <Button variant="secondary" fullWidth
              icon={<span style={{ color: 'var(--accent)' }}><Ico d={folder} /></span>}
              disabled={picking}
              onClick={pickFolder}>
              {picking ? 'Opening…' : (repo ? 'Choose a different folder' : 'Browse for a folder')}
            </Button>

            {/* Selected path, read as monospace once chosen. */}
            {repo && (
              <div style={{ ...fieldStyle, height: 48, display: 'flex', alignItems: 'center',
                gap: 11, color: 'var(--text-secondary)', padding: '0 14px', marginTop: 12 }}>
                <span style={{ color: 'var(--accent)', flex: 'none' }}><Ico d={folder} /></span>
                <span style={{ flex: 1, color: 'var(--text-primary)',
                  font: 'var(--text-base)/1 var(--font-mono)', overflow: 'hidden',
                  textOverflow: 'ellipsis', whiteSpace: 'nowrap', direction: 'rtl', textAlign: 'left' }}>
                  {repo}
                </span>
              </div>
            )}

            {/* Branch read feedback. */}
            {branchLoading && (
              <div style={{ marginTop: 14, font: 'var(--text-sm)/1.5 var(--font-ui)',
                color: 'var(--text-tertiary)', display: 'flex', alignItems: 'center', gap: 8 }}>
                <span style={{ color: 'var(--accent)' }}><Ico d={branch} w={15} /></span>
                Reading branches…
              </div>
            )}
            {!branchLoading && branchError && (
              <div style={{ marginTop: 14, display: 'flex', alignItems: 'flex-start', gap: 8,
                font: 'var(--text-sm)/1.5 var(--font-ui)', color: 'var(--flag)' }}>
                <span style={{ flex: 'none', marginTop: 1 }}><Ico d={alert} w={15} /></span>
                {branchError}
              </div>
            )}
            {!branchLoading && !branchError && branches.length > 0 && (
              <div style={{ marginTop: 14, display: 'flex', alignItems: 'center', gap: 8,
                font: 'var(--text-sm)/1.5 var(--font-ui)', color: 'var(--text-tertiary)' }}>
                <span style={{ color: 'var(--pass)' }}><Ico d={check} w={15} /></span>
                {branches.length} {branches.length === 1 ? 'branch' : 'branches'} found.
              </div>
            )}
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
              <BranchSelect value={base} onChange={setBase} disabled={branches.length === 0} />
            </div>
            <div>
              <label style={labelStyle}>Target</label>
              <BranchSelect value={target} onChange={setTarget} disabled={branches.length === 0} />
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
              : <Button variant="primary" disabled={!canNext}
                  onClick={() => props.onFinish({ repoPath: repo.trim(), base: base.trim(), target: target.trim(),
                    token: (apiKey.trim() || token.trim()) })}>Start review</Button>}
          </div>
        </div>
      </div>

      <div style={{ marginTop: 26, opacity: 'var(--dim-rest)' }}>
        <KeyHint keys="⏎" label="continue" size="sm" />
      </div>
    </div>
  );
}
