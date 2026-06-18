I now have authoritative facts to verify the design's API claims. Let me note the key findings against the design and complete the adversarial review. I have everything I need.

Key verified facts: model IDs `claude-haiku-4-5` and `claude-sonnet-4-6` are valid. `output_config.format` with `json_schema` is correct (not `output_config.format = {type: "json_schema", schema}` exactly, but close). Structured outputs supported on Haiku 4.5 and Sonnet 4.6. Assistant prefill returns 400 on both. `effort` param errors on Haiku 4.5 (and Sonnet 4.5) but works on Sonnet 4.6. Streaming required for max_tokens > ~16K.

Now compiling the adversarial review.

---

# 적대적 리뷰: Loupe v2 Stage-2 설계서

코드/기획/Anthropic API 사실 대조 완료. 검증 결과부터: TAGS_QUERY 상수는 3개 크레이트 모두 실존(`tree_sitter_{rust,go,java}::TAGS_QUERY`). `@reference.*` 캡처는 **언어마다 다름** — 설계 주장과 불일치(아래 BLOCKER). 모델 ID `claude-haiku-4-5`/`claude-sonnet-4-6` 유효. 구조화 출력은 Haiku 4.5/Sonnet 4.6 지원. prefill 400, effort는 Haiku 미지원 — 설계가 맞게 반영함.

## BLOCKER

**B1. `@reference.*` 캡처 셋이 언어마다 다르다 — 관계신호 토대(②)와 결정성이 깨진다.**
설계 §0/§5는 "`@reference.call/.type/.class` 존재(Go/Java/Rust)"라 단언하나 실제 tags.scm은:
- Rust: `@reference.call`, `@reference.implementation` (`.type`/`.class` **없음**)
- Go: `@reference.call`, `@reference.type` (`.class`/`.implementation` 없음)
- Java: `@reference.call`, `@reference.class`, `@reference.implementation` (`.type` 없음)

→ `extract_with_refs`가 단일 코드패스로 `.type`/`.class`를 모든 언어에서 뽑는다는 전제가 거짓. `RefHit{calls, type_refs}` 구조는 Rust에서 type_ref가 0, Go에서 class 관계 0이 되어 **언어별로 관계신호 품질이 비대칭**. signature 타입 관계(Request→Command→Entity)는 v2 §4.4 강한 관계의 핵심인데 Rust에서 누락. **수정**: 언어별 캡처 매핑 테이블을 명시(`definition_captures`처럼)하고, 부족분은 tree-sitter `type_identifier`/`scoped_type_identifier` 노드 직접 질의로 보강하거나, "type_ref는 Go/Java만, Rust는 call+impl만" 식으로 신호 비대칭을 설계에 명문화. ②단계 검증 기준("strong/weak 쌍 출력")이 언어별로 달라짐을 dearday(Go/Java 추정) 외 Rust 케이스로도 검증.

**B2. 단계 분해가 "독립 검증 가능"하지 않다 — ①이 진짜 먼저 검증되지 않는다.**
설계는 "각 단계는 이전 단계 없이 `cargo test`/dearday로 검증 가능"이라 하나:
- ①(AI 토대)이 "더미 cluster card 1개를 Haiku에 보내 JSON 수신"으로 검증되려면 `ClusterCardInput` 타입(③)과 스키마(prompts.rs)가 이미 있어야 한다. ①의 산출물에 `ClusterCardInput` 정의가 없으면 더미를 못 만든다. → ①과 ③이 순환 의존.
- ⑤(검증)의 단위테스트("환각 id 주입 시 reject")는 ④의 `ClusterOrderOut` 타입과 화이트리스트 소스가 필요. 
- 오케스트레이터 `cluster.rs`가 ④~⑦,⑨,⑩을 "마지막 통합"으로 묶는다고 명시 → ①~⑩이 **cluster.rs 없이는 end-to-end 검증 불가**. 즉 "독립 검증"은 단위 레벨에서만 참이고 dearday 실측은 후반에 몰린다.
**수정**: ①의 검증 기준을 "하드코딩된 JSON 문자열 1개를 reqwest로 POST → 200 + stop_reason 파싱"으로 축소(타입 의존 제거). 타입 계약(`ClusterCardInput`/`ClusterOrderOut`)을 ① 앞 "단계 0: 데이터모델 + serde 계약"으로 분리해 ①~⑩ 공통 토대로 못박기. 그래야 "AI 연동 토대가 진짜 먼저"가 성립.

