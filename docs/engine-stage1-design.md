I have everything I need. Writing the design document now.

---

# Loupe Rust 엔진 1단계 구현 설계서

## 0. 1단계 목표와 범위 (재확인)

**목표:** `src/data/fixtures.js`의 정적 `cards`를, 사용자가 고른 로컬 git repo의 `base...target` diff에서 실제로 생성한 심볼 카드 배열로 교체하여 화면에 띄운다.

**범위 IN:** git diff 읽기(git2) → tree-sitter 심볼 경계 추출 → "변경 라인 ∩ 심볼" 카드 빌더 → `load_review` IPC command → `App.jsx`가 fixtures 대신 IPC 결과 사용(로딩/에러 상태 포함). 색칠은 프론트 책임(계약 [1] §4)이므로 Rust는 색 정보를 보내지 않는다.

**범위 OUT (의도적 연기, §7 참조):** 휴리스틱 호출그래프 정렬, AI 스레드 생성, Kotlin grammar, 카드 상태(verdict/thread) 영속화, 작업트리(unstaged) diff. 단 인터페이스는 확장 가능하게 둔다.

핵심 계약 한 줄: **Rust가 IPC로 내보내야 하는 최소 단위는 `cards: ReviewCard[]` 하나뿐**이며, `verdicts`/`threads`/`spineItems`/`unresolved`는 전부 프론트가 `cards`로부터 파생한다(계약 [1] §2).

---

## 1. Crate 선택 확정 + 근거

`src-tauri/Cargo.toml`에 추가:

```toml
[dependencies]
# --- 이미 존재 (스캐폴드) ---
tauri = { version = "2", features = [] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# --- 1단계 신규 ---
git2 = { version = "0.20", default-features = false, features = ["vendored-libgit2"] }
tree-sitter = "0.25"
tree-sitter-highlight = "0.25"          # 1단계는 미사용이나 2-step 색칠 도입 대비 가능. 미사용이면 생략.
tree-sitter-go = "0.25"
tree-sitter-java = "0.23"
tree-sitter-rust = "0.24"
# Kotlin은 1단계 제외 (§7). 추가 시: tree-sitter-kotlin-ng = "1.1"

[dev-dependencies]
tempfile = "3"   # 통합 테스트에서 임시 repo 생성용
```

| 영역 | 선택 | 근거 (조사 [2][3] 종합) |
|---|---|---|
| git 읽기 | **git2 0.20** (`vendored-libgit2`) | per-line `origin_value()`/`old_lineno()`/`new_lineno()`를 그대로 제공 → 심볼 앵커링에 필요한 라인번호를 unified-diff 파싱 없이 획득. `merge_base()`+`diff_tree_to_tree()`로 3-dot, `find_similar()`로 rename. `vendored-libgit2`로 libgit2를 정적 링크 → 사용자 머신의 git 바이너리 버전 비의존. macOS는 Xcode C 툴체인 보유. gix는 per-line API가 imara-diff까지 내려가야 하고 pre-1.0 → v2 재검토. CLI subprocess는 텍스트 파싱 off-by-one 리스크로 기각(테스트 오라클로만 보존). |
| 코어 파서 | **tree-sitter 0.25** | 4개 grammar의 ABI를 하나의 코어로 로드. `set_language()` 결과를 반드시 `Result`로 처리. CI에서 `cargo tree -d`로 tree-sitter 중복 버전 점검. |
| 심볼 경계 | grammar 번들 **`TAGS_QUERY`** (Go/Java/Rust) | `@definition.function`/`.method`/`.class` 캡처 노드의 `start_position().row`/`end_position().row`가 곧 카드 경계, `@name`이 표시명. 언어별 노드 kind 암기 불필요·유지보수 용이. (1단계 언어 3종은 전부 `TAGS_QUERY` 상수를 번들.) |
| 색칠 | **하지 않음 (Rust)** | 계약 [1] §4: 구문 색칠은 100% 프론트(`highlightGo`). Rust는 raw 라인 텍스트(`c`)만 전송. `tree-sitter-highlight`는 1단계에서 불필요 — 표에는 미래 옵션으로만 표기. |

