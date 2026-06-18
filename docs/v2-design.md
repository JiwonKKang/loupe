I now have all the facts verified against the actual code. The research is accurate. Writing the design document now.

# Loupe v2 설계서 — AI 클러스터링·정렬 (Stage-2)

> 대상: 기존 Stage-1 엔진(git2 3-dot diff → tree-sitter full AST → `Vec<ReviewCard>` 평면) 위에 planning-v2.md(`/Users/jiwon/desktop/projects/loupe/docs/planning-v2.md`)의 MVP 13항목을 얹는다.
> 원칙: **Stage-1 `build_review`의 `ReviewData`/`ReviewCard`/`ReviewLine` 계약은 깨지 않는다.** Stage-2는 그 출력을 입력으로 받아 cluster 2계층을 *덧씌우는* 별도 레이어다. 정렬 교체점은 `cards.rs:64`의 `start_row` 정렬 + `mod.rs:64`의 파일 루프이며, 이 두 곳은 그대로 두고 그 *위에* AI 정렬을 후처리로 얹는다(평면 카드는 안정 id로 캐싱·검증의 기반이므로 보존).

---

## 0. 검증된 현재 상태 (코드 대조 완료)

| 사실 | 위치 | v2 영향 |
|---|---|---|
| `ReviewData { cards: Vec<ReviewCard> }` 평면 1계층 | `model.rs:11-14` | 2계층으로 *확장*(필드 추가, 기존 `cards` 유지) |
| `ReviewCard.id` 안정 키(재정렬에 불변, M3) | `model.rs:19-21` | 캐시·화이트리스트·flatten 키로 그대로 재사용 |
| `summary` B1 불변식(절대 비어있지 않음) | `model.rs:31-33` | 유지, 내용만 AI화(별 필드로 분리) |
| `changed_symbol_idxs.sort_by_key(start_row)` | `cards.rs:64` | **정렬 교체점 1** — AI `orderedSymbols`로 후처리 재정렬 |
| `for file in &diff` 파일 루프 | `mod.rs:64` | **정렬 교체점 2** — clusterOrder로 후처리 재정렬 |
| `resolve_commit` → `Oid`(내부 전용) | `gitdiff.rs:94-98` | base/head SHA **노출** 필요 → 캐시 키 |
| `Symbol { name, qualified, start_row, end_row }` (qualified==name) | `symbols.rs:17-26` | `qualified` 실제 구현 + 관계 신호 부착 |
| `TAGS_QUERY`에 `@reference.call/.type/.class` 존재(Go/Java/Rust) | `symbols.rs:56-60` | 관계 신호를 **추가 파싱 0회**로 추출 |
| AI 연동·캐시·reqwest 전무 | `Cargo.toml:20-32`, `lib.rs:39` | 신규 crate + 모듈 + 커맨드 |
| 프론트 평면 `index` + `chapter`(파일 basename) 그룹핑 | `App.jsx:20,47,53-57` | flatten 순서를 cluster 순서로, 그룹 키 `chapter`→`clusterId` |

---

## 1. Crate 추가

`/Users/jiwon/desktop/projects/loupe/src-tauri/Cargo.toml` `[dependencies]`에 추가:

```toml
# --- engine stage 2 (AI clustering/ordering) ---
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls", "stream"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
futures-util = "0.3"
async-trait = "0.1"
rusqlite = { version = "0.32", features = ["bundled"] }
sha2 = "0.10"
```

**근거 (각 줄):**
- `reqwest` + `rustls-tls` (`default-features=false`로 native-tls/OpenSSL 제거): 공식 Rust SDK가 없으므로 `api.anthropic.com/v1/messages`를 직접 호출. `rustls`는 Tauri 3-OS 배포 시 시스템 OpenSSL 의존성을 없애 빌드 단순화. `stream`은 요약(긴 출력) SSE용, `json`은 본문 직렬화용.
- `tokio` (`rt-multi-thread`): Tauri의 `async_runtime`이 tokio 기반이므로 별도 런타임 불필요. 백그라운드 선분석(`spawn`)에 멀티스레드 런타임 활용.
- `futures-util`: `Response::bytes_stream()`의 SSE 청크 누적(`StreamExt`).
- `async-trait`: `LlmProvider` trait의 `async fn`(provider 추상화).
- `rusqlite` (`bundled`): 캐시 저장소. `bundled`로 SQLite C 소스를 함께 컴파일 → 시스템 libsqlite 의존성 제거(Tauri 배포). JSON 파일 대신 SQLite인 이유는 §5 참조(행 단위 부분 무효화).
- `sha2`: `card_hash`(클러스터 카드 정규화 SHA-256) 계산.

이미 있는 `git2`(`vendored-libgit2`)·`tree-sitter`·`serde`/`serde_json`은 재사용.

---