**B3. OAuth setup-token이 죽은 코드인데 모듈/타입/feature가 설계 전반에 박혀 있다 — 스코프 과설계.**
설계 스스로 "공개 Messages API가 거부 + ToS 위반, 기본 off, onboarding은 API 키 입력으로 안내"라 결론냈다. 그렇다면 MVP에서 `OAuthProvider` 구조체, `#[cfg(feature="oauth-unofficial")]`, "헤더 부착 함수 하나로 격리", `anthropic-beta: oauth-2025-04-20`, user-agent 위조까지 코드 자리를 잡을 이유가 없다. 검증된 사실: setup-token(`sk-ant-oat01-`)은 **환경 워커/`ant auth login` 프로파일용 OAuth 토큰**이며 공개 `/v1/messages`에 BYO로 꽂는 정식 경로가 아님. 기획 §426의 "onboarding setup-token → API 직접 호출(BYO도 동일 경로)"는 **사실과 다른 기획 전제**이고, 설계가 이를 "BYO x-api-key로 번역"한 건 옳으나 OAuth 잔재를 남긴 게 문제. **수정**: OAuth 전면 삭제(trait는 `ApiKeyProvider` 단일 구현 + `LlmProvider` 추상화만 남겨 미래 확장 여지 확보). 기획의 setup-token 전제도 폐기 표시.

## MAJOR

**M1. `output_config.format` 스키마 형태가 부정확 + 미지원 제약을 "안 넣으면 됨"으로 넘김.**
설계 §4.2 "`output_config.format = { type: "json_schema", schema }`"는 방향은 맞으나 정확한 호출 형태는 `output_config: {format: {type: "json_schema", schema: {...}}}`이고 `client.messages.parse()` 사용이 권장. 검증된 미지원: 재귀/`minimum`/`maxLength`/`minItems` 등 — 설계가 "처음부터 안 넣음"으로 회피한 건 맞다. 그러나 **`additionalProperties:false`가 모든 object에 필수**이고 누락 시 컴파일 거부 → AI2의 `orderedByCluster: {clusterId: [cardId]}` 같은 **동적 키 맵은 json_schema로 표현 불가**(고정 properties만). 이게 BLOCKER 직전. **수정**: 동적 맵을 `[{clusterId, cardIds:[...]}]` 배열로 평탄화(스키마 가능형). "첫 요청 스키마 컴파일 1회 비용 + 24h 캐시" 레이턴시도 명시.

**M2. tauri async 커맨드 + reqwest 런타임 가정에 구멍.**
`async fn` 커맨드는 tauri `async_runtime`(tokio) 위에서 돈다는 주장은 맞다. 그러나 (a) `prewarm_analysis`가 `run_in_background`로 SQLite WAL 쓰기 + 메인 `load_review`(현재 **sync** 커맨드, lib.rs 확인됨)가 동시에 cache.db를 읽으면 rusqlite 연결을 커맨드마다 새로 여는 구조라야 안전한데 설계에 연결 풀/`Mutex<Connection>` 관리가 없음. (b) `load_review`를 sync→async로 바꾸면 기존 즉시반환 계약이 바뀜 — 설계는 "load_review 유지(캐시 히트면 cluster 포함)"라며 sync인 채 SQLite를 읽겠다는 건데, sync 커맨드는 tauri가 별도 스레드풀에서 호출하므로 가능하나 **reqwest await는 불가** → 캐시 미스 시 cluster 없이 반환됨을 UI가 처리해야 함(설계엔 이 분기 누락). **수정**: SQLite 연결을 `State<Mutex<Connection>>`로 관리 + WAL `busy_timeout` 설정 명시. `load_review`는 sync 유지(cache-only), AI는 전부 `analyze_review`/`prewarm`(async)로 분리하고 UI 분기 명문화.