> 핵심 결정: **1단계 Kotlin 제외.** `tree-sitter-kotlin-ng` Rust 크레이트는 `TAGS_QUERY`/`HIGHLIGHTS_QUERY` 상수를 노출하지 않아 `.scm`을 직접 번들+버전핀+통합테스트가 필요하고, 파서 정확도(~61%)·tags 미성숙 리스크가 큼(조사 [3] §6). 1단계는 Go/Java/Rust 3종으로 좁히고, 미지원 확장자는 "파일 레벨 카드"로 폴백(§5).

---

## 2. Rust 모듈 구조

```
src-tauri/src/
├── main.rs                  # 기존: loupe_lib::run() 호출만 (수정 없음)
├── lib.rs                   # run() + invoke_handler 등록. 모듈 선언. (얇게 유지)
└── engine/
    ├── mod.rs               # pub use 재노출. load_review의 순수 함수 build_review() 구현
    ├── model.rs             # 직렬화 타입: ReviewData/ReviewCard/ReviewLine (+ serde)
    ├── gitdiff.rs           # git2 래핑: 3-dot diff → FileDiff/DiffLine, 파일 전체 blob 조회
    ├── symbols.rs           # tree-sitter: lang 디스패치, TAGS_QUERY로 Symbol 경계 추출
    └── cards.rs             # FileDiff + Symbol[] → ReviewCard[] (핵심 매핑 알고리즘)
```

**책임 경계 (단방향 의존: lib → engine::mod → {gitdiff, symbols, cards} → model):**

- `gitdiff.rs` — git2만 안다. tree-sitter/serde 모름. 출력은 순수 Rust 중간타입.
- `symbols.rs` — tree-sitter만 안다. git 모름. 입력은 `&[u8]` 소스+`Lang`, 출력은 `Vec<Symbol>`.
- `cards.rs` — 위 둘의 출력을 조합해 IPC 타입(`model.rs`)을 만든다. git2/tree-sitter 직접 호출 없음 → 알고리즘 단위 테스트가 쉬움.
- `model.rs` — serde 직렬화 타입만. 프론트 계약([1])의 단일 진실원천(single source of truth).
- `mod.rs::build_review(repo, base, target) -> Result<ReviewData, EngineError>` — 순수 함수(Tauri 비의존)로 두어 `cargo test`에서 직접 호출 가능. `lib.rs`의 `#[tauri::command] load_review`는 이걸 호출하고 에러를 `String`으로 매핑만 한다.

```rust
// engine/mod.rs
mod model;  mod gitdiff;  mod symbols;  mod cards;
pub use model::{ReviewData, ReviewCard, ReviewLine};

#[derive(Debug)]
pub enum EngineError { Git(git2::Error), Parse(String), Io(std::io::Error) }
impl std::fmt::Display for EngineError { /* 사용자 친화 메시지 */ }
impl From<git2::Error> for EngineError { /* ... */ }

pub fn build_review(repo_path: &str, base: &str, target: &str)
    -> Result<ReviewData, EngineError>
{
    let diff = gitdiff::diff_three_dot(repo_path, base, target)?;   // Vec<FileDiff>
    let mut cards = Vec::new();
    for file in &diff {
        let lang = symbols::Lang::from_path(&file.new_path);
        let symbols = match lang {
            Some(l) => symbols::extract(l, &file.new_source)?,      // Vec<Symbol>
            None => Vec::new(),                                     // 미지원 → 파일레벨 폴백
        };
        cards::build_file_cards(file, &symbols, &mut cards);
    }
    Ok(ReviewData { cards })
}
```

---

## 3. 데이터 모델: Rust struct ↔ 프론트 JSON

계약 [1] §1과 정확히 일치. 프론트는 `c`/`t`/`n` (짧은 키)을 기대하므로 `#[serde(rename)]`이 **필수**.