## 2. Rust 모듈 구조

```
src-tauri/src/engine/
├── mod.rs            (변경) build_review 유지 + analyze() 신규 진입점 추가
├── gitdiff.rs        (소폭 변경) resolve_commit → pub, head/base SHA 노출
├── symbols.rs        (변경) extract_with_refs 추가 (@reference.* 수집 + qualified 구현)
├── cards.rs          (불변) Stage-1 카드/안정 id — relation·cluster의 키 소스로 재사용
├── model.rs          (확장) ReviewData 2계층 필드 추가 (기존 cards 유지)
│
├── relations.rs      (신규) 관계 신호: 이름 매칭 → RelationHints{strong,weak}
├── clustercard.rs    (신규) AI 입력 정제: ReviewCard[] + relations → ClusterCardInput
├── cluster.rs        (신규) 오케스트레이터: cache→AI→검증→fallback→JIT→layout 적용
├── cache.rs          (신규) rusqlite. (repo, base_sha, card_hash, schema_ver) 조회
├── fallback.rs       (신규) 레이어 휴리스틱 정렬 (controller→service→domain→repo→test)
├── jit.rs            (신규) JIT definition 카드 삽입 (§5)
└── ai/
    ├── mod.rs        (신규) LlmProvider trait, ModelTier, CompletionRequest/Response, LlmError
    ├── anthropic.rs  (신규) ApiKeyProvider / OAuthProvider (reqwest, SSE, structured output)
    ├── prompts.rs    (신규) AI1/AI2/AI3 시스템 프롬프트 + json_schema 상수
    ├── steps.rs      (신규) cluster_step / order_step / label_step (작은 PR 1·2 합침 분기)
    └── verify.rs     (신규) 화이트리스트 검증 (8.3)
```

의존 방향(추가): `lib → mod::analyze → cluster → {clustercard, cache, ai::{steps,verify}, fallback, jit} → {relations, symbols, cards, model}`. `ai/`는 `cluster`가 trait 객체로만 소비(테스트 시 mock provider 주입).

### 2.1 정렬 교체점의 정확한 처리

`cards.rs:64`와 `mod.rs:64`는 **건드리지 않는다.** Stage-1이 결정적 평면 `Vec<ReviewCard>`를 만든다. Stage-2 `cluster::apply_layout()`이 그 위에 **2계층 순서를 메타로 부여**한다:

```rust
// cluster.rs — 평면 카드는 입력, layout(순서)은 출력. 카드 자체를 재정렬하지 않음.
pub struct ReviewLayout {
    pub cluster_order: Vec<String>,           // ["cluster-1", "cluster-2", "__unclustered"]
    pub ordered_card_ids: Vec<String>,        // 전체 flatten 순서 (프론트 index의 source of truth)
    pub clusters: Vec<Cluster>,               // 메타 (id→title/summary/type/카드 id 목록)
    pub jit_card_ids: Vec<String>,            // JIT definition으로 삽입된 가짜 카드 id
}
```

프론트는 `cards`(diff 렌더용, 불변 계약) + `layout`(순서·그룹)을 받아 `ordered_card_ids`대로 펼친다. 카드 id가 안정적이므로 캐시 히트 시 같은 순서가 보장된다(8.1 결정성 해결).

---

## 3. 데이터 모델 (cluster 2계층)

`/Users/jiwon/desktop/projects/loupe/src-tauri/src/engine/model.rs` — 기존 타입 **유지**, 아래를 추가.