**M3. SHA 노출이 `diff_three_dot` 시그니처만 바꿔선 부족 — base는 merge-base이지 base-ref가 아니다.**
gitdiff.rs 확인: 3-dot diff의 base_tree는 `merge_base(base_oid, target_oid)`의 트리다. 설계 캐시키 `base_sha`가 "resolve_commit가 노출한 SHA"라면 그건 base 브랜치 tip이지 merge-base가 아님. **diff 내용을 결정하는 건 merge-base SHA + target SHA**. base 브랜치 tip이 바뀌어도 merge-base가 같으면 diff는 동일 → tip을 키로 쓰면 불필요한 캐시 미스. 반대로 키 의도("3-dot은 base에 의존")를 살리려면 **merge_base_oid를 노출**해야 정확. **수정**: 캐시키를 `(repo, merge_base_sha, head_sha, schema_ver)`로. `card_hash`가 심볼 diff 본문 해시를 포함하므로 사실 merge_base/head 둘 다 card_hash에 흡수되어 layout 키만 정확하면 됨 — base_sha 필드의 의미를 merge-base로 재정의.

**M4. 화이트리스트 검증에 실제 구멍 — title/summary 텍스트 환각은 안 막힌다.**
설계 §4.5가 스스로 인정: "title/summary가 입력에 없는 심볼을 언급해도 텍스트 검증은 어렵다". 기획 §8.3은 "title/summary/orderedSymbols가 입력에 없는 symbol을 언급하면 버림"을 요구 → **기획 요구 대비 미달**. orderedCardIds 집합 검증만으론 AI3가 "CouponPolicy를 호출하는 PaymentGateway"처럼 없는 심볼을 요약문에 지어내는 걸 못 잡음. 리뷰어 신뢰 직결. **수정**: summary에 등장하는 식별자 토큰을 입력 심볼 name 집합과 대조하는 **느슨한 토큰 검증**(코드 식별자 패턴만 추출해 화이트리스트 교집합 검사, 자연어는 통과) 추가. 완벽 불가하므로 "단정 금지" 프롬프트 + 이 토큰 검증을 2중으로. 못 막는 부분은 리스크로 명문화.

**M5. 관계신호 과설계 — 튜닝지옥 진입 신호.**
hub 이름셋 하드코딩(`{Logger, DateUtils,...}` 11개) + `fan-in≥4` 임계 + `strong top-K=8` + test→impl strong + import-only weak + medium 쌍 미생성. 파라미터가 5개+. 기획 §4.4가 명시적으로 경고한 "community detection 튜닝지옥"을 이름매칭 휴리스틱으로 옮겼을 뿐 파라미터 수는 비슷. dearday 한 레포로 fan-in=4, top-K=8을 정당화할 수 없음(과적합). **수정**: MVP는 **strong/weak 2종 + hub 이름셋만**으로 시작. fan-in 임계·top-K는 "관계가 AI 입력 토큰을 터뜨릴 때만" 도입하는 후속으로 미루기. 기획도 "강/중/약→strong/weak로 단순화" 정도만 요구하니 신호를 더 줄여도 기획 위반 아님. 결정성은 순수함수+head SHA로 이미 보장되므로 파라미터 줄여도 캐시 정합 유지.

## MINOR

**m1. 데이터모델/프론트 계약 불일치 잠재.** model.rs는 `ReviewLine`에 단축키 `n/t/c`를 쓰고 `ReviewData`엔 camelCase 미적용 상태(현재 `cards`만). 설계는 `ReviewData`에만 `#[serde(rename_all="camelCase")]` 부여 + "기존 `n/t/c`는 영향 없음"이라 하나, rename_all은 구조체 단위라 `ReviewLine`엔 영향 없어 맞다. 단 `ReviewCard`에 신규 필드(`cluster_id`, `change_type` 등) 추가 시 **`ReviewCard`에도 camelCase를 따로 부여**해야 `clusterId`로 나감 — 설계가 `ReviewData`에만 부여한다고 읽히므로 `ReviewCard`/`Cluster`/`JitDefinition` 각각 부여 명시 필요. 또 App.jsx는 `data.cards`만 읽으므로(확인됨) 신규 필드 무시는 안전하나, `spineItems`가 `c.chapter` 의존(App.jsx:54) → `clusterId` 전환 시 chapter 필드 제거하면 깨짐. chapter는 유지하라(설계도 유지로 보이나 명문화).