```rust
// engine/model.rs
use serde::Serialize;

#[derive(Serialize, Debug, Clone)]
pub struct ReviewData {
    pub cards: Vec<ReviewCard>,        // 순서 = 1단계에서는 (path, 첫 변경라인) 안정정렬. 정렬 휴리스틱은 2단계.
}

#[derive(Serialize, Debug, Clone)]
pub struct ReviewCard {
    pub id: String,        // 전역 유일. 예: "internal/auth/session.go::Session.Validate"
    pub chapter: String,   // 그룹 라벨. 1단계 = 파일 basename (예: "session.go")
    pub symbol: String,    // 표시 제목. 예: "Session.Validate" (미지원/파일레벨이면 basename)
    pub path: String,      // repo-relative. 예: "internal/auth/session.go"
    pub status: String,    // 항상 "pending" (seed only — 프론트가 verdict로 재계산하므로 무의미하나 계약상 필수)
    pub summary: String,   // 대문자로 시작하는 한 문장. 1단계 = 기계 생성 문장(아래)
    pub lines: Vec<ReviewLine>,
}

#[derive(Serialize, Debug, Clone)]
pub struct ReviewLine {
    pub n: u32,            // 게터 표시용 라인번호. add/ctx = new_lineno, del = old_lineno
    pub t: &'static str,   // "add" | "del" | "ctx"  (계약 [1]: 정확히 이 3개)
    pub c: String,         // raw 코드(선행 \t 포함, 줄바꿈/diff 마커 없음)
}
```

**필드별 계약 일치 검증 (계약 [1] §1 대조):**

| 계약 필드 | Rust 산출 규칙 | 비고 |
|---|---|---|
| `id` | `format!("{}::{}", path, symbol)` (파일레벨은 `format!("{}::__file", path)`) | 전역 유일 보장. 같은 파일 내 동명 메서드는 1단계에서 발생 가능 → `::{idx}` suffix로 충돌 회피(§5). |
| `chapter` | 파일 basename | ProgressSpine이 **연속** 동일 chapter를 섹션으로 묶음 → 같은 파일 카드가 연속 출력되도록 정렬 유지. |
| `symbol` | TAGS `@name` (중첩이면 `Parent.method` 형태로 조립). 폴백 시 basename. | |
| `path` | `file.new_path` (rename이면 new) | 프론트 `path.split('/').pop()` = spine file. |
| `status` | 상수 `"pending"` | 프론트가 render 시 무시·재계산(계약 §2). 정확성 위해 유효값만 전송. |
| `summary` | 카드 내 add/del 카운트로 생성: `"Updates {symbol}: +{adds} −{dels} lines."` (대문자 시작) | 프론트가 첫 글자 소문자화해 aiSeed에 씀 → 대문자 시작 문장 형태 준수. |
| `lines[].t` | `"add"|"del"|"ctx"` only | git2 `DiffLineType` 매핑. 그 외(파일/헝크 헤더)는 emit 안 함. |
| `lines[].c` | blob에서 추출한 raw 라인, 선행 탭 포함, 후행 `\n` 제거, `+/-` 마커 없음 | git2 `line.content()`는 `\n` 포함 → `trim_end_matches(['\n','\r'])`. |
| `lines[].n` | add/ctx=`new_lineno`, del=`old_lineno` (u32) | 계약상 "각 라인에 합리적 정수". |

> `n` 주의: 계약 [1]은 SummaryScreen의 ref가 `lines[t.lineN].n`을 쓰지만 `t.lineN`은 **배열 인덱스**(0-base)라고 못박음. 즉 `n`은 게터 표시값일 뿐, 스레드 앵커는 인덱스다. 1단계는 스레드를 emit하지 않으므로 영향 없음 — 단 §7에 영속화 시 `lineN`=0-base 인덱스로 고정한다고 명시.

---

## 4. IPC command 시그니처 + 프론트 연동

### 4.1 Rust 쪽 — `lib.rs`