```rust
/// IPC payload. 기존 `cards`는 그대로 (diff 렌더 계약 불변). layout/clusters는 사이드카.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct ReviewData {
    pub cards: Vec<ReviewCard>,          // [불변] Stage-1 출력 그대로
    pub clusters: Vec<Cluster>,          // [신규] AI 클러스터 메타. fallback/empty면 Vec::new()
    pub cluster_order: Vec<String>,      // [신규] 클러스터 간 순서 (마지막에 "__unclustered")
    pub ordered_card_ids: Vec<String>,   // [신규] 전체 flatten 순서 (프론트 index 기준)
    pub unclustered: Vec<String>,        // [신규] §3.1 버킷에 들어간 card id 목록
    pub jit_defs: Vec<JitDefinition>,    // [신규] §5 정의 카드 (별 카드로 ordered에 삽입됨)
    pub head_sha: String,                // [신규] 캐시 키 / "같은 head=같은 순서" 표식
    pub base_sha: String,                // [신규] 3-dot은 base에 의존 → 키
    pub analysis: AnalysisState,         // [신규] done | fallback | partial (스트리밍용)
    pub merge_suggestions: Vec<Suggestion>,   // §6.3
    pub split_suggestions: Vec<Suggestion>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Cluster {
    pub id: String,                  // "cluster-1" (휘발성 라벨, 캐시 해시에서 제외)
    pub title: String,               // AI (§6.2) — B1처럼 절대 비어있지 않음 보장(fallback="Changes")
    pub summary: String,             // AI 1~3문장
    pub kind: ClusterKind,           // 최종 분류 (AI)
    pub type_hint: ClusterKind,      // 알고리즘 힌트(AI 입력) — 디버그/표시용 보존
    pub ordered_card_ids: Vec<String>, // 클러스터 내부 순서 (AI orderedSymbols → card id)
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ClusterKind { Flow, Contract, DomainConcept, SharedFoundation, Infra }

/// §5 JIT — 일반 카드 사이에 끼는 "정의 개요" 가짜 카드. 프론트는 kind로 분기.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct JitDefinition {
    pub id: String,                  // "jit-<symbolid>" (안정)
    pub symbol: String,              // "OrderDraft"
    pub path: String,
    pub overview: DefinitionOverview,
    pub injected_before: String,     // 이 card id 앞에 삽입됨
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct DefinitionOverview {
    pub role: Option<String>,        // AI 한 줄 (없으면 None)
    pub fields: Vec<String>,         // tree-sitter로 뽑은 필드 시그니처
    pub constructor: Option<String>,
    pub public_methods: Vec<String>,
    pub changed_methods: Vec<String>, // 이번 PR에서 바뀐 메서드(= changed_symbol과 교집합)
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Suggestion {                 // §6.3 merge/split — 표시만, 자동적용 X
    pub kind: &'static str,             // "merge" | "split"
    pub cluster_ids: Vec<String>,
    pub reason: String,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AnalysisState { Done, Fallback, Partial }
```

`ReviewCard`에 추가(기존 필드 유지, 모두 옵션/기본값이라 Stage-1 테스트 영향 없음):

```rust
pub struct ReviewCard {
    // ... 기존 id/chapter/symbol/path/status/summary/lines 유지 ...
    pub cluster_id: Option<String>,   // [신규] 소속 클러스터. None=Stage-1만 돌았을 때
    pub kind: SymbolKind,             // [신규] §2.2 분류 (default: Function)
    pub qualified: String,            // [신규] "OrderService.calculatePrice" (실제 정규화)
    pub change_type: ChangeType,      // [신규] Added | Modified | Deleted
    pub ai_summary: Option<String>,   // [신규] AI 의미 요약 (통계 summary와 분리 — B1은 summary가 지킴)
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SymbolKind { Function, Method, Class, Type, Interface, Enum, Dto, Test, Migration, Config, File }

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChangeType { Added, Modified, Deleted }
```

**프론트 JSON 계약(요지):** `load_review`/`analyze_review`는 `{ cards, clusters, clusterOrder, orderedCardIds, unclustered, jitDefs, headSha, baseSha, analysis, mergeSuggestions, splitSuggestions }`. serde가 camelCase로 직렬화하도록 `#[serde(rename_all = "camelCase")]`를 `ReviewData`에 부여(기존 `n`/`t`/`c`/`cards`는 영향 없음 — 새 필드만 camel). 프론트는 `cards`로 diff를, `orderedCardIds`로 순서를, `clusters`로 spine 그룹을 만든다.

---

## 4. AI 입출력 계약

### 4.1 provider 추상화 — `ai/mod.rs`

```rust
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    /// 분류·정렬 등 구조화 1회 호출 (비스트리밍).
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError>;
    /// 요약 등 긴 출력 — SSE 누적 텍스트.
    async fn complete_streaming(&self, req: CompletionRequest) -> Result<String, LlmError>;
    fn model_for(&self, tier: ModelTier) -> &'static str;
}

pub struct CompletionRequest {
    pub system: String,
    pub user: String,
    pub max_tokens: u32,
    pub json_schema: Option<serde_json::Value>, // structured output 강제
    pub tier: ModelTier,
}
pub struct CompletionResponse {
    pub json: serde_json::Value,   // structured output (from_str로 파싱한 결과)
    pub stop_reason: String,       // refusal/max_tokens 확인 필수
}

#[derive(Clone, Copy)]
pub enum ModelTier { Fast, Quality }  // Fast=정렬/분류, Quality=요약

pub enum LlmError { Http(String), Auth, Refusal, Parse(String), Overloaded, Timeout }
```

**모델 티어 (2026-06 확정 ID, 날짜 suffix 금지):**

```rust
impl ApiKeyProvider {
    fn model_for(&self, tier: ModelTier) -> &'static str {
        match tier {
            ModelTier::Fast    => "claude-haiku-4-5",    // 클러스터링/정렬 — 200K/64K, $1/$5
            ModelTier::Quality => "claude-sonnet-4-6",   // title/summary — 1M/64K, $3/$15
        }
    }
}
```