**m2. 큰 PR 토큰/rate limit 대비가 추상적.** "한도 초과 시 fallback"만 있고 사전 토큰 카운트 없음. `count_tokens` API로 ClusterCardInput 직렬화 크기를 미리 재 SMALL_PR_SYMBOLS(12) 분기 외에 **토큰 상한 분기**를 둬야 함. Haiku 200K 입력이지만 배치 label_step(Sonnet streaming)에서 클러스터 N개를 1호출로 묶으면 출력 64K 한도. rate limit은 BYO 키 tier 의존 — 백그라운드 prewarm이 429 맞으면 `retry-after` 존중(SDK 아니므로 직접 처리) 명시 필요.

**m3. JIT overview의 changed_methods 교집합 계산 비용.** "이번 PR에서 바뀐 메서드 = changed_symbol과 교집합"은 클래스 overview마다 changed 심볼 전체 스캔 → O(클래스수 × changed). 작은 PR이라 무해하나 SymbolIndex에 path별 인덱스 전제 명시.

**m4. `qualified == name` 현재 구현(symbols.rs:144) → 설계의 "OrderService.calculatePrice" 정규화는 신규 구현 부담.** 메서드의 수신자/클래스명을 tags.scm만으로 못 뽑는 언어 있음(Go 메서드 receiver는 별도 노드). "qualified 실제 구현"을 ②에 끼워넣었으나 난이도 과소평가. dup_counts/stable_id(cards.rs)가 `qualified`에 의존하므로 **qualified 바뀌면 기존 카드 id가 바뀜 → 캐시 전부 무효 + M3 안정성 위반**. qualified 변경은 Stage-1 id 계약을 건드린다 — 설계의 "cards.rs 불변" 전제와 충돌. **이건 MAJOR 승격 후보**: qualified를 바꾸려면 stable_symbol_id를 `name+start_row` 기반으로 먼저 옮겨야 함.

---

## 견고한 점
- Stage-1 평면 `Vec<ReviewCard>` 불변 + layout 사이드카 분리는 옳은 결정(id 안정성·캐시 기반 보존). 정렬 교체점을 후처리로 얹는 접근 타당.
- rustls 번들 + rusqlite bundled로 3-OS 배포 의존성 제거는 정확한 트레이드오프.
- Haiku(분류/정렬)/Sonnet(요약) 티어 분리, prefill 금지, effort를 Sonnet에만 — 전부 API 사실과 일치(검증됨).

## 핵심 리스크 TOP 3
1. **타입 계약을 ① 앞 "단계 0"으로 못박지 않으면 단계 독립검증이 환상**(B2). AI 토대가 먼저 검증된다는 설계의 핵심 주장이 무너짐.
2. **`@reference.*` 언어별 비대칭**(B1)으로 관계신호가 Rust/Go/Java에서 다른 품질 → 클러스터링 결과가 언어 의존적이 되어 "그럴듯함" 일관성 붕괴.
3. **`qualified` 정규화가 Stage-1 stable id를 흔든다**(m4 승격) → "cards.rs 불변/M3 안정성/캐시" 세 전제가 동시에 깨질 수 있음. qualified 도입 전 id 키를 start_row 기반으로 이전하는 선행 작업이 누락.

**대조한 절대경로:** `/Users/jiwon/desktop/projects/loupe/src-tauri/src/engine/{model.rs,cards.rs,symbols.rs,gitdiff.rs,mod.rs}`, `/Users/jiwon/desktop/projects/loupe/src-tauri/src/lib.rs`, `/Users/jiwon/desktop/projects/loupe/src/App.jsx`, `/Users/jiwon/desktop/projects/loupe/docs/planning-v2.md`. tags.scm 캡처 검증: `~/.cargo/registry/.../tree-sitter-{rust-0.24.2,go-0.25.0,java-0.23.5}/queries/tags.scm`.
