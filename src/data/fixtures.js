/* Loupe UI kit — fake review data + a tiny Go syntax highlighter.
   Exports { cards, highlightGo }. */

import React from 'react';

const KW = new Set(['func','return','if','else','for','range','package','import','var',
  'const','type','struct','interface','map','chan','go','defer','switch','case','select','break','continue']);
const TY = new Set(['string','int','int64','uint','bool','error','byte','rune','float64',
  'context','Context','Time','Duration','Request','Response','ResponseWriter','Server',
  'Session','Logger','Server','Reader','Writer','http','json','time','metrics','sync']);
const CONST = new Set(['nil','true','false','iota']);

// Returns an array of styled React spans for one line of Go.
export function highlightGo(code) {
  const out = [];
  const re = /(\/\/[^\n]*)|(`[^`]*`|"(?:\\.|[^"\\])*")|(\b\d[\d_.xeE]*\b)|([A-Za-z_]\w*)|(\s+)|([^\sA-Za-z_0-9]+)/g;
  let m, key = 0;
  while ((m = re.exec(code)) !== null) {
    let color = 'var(--syn-plain)', italic = false, text = m[0];
    if (m[1] !== undefined) { color = 'var(--syn-comment)'; italic = true; }
    else if (m[2] !== undefined) color = 'var(--syn-string)';
    else if (m[3] !== undefined) color = 'var(--syn-number)';
    else if (m[4] !== undefined) {
      const w = m[4];
      const after = code.slice(re.lastIndex).match(/^\s*\(/);
      if (KW.has(w)) color = 'var(--syn-keyword)';
      else if (CONST.has(w)) color = 'var(--syn-const)';
      else if (TY.has(w)) color = 'var(--syn-type)';
      else if (after) color = 'var(--syn-func)';
      else color = 'var(--syn-plain)';
    } else if (m[6] !== undefined) color = 'var(--syn-punct)';
    out.push(React.createElement('span', { key: key++, style: { color, fontStyle: italic ? 'italic' : 'normal' } }, text));
  }
  return out;
}

// helper to build diff lines: ['ctx', code] | ['add', code] | ['del', code]
const L = (rows) => {
  let n = rows._start || 40;
  return rows.lines.map((r) => {
    const [t, c] = r;
    const line = { t, c, n: t === 'del' ? n : n };
    if (t !== 'del') n += 1;
    return line;
  });
};

export const cards = [
  {
    id: 'decodeJSON', chapter: 'Request intake', symbol: 'decodeJSON',
    path: 'internal/api/decode.go', status: 'pass',
    summary: 'Extracts JSON decoding into a shared helper with a body-size limit.',
    lines: L({ _start: 18, lines: [
      ['ctx', '// decodeJSON reads a JSON body with a fixed size limit.'],
      ['ctx', 'func decodeJSON(body io.Reader, v any) error {'],
      ['del', '\tdec := json.NewDecoder(body)'],
      ['add', '\tlimited := io.LimitReader(body, maxBodyBytes)'],
      ['add', '\tdec := json.NewDecoder(limited)'],
      ['add', '\tdec.DisallowUnknownFields()'],
      ['ctx', '\treturn dec.Decode(v)'],
      ['ctx', '}'],
    ]}),
  },
  {
    id: 'handleLogin', chapter: 'Request intake', symbol: 'Server.handleLogin',
    path: 'internal/api/auth.go', status: 'pass',
    summary: 'Routes decoding through decodeJSON and forwards the request context.',
    lines: L({ _start: 52, lines: [
      ['ctx', 'func (s *Server) handleLogin(w http.ResponseWriter, r *http.Request) {'],
      ['ctx', '\tvar req loginRequest'],
      ['del', '\tif err := json.NewDecoder(r.Body).Decode(&req); err != nil {'],
      ['add', '\tif err := decodeJSON(r.Body, &req); err != nil {'],
      ['ctx', '\t\twriteError(w, http.StatusBadRequest, err)'],
      ['ctx', '\t\treturn'],
      ['ctx', '\t}'],
      ['add', '\tsess, err := s.auth.Start(r.Context(), req.Email, req.Password)'],
      ['del', '\tsess, err := s.auth.Start(req.Email, req.Password)'],
      ['ctx', '\tif err != nil {'],
      ['ctx', '\t\twriteError(w, http.StatusUnauthorized, err)'],
      ['ctx', '\t}'],
    ]}),
  },
  {
    id: 'validate', chapter: 'Authentication', symbol: 'Session.Validate',
    path: 'internal/auth/session.go', status: 'active',
    summary: 'Replaces the empty-token guard with a real lease-expiry check.',
    lines: L({ _start: 41, lines: [
      ['ctx', '// Validate refreshes the lease before the session is used.'],
      ['ctx', 'func (s *Session) Validate(ctx context.Context) error {'],
      ['del', '\tif s.token == "" {'],
      ['add', '\tif s.expired(now()) {'],
      ['add', '\t\tif err := s.refreshLease(ctx); err != nil {'],
      ['add', '\t\t\treturn ErrLeaseExpired'],
      ['add', '\t\t}'],
      ['ctx', '\t}'],
      ['ctx', '\treturn nil'],
      ['ctx', '}'],
    ]}),
  },
  {
    id: 'refreshLease', chapter: 'Authentication', symbol: 'Session.refreshLease',
    path: 'internal/auth/session.go', status: 'pending',
    summary: 'New: requests a fresh lease and stores the next expiry.',
    lines: L({ _start: 58, lines: [
      ['add', 'func (s *Session) refreshLease(ctx context.Context) error {'],
      ['add', '\tlease, err := s.broker.Acquire(ctx, s.id)'],
      ['add', '\tif err != nil {'],
      ['add', '\t\treturn fmt.Errorf("refresh lease: %w", err)'],
      ['add', '\t}'],
      ['add', '\ts.expiresAt = lease.Until'],
      ['add', '\treturn nil'],
      ['add', '}'],
    ]}),
  },
  {
    id: 'rotateKey', chapter: 'Authentication', symbol: 'rotateKey',
    path: 'internal/auth/keys.go', status: 'pending',
    summary: 'Rotates the signing key under a write lock instead of a bare swap.',
    lines: L({ _start: 90, lines: [
      ['ctx', 'func rotateKey(store *KeyStore) error {'],
      ['del', '\tstore.active = store.next'],
      ['add', '\tstore.mu.Lock()'],
      ['add', '\tdefer store.mu.Unlock()'],
      ['add', '\tstore.previous = store.active'],
      ['add', '\tstore.active = store.next'],
      ['ctx', '\treturn store.persist()'],
      ['ctx', '}'],
    ]}),
  },
  {
    id: 'logAttempt', chapter: 'Logging', symbol: 'logAttempt',
    path: 'internal/log/audit.go', status: 'pending',
    summary: 'Switches audit logging to structured fields.',
    lines: L({ _start: 12, lines: [
      ['ctx', 'func logAttempt(l *Logger, email string, ok bool) {'],
      ['del', '\tl.Printf("login %s ok=%v", email, ok)'],
      ['add', '\tl.Info("login_attempt",'],
      ['add', '\t\t"email", redactPII(email),'],
      ['add', '\t\t"success", ok,'],
      ['add', '\t)'],
      ['ctx', '}'],
    ]}),
  },
  {
    id: 'redactPII', chapter: 'Logging', symbol: 'redactPII',
    path: 'internal/log/redact.go', status: 'pending',
    summary: 'New: masks the local part of an email before it is logged.',
    lines: L({ _start: 4, lines: [
      ['add', '// redactPII masks the local part of an email address.'],
      ['add', 'func redactPII(email string) string {'],
      ['add', '\tat := strings.IndexByte(email, \'@\')'],
      ['add', '\tif at <= 1 {'],
      ['add', '\t\treturn "***"'],
      ['add', '\t}'],
      ['add', '\treturn email[:1] + "***" + email[at:]'],
      ['add', '}'],
    ]}),
  },
  {
    id: 'metricsInc', chapter: 'Logging', symbol: 'metrics.Inc',
    path: 'internal/metrics/counter.go', status: 'pending',
    summary: 'Adds an outcome label to the auth counter.',
    lines: L({ _start: 33, lines: [
      ['del', '\tmetrics.Inc("auth.attempts")'],
      ['add', '\tmetrics.Inc("auth.attempts", "outcome", outcome)'],
    ]}),
  },
];