**인증 (`ai/anthropic.rs`) — 핵심 리스크 반영:**

```rust
// 기본/권장: BYO API 키 (x-api-key). 정식 지원.
struct ApiKeyProvider { client: reqwest::Client, api_key: String }   // sk-ant-api...
// 비공식/ToS 주의: OAuth setup-token. feature flag 뒤, UI 경고, 기본 비활성.
struct OAuthProvider  { client: reqwest::Client, token: String }     // sk-ant-oat01...
```

- 인증 차이는 **헤더 부착 함수 하나로만** 격리. 본문 빌더/SSE 파서/스키마 로직은 공유.
  - ApiKey: `x-api-key: <key>` + `anthropic-version: 2023-06-01`
  - OAuth: `authorization: Bearer <token>` + `anthropic-beta: oauth-2025-04-20` + `user-agent: claude-code/<ver>` — **공개 Messages API가 거부하며 ToS 위반.** 코드 자리만 두고 `#[cfg(feature = "oauth-unofficial")]` + 기본 off + onboarding UI에 "비공식/제재 리스크" 경고. **출시 기본 경로는 BYO `x-api-key`.**
- onboarding setup-token으로 공개 API 직접 호출은 동작하지 않으므로, **onboarding에서 받는 인증을 "API 키 입력"으로 안내**하고, setup-token은 향후 Anthropic #37205 정식 지원 시 활성화.
- 주의: `x-api-key`와 `authorization` 동시 전송 시 401 — 둘 중 하나만. **assistant prefill 금지**(Haiku 4.5/Sonnet 4.6에서 400). JSON 시작 강제는 structured output으로. **effort 파라미터는 Haiku에 넣지 말 것**(미지원 400) — Sonnet 요약 경로에만 `medium`.

### 4.2 구조화 출력 강제

요청 본문에 `output_config.format = { type: "json_schema", schema }`. 스키마 제약: `additionalProperties:false` 필수, `enum`/`anyOf` 가능, **재귀·`minimum`/`maxLength` 등 미지원**(처음부터 넣지 않음). 응답의 tool/json은 **반드시 `serde_json::from_str`로 파싱**(raw 매칭 금지). `stop_reason`이 refusal이면 content 비어있을 수 있으니 검증 전 확인.

### 4.3 AI 입력 — cluster card (`clustercard.rs`)

planning §6.1과 동형. Stage-1 카드 + relations에서 생성:

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterCardInput {
    pub cluster_id: String,                  // 알고리즘 1차 묶음 라벨(파일/path 기반 seed)
    pub algorithmic_type_hint: ClusterKind,
    pub entrypoint_candidates: Vec<String>,  // 경로/어노테이션 휴리스틱 (라우트 등)
    pub changed_symbols: Vec<ChangedSymbolIn>, // {name, kind, changeType, summary}
    pub relation_hints: RelationHints,       // {strong:[[a,b]], weak:[[a,b]]} — card id 쌍
    pub contracts_changed: Vec<String>,      // DTO 필드/마이그레이션 등
    pub related_tests: Vec<String>,
}
#[derive(Serialize)]
pub struct ChangedSymbolIn { pub card_id: String, pub name: String,
    pub kind: SymbolKind, pub change_type: ChangeType, pub summary: String }