```rust
mod engine;

#[tauri::command]
fn load_review(repo_path: String, base: String, target: String)
    -> Result<engine::ReviewData, String>
{
    engine::build_review(&repo_path, &base, &target)
        .map_err(|e| e.to_string())   // Err → JS에서 throw로 잡힘
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet, load_review])  // ← greet 옆에 등록 (최빈 누락 지점)
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- 인자는 Rust `snake_case`(`repo_path`) ↔ 프론트 `camelCase`(`repoPath`) 자동 매핑.
- **capabilities/tauri.conf.json 수정 불필요** (조사 [4] §3): 자체 `#[tauri::command]`는 capability 권한 항목이 필요 없다. 추후 `plugin-dialog`로 repo 경로를 GUI 선택하게 하면 그때만 권한 추가.

### 4.2 프론트 쪽 — 교체 지점은 `App.jsx`

현재 `App.jsx:7`의 `import { cards } from './data/fixtures'`가 정적 결합. 이 한 줄을 제거하고 IPC + 로딩/에러 상태로 교체한다. `fixtures.js`의 `highlightGo`는 **유지**(색칠은 프론트 책임).

```jsx
// App.jsx 상단
import { invoke } from '@tauri-apps/api/core';   // v2 경로 ('/tauri' 아님)
import { highlightGo } from './data/fixtures';    // 색칠 헬퍼는 계속 사용

export default function App() {
  const [cards, setCards] = React.useState(null);   // null=로딩, []=빈 diff
  const [loadError, setLoadError] = React.useState(null);

  // 1단계: repo/base/target은 하드코딩(또는 Onboarding 입력). dialog 선택은 §7.
  React.useEffect(() => {
    invoke('load_review', {
      repoPath: '/path/to/repo',     // TODO: Onboarding에서 받기
      base: 'main',
      target: 'agent/refactor-auth',
    })
      .then((data) => { setCards(data.cards); setLoadError(null); })
      .catch((err) => { setLoadError(String(err)); setCards([]); });
  }, []);

  // --- 가드: 로딩/에러/빈 결과를 기존 렌더 위에서 처리 ---
  if (loadError) return <LoadErrorScreen message={loadError} onRetry={/* 위 effect 재실행 */} />;
  if (cards === null) return <LoadingScreen />;
  if (cards.length === 0) return <EmptyDiffScreen />;   // base==target 등

  // 이하 기존 로직 그대로: index/verdicts/threads/spineItems 전부 cards 파생
  // const card = cards[index]; ...
}
```

**연동 시 반드시 손봐야 할 부수 지점 (cards가 비동기·동적이 되며 깨질 수 있는 곳):**
1. `useState(2)`로 시작하는 `index`(App.jsx:11) → cards가 3개 미만일 수 있으므로 **`useState(0)`**로 변경.
2. `verdicts` seed `{ decodeJSON:'pass', ... }`(App.jsx:13) → 실제 카드 id와 무관하므로 **`{}`**로 변경.
3. `threads` seed(App.jsx:14-20)는 fixtures의 `validate` id에 묶임 → **`[]`**로 변경(1단계는 스레드 미생성).
4. 위 가드들이 cards 로딩 전에 `cards[index]`/`cards.length` 접근하는 hook 순서를 깨지 않도록, 모든 `useState`/`useEffect` 선언 **뒤에** early-return 가드를 둔다(React hooks 규칙).

`ReviewScreen`은 `card.lines`를 그대로 받고 내부에서 `highlightGo(c)` + `t`로 색칠/마커를 만든다(계약 §4) → **ReviewScreen/SummaryScreen은 수정 불필요.** 데이터 형태가 동일하기 때문.

---

## 5. "diff 라인 → 심볼 카드" 알고리즘 (`cards.rs`)

입력: 한 파일의 `FileDiff { new_path, old_path, new_source: Vec<u8>, lines: Vec<DiffLine> }` + 그 파일의 `Vec<Symbol>`.
`Symbol { name, qualified_name, start_row, end_row }` (0-base row, target 파일 좌표계).

