/* Loupe UI kit — fake review data (cards + tree) for dev/demo.
   Exports { cards, tree }. Real syntax highlighting lives in ./highlight (Shiki). */

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
    summary: 'JSON 디코딩을 공용 헬퍼로 분리하고 본문 크기 제한을 추가합니다.',
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
    summary: '디코딩을 decodeJSON으로 일원화하고 요청 컨텍스트를 함께 전달합니다.',
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
    summary: '빈 토큰 검사 대신 실제 리스 만료 검사를 수행하도록 바꿉니다.',
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
    summary: '신규: 새 리스를 요청하고 다음 만료 시각을 저장합니다.',
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
    summary: '서명 키를 단순 교체가 아니라 쓰기 잠금 안에서 회전시킵니다.',
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
    summary: '감사 로그를 구조화된 필드 방식으로 전환합니다.',
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
    summary: '신규: 로그에 남기기 전에 이메일의 로컬 부분을 마스킹합니다.',
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
    summary: '인증 카운터에 결과(outcome) 라벨을 추가합니다.',
    lines: L({ _start: 33, lines: [
      ['del', '\tmetrics.Inc("auth.attempts")'],
      ['add', '\tmetrics.Inc("auth.attempts", "outcome", outcome)'],
    ]}),
  },
];

// Build a nested folder tree from the card paths (GitHub PR style).
// A file appears ONCE; if several cards touch it, they hang under the file
// as symbol entries. Each file aggregates its cards' +/- line counts.
export function buildTree(cards) {
  const root = { name: '', dirs: {}, files: {} };
  cards.forEach((c) => {
    const parts = c.path.split('/');
    const file = parts.pop();
    let node = root;
    parts.forEach((p) => { node.dirs[p] = node.dirs[p] || { name: p, dirs: {}, files: {} }; node = node.dirs[p]; });
    let add = 0, del = 0;
    c.lines.forEach((l) => { if (l.t === 'add') add++; else if (l.t === 'del') del++; });
    const entry = node.files[file] || (node.files[file] = { name: file, add: 0, del: 0, cards: [] });
    entry.add += add; entry.del += del;
    entry.cards.push({ id: c.id, status: c.status, symbol: c.symbol, add, del });
  });
  // turn files maps into arrays; collapse single-child directory chains
  const finalize = (node) => {
    Object.values(node.dirs).forEach(finalize);
    node.files = Object.values(node.files);
  };
  const collapse = (node) => {
    Object.values(node.dirs).forEach(collapse);
    const keys = Object.keys(node.dirs);
    if (node !== root && keys.length === 1 && Object.keys(node.files).length === 0) {
      const child = node.dirs[keys[0]];
      node.name = node.name + '/' + child.name;
      node.dirs = child.dirs; node.files = child.files;
    }
  };
  Object.values(root.dirs).forEach(collapse);
  finalize(root);
  return root;
}

// Demo tree built from the fixture cards (the running app builds its own from
// the real cards returned by the engine).
export const tree = buildTree(cards);