```

`name`은 `card_id`로 역참조 가능해야 검증·flatten이 된다(아래 4.5). 입력은 raw diff가 아니라 이 정제 카드만.

### 4.4 3단계 + 작은 PR 분기 (`ai/steps.rs`)

```rust
pub async fn run_pipeline(
    p: &dyn LlmProvider, cards: &[ClusterCardInput], whitelist: &HashSet<String>,
) -> Result<AiResult, LlmError> {
    let small = cards.iter().map(|c| c.changed_symbols.len()).sum::<usize>() <= SMALL_PR_SYMBOLS; // 예: 12
    let clustered_ordered = if small {
        cluster_and_order_combined(p, cards).await?   // [AI 1+2] 1호출 (Fast)
    } else {
        let clusters = cluster_step(p, cards).await?; // [AI 1] (Fast)
        order_step(p, &clusters).await?               // [AI 2] (Fast)
    };
    verify::whitelist(&clustered_ordered, whitelist)?;     // §8.3 — 실패 시 1회 재요청 후 Err
    let labeled = label_step(p, &clustered_ordered).await?; // [AI 3] title/summary 1호출 배치 (Quality, streaming)
    verify::whitelist_labels(&labeled, whitelist)?;
    Ok(labeled)
}
```

- AI1 출력 스키마: `{ clusters: [{clusterId, memberCardIds[], kind}], unclustered: [cardId] }`.
- AI2 출력: `{ clusterOrder: [clusterId], orderedByCluster: {clusterId: [cardId]} }`.
- AI1+2 합침 출력: 위 둘을 한 객체로.
- AI3 출력: `{ clusters: [{clusterId, title, summary}], mergeSuggestions[], splitSuggestions[] }` — §6.2.
- **모든 심볼 식별자는 `cardId`로 주고받는다**(planning은 name으로 예시했으나, 안정 id가 검증·flatten 키이므로 id로 통일; name은 표시용으로만 동봉).

### 4.5 화이트리스트 검증 (`ai/verify.rs`, §8.3)

```rust
/// AI 출력의 모든 cardId가 입력 화이트리스트(= Stage-1 카드 id 집합)에 있어야 한다.
/// 누락(입력에 있는데 출력에 없음)·환각(출력에 있는데 입력에 없음) 둘 다 검출.
pub fn whitelist(out: &ClusterOrderOut, allow: &HashSet<String>) -> Result<(), LlmError> {
    for id in out.all_card_ids() {
        if !allow.contains(id) { return Err(LlmError::Parse(format!("hallucinated id {id}"))); }
    }
    // 누락 id는 자동으로 __unclustered로 흡수(드롭 금지 — "모든 변경이 보인다" §3.1).
    Ok(())
}
```

화이트리스트 소스 = `cards.rs:54-64`의 changed-symbol 집합에서 만든 card id(= `ReviewCard.id`). 검증 실패 → 1회 재요청 → 또 실패 시 fallback(§9). title/summary가 입력에 없는 심볼을 언급해도 텍스트 검증은 어렵지만, orderedSymbols id 집합 검증이 1차 안전장치.

### 4.6 AI 제약 프롬프트 (`ai/prompts.rs`, §10)

시스템 프롬프트에 고정 규칙: 제공 카드에 없는 symbol 생성 금지 / 미제공 side effect 추정 금지 / 없는 테스트 지어내기 금지 / 클러스터 이름 `[대상]+[변경동작]` 짧게 / 요약 1~3문장 / 불확실하면 단정 금지 / merge·split은 명확할 때만. (검증이 진짜 안전장치, 프롬프트는 보조.)

---

## 5. 관계 신호 (`relations.rs`, §4.4)

call graph가 아니라 **이름 매칭 힌트**. `symbols.rs`의 `extract_with_refs`가 같은 `TAGS_QUERY` 패스에서 `@reference.call/.type/.class` 식별자를 심볼 본문 row에 귀속(추가 파싱 0회):

```rust
// symbols.rs (추가) — 기존 extract는 유지, refs까지 한 패스로.
pub fn extract_with_refs(lang: Lang, source: &str)
    -> Result<Option<(Vec<Symbol>, Vec<SymbolRefs>)>, EngineError>;