```rust
// engine/symbols.rs
pub struct Symbol {
    pub name: String,           // @name
    pub qualified: String,      // 중첩 시 "Parent.method"
    pub start_row: usize,       // 0-base, inclusive (node.start_position().row)
    pub end_row: usize,         // 0-base, inclusive (node.end_position().row)
}
// gitdiff.rs
pub struct DiffLine { pub kind: LineKind, pub new_lineno: Option<u32>, pub old_lineno: Option<u32>, pub content: String }
pub enum LineKind { Add, Del, Ctx }
```

### 알고리즘 (파일 단위)

1. **변경 라인 → 심볼 귀속.** 각 diff 라인 중 `Add`/`Ctx`는 `new_lineno`(0-base 변환)로 target 좌표에 위치. `Del`은 target에 좌표가 없으므로 **직전·직후 컨텍스트 라인의 심볼에 흡수**(헝크 단위로 처리: 헝크가 닿는 심볼에 그 헝크의 del을 함께 넣음).
2. **포함 심볼 선택 = 가장 안쪽(좁은 범위).** target 라인 `L`에 대해 `start_row ≤ L ≤ end_row`인 심볼 중 `(end_row − start_row)`가 최소인 것 선택 → 중첩 클래스/메서드에서 메서드 우선(조사 [3] §3). 구현은 심볼을 범위 너비 오름차순 정렬 후 첫 포함, 또는 `named_descendant_for_point_range` 후 부모 상향(둘 중 [3]이 권장한 후자가 정렬 불필요·O(depth)).
3. **카드 = (심볼 ∩ 그 심볼에 변경이 있는 헝크).** 한 심볼에 귀속된 헝크들을 모아 하나의 카드를 만든다. 카드 `lines`는 **각 헝크의 변경 + 위/아래 컨텍스트 3줄**(git2 `context_lines(3)`)을 헝크 순서대로 이어 붙임. 같은 심볼에 헝크가 여러 개면 한 카드 안에서 헝크 사이에 구분 없이(또는 ctx gap) 연결 — 1단계는 단순 연결.
4. **심볼 밖 변경 (import/공백/파일상단)** → 그 파일의 **"파일 레벨 카드"** 하나로 합침. `symbol=basename`, `id="{path}::__file"`. 미지원 언어(`Lang::from_path`=None)·파서 ERROR(`tree.root_node().has_error()`) 파일도 전부 파일레벨 카드로 폴백.
5. **작은 파일 합치기 규칙.** 한 파일의 변경된 심볼 카드 수 ≤ **2** **또는** 파일 전체 변경 라인 수 ≤ **12**이면, 심볼별로 쪼개지 않고 **파일 1카드**로 병합(`symbol=basename`). 큰 파일/큰 클래스는 심볼(메서드)별 카드 유지. (상위 결정 "작은 파일은 한 카드, 큰 클래스는 메서드별"의 구체화 — 임계값은 상수로 두어 튜닝 가능: `const SMALL_FILE_MAX_SYMBOLS: usize = 2; const SMALL_FILE_MAX_CHANGED: usize = 12;`)
6. **id 충돌 회피.** 같은 파일 내 동일 `qualified`가 둘 이상이면(오버로드 등) 등장 순서로 `::{i}` suffix.
7. **카드 순서(1단계).** 파일은 diff 등장 순, 파일 내 카드는 `start_row` 오름차순. 같은 파일 카드가 연속 → `chapter`(basename) 연속 그룹이 ProgressSpine에서 한 섹션. **호출그래프 정렬은 2단계** — 이 순서를 만드는 함수 `order_cards()`를 따로 두어 2단계에 교체 가능하게 한다.