pub struct SymbolRefs { pub sym_idx: usize, pub calls: Vec<RefHit>, pub type_refs: Vec<RefHit> }
pub struct RefHit { pub ident: String, pub row: usize, pub in_header: bool }
```

```rust
// relations.rs — 순수 함수, 그래프 라이브러리 없음.
pub fn compute_relation_hints(input: &RelationInput) -> Vec<RelationHint>;
// O(refs × changed) 이름 매칭. self 제외, hub(이름셋 + fan-in≥4) 강등/드롭,
// 심볼당 strong top-K=8 상한, import-only=weak, test→impl/class-helper=strong.
// 결과는 card id 쌍 → RelationHints{strong, weak}로 clustercard에 실림.
```

판정 2값(strong/weak)만 emit. medium(같은 path/package)은 쌍으로 안 만들고 카드의 `path`로 AI가 판단(N² 폭발 방지). hub 이름셋: `{Logger, log, DateUtils, StringUtils, JsonUtils, ErrorCode, BaseResponse, CommonException, Objects, Optional, Arrays}` + fan-in 임계(PR 지역 통계). parser ERROR/미지원 언어 → refs 0 → 관계 0(안전 강등). **결정적**(head SHA 고정 + 순수 함수)이라 캐시 키와 정합.

`mod.rs::build_review`에서 `symbols::extract` 호출부(`mod.rs:75`)를 `extract_with_refs`로 교체하되, **Stage-1 `ReviewData`는 불변** — refs는 Stage-2 `analyze()`에서만 소비.

---

## 6. JIT definition (`jit.rs`, planning §5)

AI 정렬 결과(`ordered_card_ids`) 위에 **후처리로** 정의 카드를 삽입. 옵션 (a) 채택: flatten list에 `JitDefinition`을 별 항목으로 끼움(평면 index 모델과 자연 정합).

```rust
pub fn inject(layout: &mut ReviewLayout, cards: &[ReviewCard], symbols_by_path: &SymbolIndex);
// 규칙: (1) signature에 등장하는 새 타입 → 그 함수 카드 앞. (2) 함수 본문에서 처음 생성되는
// 새 클래스 → 생성 카드 앞. (3) 클래스 메서드 진입 전 class overview.
// overview(fields/constructor/public_methods)는 tree-sitter로 추출(추가 AI 호출 없음).
// role 한 줄만 AI3 배치에 슬롯으로 끼워 받음(선택).
```

프론트: `card.kind === 'definition'`(또는 `jitDefs`에 id 존재) → diff 대신 개요 패널 렌더. 같은 head 캐시에서 동일하게 삽입되어 결정성 유지.

---

## 7. 캐싱 (`cache.rs`, §8.2)

**SQLite (rusqlite, `app_data_dir/loupe/cache.db`).** JSON 파일이 아닌 이유: 부분 무효화(바뀐 클러스터만 재호출)가 행 단위 UPSERT로 공짜, 백그라운드 선분석과 메인 로드의 동시 쓰기 안전(WAL), 복합키 인덱스 조회. JSON은 전체 read-merge-write라 write amplification + 부분쓰기 손상 위험.

```sql
CREATE TABLE IF NOT EXISTS cluster_result (
  repo_path  TEXT NOT NULL,   -- canonicalize된 절대경로
  base_sha   TEXT NOT NULL,   -- resolve_commit가 노출한 SHA (3-dot은 base 의존)
  card_hash  TEXT NOT NULL,   -- 클러스터 카드 정규화 SHA-256 (§아래)
  schema_ver INTEGER NOT NULL,-- 프롬프트/스키마 버전 (올리면 자동 미스)
  result_json TEXT NOT NULL,  -- AI 결과 {title, summary, kind, orderedCardIds}
  created_at INTEGER NOT NULL,
  PRIMARY KEY (repo_path, base_sha, card_hash, schema_ver)
);
CREATE TABLE IF NOT EXISTS review_layout (   -- head 단위 클러스터 간 순서 + unclustered
  repo_path TEXT NOT NULL, base_sha TEXT NOT NULL, head_sha TEXT NOT NULL,
  schema_ver INTEGER NOT NULL, layout_json TEXT NOT NULL,
  PRIMARY KEY (repo_path, base_sha, head_sha, schema_ver)
);
```

- **조회 키 = `(repo_path, base_sha, card_hash, schema_ver)`** — head를 키에서 빼서 부분 무효화가 공짜. push로 head 바뀌어도 내용 같은 클러스터(`card_hash` 동일)는 hit → AI 재호출 회피. 바뀐 클러스터만 miss.
- **layout(클러스터 간 순서)은 head 단위로 캐싱** → "같은 head=같은 순서" 결정성(§8.1) 보장.
- `card_hash` = 정규화 직렬화 후 SHA-256. **포함**: `changedSymbols`(name+kind+changeType+각 심볼 diff 본문 해시), `relationHints`, `contractsChanged`, `algorithmicTypeHint`, `entrypointCandidates`. **제외**: `clusterId`(휘발성), 순서 의존 배열은 정렬 후 해시. `schema_ver`를 해시 prefix에 섞어 프롬프트 변경 시 자동 무효화.
- `gitdiff.rs`: `resolve_commit`을 `pub`으로, `diff_three_dot`가 base/head SHA를 함께 반환하도록 시그니처 확장(또는 `pub fn resolve_shas(repo, base, target) -> (String, String)` 추가).

---

## 8. 프론트 (클러스터 2계층 + 로딩)

### 8.1 App.jsx
- 상태 추가: `clusters`, `clusterOrder`, `orderedCardIds`, `unclustered`, `jitDefs`, `analysisState`(`'idle'|'clustering'|'ordering'|'summarizing'|'done'|'fallback'`).
- `load_review` 응답(`App.jsx:35`) 파싱 확장: `setCards(data.cards)` 유지 + 신규 필드 set.
- `const list`(`App.jsx:47`)를 `flattenByOrder(cards, jitDefs, orderedCardIds)`로 — index가 cluster 순서·JIT 삽입을 자동으로 탄다. `advance/next/prev`(`App.jsx:61-67`)는 불변.
- `spineItems`(`App.jsx:53-57`): `chapter: c.chapter` → `clusterId/clusterTitle/clusterKind`를 직접 전달. Unclustered 카드는 `clusterId:'__unclustered'`, 제목 "Unclustered changes".

### 8.2 ProgressSpine.jsx (+ 점 잘림 폴리시)
- 그룹핑(`:19-24`): `it.chapter` → `it.clusterId` consecutive 묶기. 그룹 헤더(`:54-60`)에 cluster title + 타입 점(flow/contract/domain/shared/infra 색) + 확장 시 summary. Unclustered는 맨 끝 흐릿한 톤.
- **점 잘림 버그 수정**: 축소 레일 `--rail-width:14px`(`spacing.css:22`) + 컨테이너 `overflow:hidden`(`:38`) + active 점 `width:7` + `boxShadow:'0 0 0 4px'`(`:67-69`) = 15px가 14px 레일에서 클립. 수정 ① `--rail-width` 16~18px, ③ 컨테이너 `overflow-x:visible; overflow-y:auto`. 클러스터 경계만 `marginTop` 키워 시각 구분.

### 8.3 ReviewScreen.jsx
- 상단 진행바(`:177`) `card.chapter` → `clusterTitle`. 카드 헤더(`:218-234`)에 클러스터 띠(type 뱃지 + title) + "n / m in this cluster"(`:173-175` 옆).
- diff 렌더 분기: `card.kind === 'definition'` → JIT 개요 패널(fields/constructor/public methods/이번 PR 변경 methods).

### 8.4 LoupeLoader.jsx (신규 — 현재 없음)
- `src/components/LoupeLoader.jsx` 생성. `App.jsx:145-152`의 `LoadingScreen` 교체. 2단계: ① `cards===null`(diff 읽는 중, 짧음) → ② `analysisState∈{clustering,ordering,summarizing}`(클러스터 채워지는 대로 spine 점진 노출, §8.4 스트리밍).

---

## 9. IPC

```rust
// lib.rs — 기존 load_review 유지(Stage-1 즉시 반환, 캐시 히트면 cluster 포함).
#[tauri::command]
async fn analyze_review(app: AppHandle, repo_path: String, base: String, target: String)
    -> Result<engine::ReviewData, String>;   // 캐시 히트=즉시, 미스=AI 파이프라인 동기 1회