```rust
// engine/cards.rs
pub fn build_file_cards(file: &FileDiff, symbols: &[Symbol], out: &mut Vec<ReviewCard>) {
    let hunks = group_lines_into_hunks(&file.lines);           // 컨텍스트 경계로 헝크 분할
    let assign = assign_hunks_to_symbols(&hunks, symbols);     // 헝크 → innermost symbol | None(파일레벨)
    let total_changed = file.lines.iter().filter(|l| l.kind != Ctx).count();

    let symbol_hunks: Vec<_> = /* assign에서 Some(sym)인 것 그룹 */;
    if symbol_hunks.len() <= SMALL_FILE_MAX_SYMBOLS || total_changed <= SMALL_FILE_MAX_CHANGED {
        out.push(make_file_level_card(file, &hunks));          // 규칙 5: 파일 1카드
        return;
    }
    for (sym, hunks) in symbol_hunks { out.push(make_symbol_card(file, sym, &hunks)); }
    let orphan: Vec<_> = /* assign이 None인 헝크 */;
    if !orphan.is_empty() { out.push(make_file_level_card_from(file, &orphan)); }  // 규칙 4
}
```

각 `make_*_card`는 §3 규칙대로 `lines`를 채운다(`c`=raw, `t`=kind, `n`=lineno). 이 함수들은 git2/tree-sitter를 직접 부르지 않으므로 **순수 단위 테스트 가능**(고정 `FileDiff`+`Symbol` 입력 → 카드 JSON 스냅샷).

---

## 6. 구현 태스크 순서 (각 독립 검증 가능)

| # | 태스크 | 산출/검증 방법 | 의존 |
|---|---|---|---|
| T1 | `Cargo.toml`에 git2 추가, `engine/gitdiff.rs`에 `diff_three_dot()` 작성, `lib.rs`에 임시 디버그 호출 | `cargo run` 시 **stdout에 `path / new_lineno / old_lineno / kind` 출력**. 실제 repo로 눈 검증(`git diff base...target`과 라인 대조). | — |
| T2 | `engine/model.rs` 타입 + `load_review` command (cards는 **파일 레벨 카드만**, 심볼 추출 없이) 등록 | `invoke('load_review', …)`를 브라우저 콘솔/임시 버튼에서 호출 → **`{cards:[…]}` JSON 수신**. serde 키가 `n/t/c`인지 확인. | T1 |
| T3 | `engine/symbols.rs`: Go grammar만 `TAGS_QUERY`로 `extract()` | `cargo test`: 고정 Go 소스 → 기대 `Symbol{name,start_row,end_row}` 비교. (Java/Rust는 같은 패턴 복제) | — (병렬 가능) |
| T4 | `engine/cards.rs`: 헝크 분할 + 심볼 귀속 + 합치기 규칙. `build_review` 완성 | `cargo test`: 합성 `FileDiff`+`Symbol`로 카드 분할/파일레벨 폴백/작은파일 병합 **스냅샷 테스트**(`tempfile`로 mini-repo 만들어 e2e도 1개). | T1,T3 |
| T5 | Java/Rust grammar 추가 + `Lang::from_path` 디스패치 + 미지원/ERROR 폴백 | 각 언어 샘플 repo로 `load_review` 호출 → 카드 생성 확인. 미지원 확장자가 파일레벨로 떨어지는지. | T3,T4 |
| T6 | 프론트 연결: `App.jsx`에서 fixtures import 제거 → `invoke` + 로딩/에러/빈상태 가드 + index/verdicts/threads seed 정리(§4.2) | **앱 실행 → 실제 diff 카드가 ReviewScreen에 색칠되어 표시**, Space/F/J/K 동작. base==target일 때 EmptyDiffScreen. | T2,T4 |
| T7 | 마감: `cargo tree -d`로 tree-sitter 중복 점검, 에러 메시지 사용자 친화화, `greet` command 제거 | CI 그린. 잘못된 repo 경로/존재하지 않는 base → 빨간 에러 화면. | T5,T6 |

**T1·T2까지가 "수직 슬라이스"**(git→IPC→프론트 빈카드)로 IPC 배선을 먼저 증명하고, T3~T5에서 심볼 추출을 채워 카드를 풍부하게 만든 뒤, T6에서 fixtures를 최종 교체하는 흐름. T3는 T1과 무관하게 병렬 착수 가능.

---

## 7. 알려진 리스크 & 1단계에서 의도적으로 미루는 것

**리스크 (조사 종합):**
- **ABI/버전 스큐** (조사 [3]): grammar별 tree-sitter 타깃이 0.23~0.26 분산. 코어 0.25 고정 + `set_language()` `Result` 처리 + CI `cargo tree -d`. → T7에서 차단.
- **파서 ERROR 노드** (Kotlin뿐 아니라 복잡한 제네릭/매크로): `tree.has_error()` 가드 → 파일레벨 폴백(§5)으로 안전 강등. 절대 panic 금지.
- **git2 빌드** (`vendored-libgit2`): macOS C 툴체인 필요(Xcode 보유). 크로스컴파일 시 feature 명시 유지.
- **rename**: `find_similar()` 적용 후 `delta.new_file()` 경로 사용. old만 있는 삭제 파일은 카드화하지 않거나 "삭제됨" 파일레벨 카드(1단계는 new가 없으면 skip).
- **바이트 vs 문자 컬럼**: `n`(라인번호)은 영향 없음. `c`는 raw UTF-8 문자열 그대로 전달하므로 한글/이모지 안전(컬럼 계산을 안 함).
- **大 diff 성능**: 1단계는 동기 `invoke` → 매우 큰 repo에서 UI 블로킹 가능. 임계 넘으면 `tauri::async_runtime`/스트리밍은 2단계.

**의도적으로 미루는 것 (인터페이스는 확장 가능하게 유지):**
- **휴리스틱 호출그래프 정렬** → `cards.rs::order_cards()` 격리. 1단계는 (path, start_row) 안정정렬. 교체 지점 1곳.
- **AI 스레드/요약 생성** → `summary`는 기계 생성 문장, `threads`는 emit 안 함(프론트가 `[]`로 시작). Rust가 추후 thread를 영속·전송할 때의 shape는 계약 [1] §3 고정: `lineN` = **0-base `card.lines` 인덱스**.
- **Kotlin grammar** → `Lang::from_path`가 `.kt`를 None 처리 → 파일레벨 카드. grammar 성숙 후 `tree-sitter-kotlin-ng`+번들 `.scm`로 추가(§1 근거).
- **상태 영속화(verdict/thread를 커밋 SHA에 묶어 저장)** → 1단계는 IPC가 `cards`만 반환, 상태는 프론트 메모리. 저장은 별도 command(`save_review_state`)로 2단계.
- **repo/base/target GUI 선택**(`plugin-dialog`/`plugin-fs`) → 1단계는 하드코딩/Onboarding 텍스트 입력. 추가 시 capabilities 권한 보강 필요(조사 [4] §3).
- **작업트리(unstaged) diff** → 1단계는 tree-to-tree(SHA-bound)만. workdir diff는 후순위.

---

### 한 줄 요약
git2(3-dot, 벤더드)로 per-line diff를 받아 → tree-sitter `TAGS_QUERY`(Go/Java/Rust)로 심볼 경계를 잡고 → `cards.rs`에서 "변경 라인 ∩ innermost 심볼 + 작은파일 병합/폴백" 규칙으로 `ReviewCard[]`를 만들어 → serde(`n/t/c` rename)로 계약 [1]에 정확히 맞춰 `load_review` command가 반환하고 → `App.jsx`가 `fixtures` import를 `invoke`+로딩/에러 가드로 교체한다. 정렬·AI·Kotlin·영속화는 격리된 확장점으로 남긴다.

**핵심 파일:** `/Users/jiwon/desktop/projects/loupe/src-tauri/Cargo.toml`, `/Users/jiwon/desktop/projects/loupe/src-tauri/src/lib.rs`, 신규 `/Users/jiwon/desktop/projects/loupe/src-tauri/src/engine/{mod,model,gitdiff,symbols,cards}.rs`, `/Users/jiwon/desktop/projects/loupe/src/App.jsx`(교체 지점: line 7 import / line 11 index / line 13 verdicts / line 14 threads), 유지: `/Users/jiwon/desktop/projects/loupe/src/data/fixtures.js`의 `highlightGo`.