// 백그라운드 선분석 (planning §8.4 / 로컬 번역): startReview 시점에 띄움. head SHA 멱등.
#[tauri::command]
async fn prewarm_analysis(app: AppHandle, repo_path: String, base: String, target: String)
    -> Result<(), String>;  // 결과를 SQLite에 쌓고 단계별 emit("analysis-progress", ...)
```

- `invoke_handler`(`lib.rs:39`)에 `analyze_review`, `prewarm_analysis` 등록.
- 흐름: `startReview()`(`App.jsx:41`) → `load_review`(즉시 diff+평면 카드) + `prewarm_analysis`(백그라운드). 프론트는 `analysis-progress` 이벤트로 클러스터를 점진 수신. 캐시 히트면 `load_review` 응답에 이미 cluster 포함 → 체감 0초.
- `async fn` 커맨드는 Tauri `async_runtime`(tokio) 위에서 reqwest를 그대로 await.

---

## 10. 단계별 구현 태스크 (독립 검증, dearday `main...feat/https-via-caddy`)

각 단계는 이전 단계 없이도 `cargo test` 또는 dearday 실측으로 검증 가능.

| # | 태스크 | 산출물 | dearday 검증 |
|---|---|---|---|
| ① | **AI 토대** — `ai/mod.rs` trait + `anthropic.rs` ApiKeyProvider + structured output + SSE | reqwest 빌드, BYO 키로 "ping" classify 1회 성공 | 더미 cluster card 1개를 Haiku에 보내 `{clusterId,kind}` JSON 수신 확인 |
| ② | **관계 신호** — `symbols.rs::extract_with_refs` + `relations.rs` | `compute_relation_hints` 순수 함수 + 단위 테스트 | dearday diff에서 strong/weak 쌍 출력, hub(Logger 등) 강등 확인 |
| ③ | **cluster card 정제** — `clustercard.rs` | Stage-1 카드 + relations → `ClusterCardInput[]` (직렬화) | dearday `feat/https-via-caddy`의 변경 심볼이 카드로 압축됨 (raw diff 미포함 확인) |
| ④ | **AI 클러스터링** — `ai/steps.rs::cluster_step` | AI1 호출 + 스키마 출력 | dearday 변경이 의미 클러스터로 묶임, `memberCardIds` 모두 화이트리스트 내 |
| ⑤ | **정렬 + 검증** — `order_step` + `verify.rs` + 작은PR 1·2 합침 | `ReviewLayout` + 화이트리스트 통과/실패 분기 | dearday(작은 PR이면 1호출) 순서 결정, 환각 id 주입 시 reject 단위테스트 |
| ⑥ | **title/summary 배치** — `label_step` (Quality, streaming) | `Cluster.title/summary` 채움, B1·1~3문장 | dearday 클러스터에 한국어 title/summary, merge/split 제안 파싱 |
| ⑦ | **SHA 캐싱** — `cache.rs` + `gitdiff` SHA 노출 | rusqlite, 부분 무효화 | dearday 2회 분석 → 2회차 AI 0호출(전부 hit), head 바꾼 척 schema_ver 올리면 미스 |
| ⑧ | **프론트 cluster UI + 로딩** — App/Spine/Review + LoupeLoader + 점잘림 | 2계층 spine, flatten index, 로딩 | dearday 리뷰 화면에서 클러스터 그룹·title·Unclustered 노출, 점 안 잘림 |
| ⑨ | **JIT + Unclustered** — `jit.rs` + 버킷 | 정의 카드 삽입, Unclustered 항상 노출 | dearday import-only/모듈레벨 변경이 Unclustered에, 새 타입 정의 카드가 사용처 앞에 |
| ⑩ | **레이어 fallback** — `fallback.rs` | AI off/실패 시 path 휴리스틱 정렬 | API 키 제거 후에도 dearday가 controller→service→domain→repo→test 순으로 동작 |

오케스트레이터 `cluster.rs`는 ④~⑦, ⑨~⑩을 묶는 마지막 통합 — `cache.get → miss면 run_pipeline → verify → 성공 시 jit::inject + cache.put / 실패 시 fallback::sort`.

---

## 11. 리스크 + 의도적 미루기

**리스크:**
1. **OAuth setup-token이 공개 API에서 거부 + ToS 위반.** → 출시 기본은 BYO `x-api-key`. OAuth는 feature flag + 경고 + 기본 off. onboarding을 "API 키 입력"으로 안내.
2. **이름 매칭 false positive**(relations bare-name). → 힌트일 뿐 AI가 path/kind로 거름. 정렬 결정에 쓰지 않음. hub fan-in 강등으로 완충.
3. **AI 비결정성.** → head SHA 캐싱(layout) + 화이트리스트 검증으로 흡수.
4. **큰 PR 입력 토큰 폭발.** → cluster card 정제(raw diff 미포함) + strong top-K=8 + medium 쌍 미생성. 한도 초과 시 fallback.
5. **Haiku effort/prefill 400.** → effort는 Sonnet에만, prefill 전면 금지, structured output으로 대체.
6. **parser ERROR / Kotlin 등 미지원.** → Stage-1대로 None → 심볼·관계 0 → file-level + Unclustered. 안전 강등.
7. **rusqlite bundled 빌드 시간.** → 수용(시스템 SQLite 의존성 제거 이득이 큼).

**의도적으로 미루는 것 (planning §12 제외 항목):**
- 호출 그래프 DFS 정렬(2차 fallback 후속) — `fallback.rs`는 레이어 휴리스틱만.
- AI 위험도 판단 / 리뷰 포커스 / 테스트 부족 분석.
- 리뷰어별 맞춤 정렬 / pairwise ranking.
- community detection 클러스터링 / 복잡한 자동 우선순위 재정렬.
- merge/split **자동 적용**(표시만 — §6.3).
- OllamaProvider(trait 자리만, 구현 후속).

---

## 핵심 한 줄

Stage-1의 평면 `Vec<ReviewCard>`(안정 id·결정적)는 **불변으로 두고**, 그 위에 `relations`(이름 매칭 힌트) → `clustercard`(AI 입력 정제) → `ai/`(BYO `x-api-key`, Haiku 클러스터링·정렬 / Sonnet 요약, structured output + 화이트리스트 검증) → `cache`(SQLite, `(repo,base_sha,card_hash,schema_ver)` 부분 무효화) → `jit`/`fallback`을 `cluster.rs` 오케스트레이터로 엮어 `ReviewData`에 `clusters/clusterOrder/orderedCardIds/unclustered/jitDefs`를 **덧씌우고**, 프론트는 평면 `index`를 유지한 채 `orderedCardIds`로 flatten하고 spine 그룹 키를 `chapter`→`clusterId`로 바꾼다.

**참조 파일(절대경로):** 변경 `/Users/jiwon/desktop/projects/loupe/src-tauri/src/engine/{mod.rs,model.rs,symbols.rs,gitdiff.rs}`, `/Users/jiwon/desktop/projects/loupe/src-tauri/Cargo.toml`, `/Users/jiwon/desktop/projects/loupe/src-tauri/src/lib.rs`, `/Users/jiwon/desktop/projects/loupe/src/App.jsx`, `/Users/jiwon/desktop/projects/loupe/src/components/ProgressSpine.jsx`, `/Users/jiwon/desktop/projects/loupe/src/screens/ReviewScreen.jsx`, `/Users/jiwon/desktop/projects/loupe/src/styles/tokens/spacing.css`. 신규 `/Users/jiwon/desktop/projects/loupe/src-tauri/src/engine/{relations.rs,clustercard.rs,cluster.rs,cache.rs,fallback.rs,jit.rs}` + `engine/ai/{mod.rs,anthropic.rs,prompts.rs,steps.rs,verify.rs}`, `/Users/jiwon/desktop/projects/loupe/src/components/LoupeLoader.jsx`. 불변(키 소스) `/Users/jiwon/desktop/projects/loupe/src-tauri/src/engine/cards.rs`.
