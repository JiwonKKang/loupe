I have everything I need. The evidence from the actual codebase confirms several concrete contract violations the design either missed or got subtly wrong. Here is my adversarial review.

---

# Loupe 1단계 설계서 — 적대적 리뷰

코드(`App.jsx`, `fixtures.js`, `ReviewScreen.jsx`, `SummaryScreen.jsx`, `lib.rs`, `Cargo.toml`)와 대조해 검증함. 설계는 **대체로 견고하나**, 프론트 계약과 어긋나는 점이 2개(blocker 1, major 1) 확인되고, crate/엣지케이스에서 major급 함정이 다수 있다.

---

## (1) 프론트 데이터 계약 위반

### B1 [blocker] `summary`가 없으면 `aiSeed()`가 런타임 크래시
`App.jsx:64-66`:
```js
function aiSeed(c) {
  return `This line is part of: ${c.summary.charAt(0).toLowerCase() + c.summary.slice(1)} Ask anything...`;
}
```
`c.summary`에 `.charAt`/`.slice`를 **무방비로** 호출한다. 설계는 모든 카드에 기계 생성 `summary`를 넣겠다고 했으나, `make_file_level_card`/폴백 경로에서 빈 문자열이거나 누락되면 `undefined.charAt` → throw. **계약상 `summary`는 항상 non-empty여야 한다.**
- 수정: `model.rs`에서 `summary: String`을 절대 빈 문자열로 두지 말 것. 폴백 카드도 `"Updates {basename}: +{a} −{d} lines."`를 보장. 단위 테스트에 "모든 카드 summary.len() > 0" 불변식 추가.

### B2 [major] `lines[].n` 값 규칙이 프론트 표시와 어긋남 — del 라인 번호
`ReviewScreen.jsx:150`은 `t`와 무관하게 `{ln.n}`을 그대로 찍는다. 설계 §3은 `del`일 때 `n = old_lineno`라 했는데, **fixtures의 실제 동작(`fixtures.js:38-45`)은 del 라인도 인접 라인과 같은 new 좌표 번호를 쓴다**(`n: t === 'del' ? n : n` — 즉 항상 같은 카운터). 즉 기존 UI는 "한 거터에 단조 증가하는 new-기준 번호"를 전제로 디자인됨. del에 `old_lineno`를 넣으면 거터 번호가 들쭉날쭉(예: 52,53,**51**,53...)해져 시각적으로 깨진다.
- 수정: del 라인의 `n`도 "그 위치에 해당하는 new 좌표(직전 ctx의 new_lineno)"로 채워 단조성 유지. `old_lineno`는 1단계에서 사용처가 없음. 설계 §3 표의 "del = old_lineno" 항목을 정정.

### M3 [minor] `id` 안정성 — 프론트가 `id`를 React key/verdict/thread 키로 씀
`App.jsx:27-30, 40, 43`에서 `c.id`가 spine key, `verdicts[card.id]`, thread 매칭의 1차 키다. 설계의 `::{idx}` 충돌 회피는 OK지만, **카드 순서가 바뀌면(2단계 정렬) idx suffix가 바뀌어 같은 심볼의 id가 달라진다** → 영속화(2단계) 시 verdict가 유실. 1단계 영향은 없으나 §7에서 "id는 정렬 불변이어야 한다"를 명시하고 suffix를 등장순이 아니라 `start_row` 기반 등 안정 키로.

---

## (2) Crate 선택의 숨은 함정

### M4 [major] `tree-sitter-java = "0.23"` — 코어 0.25와 ABI/버전 스큐
설계 표가 java만 0.23, rust 0.24, go 0.25로 **제각각**이다. tree-sitter 0.25 코어에 0.23 타깃 grammar를 `set_language`하면 ABI mismatch로 **런타임 `LanguageError`** 가능(컴파일은 통과). 설계는 `cargo tree -d`로 "중복"만 본다는데, 그건 중복 검출이지 **ABI 호환 검출이 아니다.**
- 수정: 각 grammar crate가 의존하는 tree-sitter 버전을 `cargo tree -i tree-sitter`로 실제 확인하고, 코어를 grammar들이 공통 허용하는 범위로 핀. T3에 "각 언어 `Parser::set_language()`가 `Ok`인지" 스모크 테스트를 **명시적 검증 항목으로** 추가(현재 T3는 Symbol 비교만 한다).

### M5 [major] `TAGS_QUERY` 상수 존재를 미검증 — 설계 근간 가정
설계 핵심은 "Go/Java/Rust crate가 `TAGS_QUERY` 상수를 번들한다"인데, 이는 crate별로 **보장되지 않는다.** `tree-sitter-rust`는 `TAGS_QUERY`를 제공하지만, `tree-sitter-go`/`tree-sitter-java`의 특정 버전이 `tags.scm`을 export하는지는 버전 의존적이다. 없으면 설계 §1의 "노드 kind 암기 불필요" 전제가 무너지고 `.scm` 직접 번들(=Kotlin을 뺀 바로 그 이유)이 되살아난다.
- 수정: **태스크 T3 이전에 30분 스파이크**로 3개 crate의 `TAGS_QUERY`/tags 캡처 이름(`@definition.function` 등)이 실제로 존재하는지 확인. 없으면 `symbols.rs`가 직접 `Query::new(lang, INLINE_SCM)`을 들고 가도록 fallback 설계를 §5에 미리 넣을 것.

### M6 [major] macOS 코드사인/공증 — `vendored-libgit2` 정적 링크의 사인 영향 미언급
설계는 git2 빌드(C 툴체인)만 다루고 **공증(notarization)을 전혀 안 다룬다.** vendored-libgit2는 정적 링크라 별도 dylib 사인 이슈는 줄지만, tree-sitter grammar들은 각자 C 코드를 컴파일해 최종 바이너리에 들어가므로, 배포 시 hardened runtime + notarization이 필요. 1단계가 "화면에 띄운다"까지면 dev 빌드라 무관하나, **§7 리스크에 "배포 사인/공증은 별도 과제"로 1줄 명시**가 없어 나중에 폭탄. (minor로 강등 가능 — 1단계 스코프가 로컬 실행이면.)

### M7 [minor] 크로스컴파일 — Apple Silicon/Intel universal 바이너리
`vendored-libgit2` + 4개 grammar의 C 빌드를 `aarch64`+`x86_64` universal로 묶으려면 각 타깃 C 툴체인 필요. 1단계 로컬 실행엔 무관하나 설계가 "크로스컴파일 시 feature 유지"라고만 적어 과소평가. 1단계 OUT으로 명시 권장.

---

## (3) 심볼 카드 엣지케이스 — 빠진 것들

설계 §5가 다룬 것: 심볼 밖 변경(파일레벨), 미지원 언어, 파서 ERROR, rename(new 경로), id 충돌, 삭제 파일 skip. 빠지거나 부실한 것:

### M8 [major] 삭제된 파일 — "skip"은 리뷰 누락
§7: "old만 있는 삭제 파일은 카드화하지 않거나 skip(1단계는 new가 없으면 skip)". **삭제는 리뷰에서 가장 중요한 변경 중 하나**인데 통째로 사라진다. 리뷰어가 "이 파일이 지워졌다"를 못 본다.
- 수정: 삭제 파일도 `del`-only 파일레벨 카드 1개로 emit. `new_source`가 없으므로 `old_source`에서 라인 추출, `n`은 old_lineno, `path`는 old_path. 심볼 추출은 생략(파일레벨).

### M9 [major] del 라인의 심볼 귀속 — `new_source`에 좌표가 없는 근본 문제
§5-1은 "del은 인접 ctx의 심볼에 흡수"라지만, **순수 삭제 헝크(앞뒤로 ctx가 같은 심볼이 아니거나, 심볼 전체가 삭제)**는 귀속 대상이 모호하다. 특히 "심볼 본문 전체 삭제 + 그 자리에 다른 심볼"이면 innermost 매칭이 엉뚱한 심볼을 가리킴. tree-sitter는 **target(new) 소스만 파싱**하므로 삭제된 심볼은 트리에 아예 없다.
- 수정: del 귀속을 "헝크 직전 ctx 라인의 new_lineno → 그 좌표의 innermost 심볼"로 결정론적 규칙화. 직전 ctx가 없으면(파일 선두) 직후 ctx 사용. 둘 다 없으면 파일레벨. 이 규칙을 §5에 명문화하고 단위 테스트.

### M10 [major] hunk가 심볼 경계를 가로지름 — 한 헝크 = 한 카드 가정 붕괴
§5-3은 "헝크를 심볼에 귀속"하는데, **context_lines(3) 때문에 한 헝크가 두 함수에 걸치는 경우**(함수 A 끝 + 함수 B 시작이 6줄 이내)가 흔하다. 현재 알고리즘은 헝크 단위로 1개 심볼에 넣으므로, 한 헝크 내 라인들이 서로 다른 심볼에 속해도 한쪽으로 몰린다.
- 수정: 귀속 단위를 **헝크가 아니라 라인**으로 내려야 정확하다. "각 변경 라인 → innermost 심볼" 후 같은 심볼 라인끼리 카드로 묶고, 카드 내에서 ctx 3줄 부여. §5-3의 "헝크 단위 처리"를 "라인 단위 귀속 후 심볼별 재그룹"으로 교체. (이게 §5-1과 §5-3의 내부 모순이기도 함.)

### M11 [minor] 한 심볼이 여러 hunk — 카드 내 연결 시 거터 점프
§5-3은 "헝크 사이 단순 연결"인데, ctx gap 없이 이으면 `ReviewScreen`의 단조 거터 번호(B2 참조)가 점프해 보인다(예: 45,46,...,89,90). 프론트는 hunk 구분선 UI가 없다(확인함).
- 수정: 헝크 사이에 `ctx` placeholder 라인 1개(예: `c: ""` 또는 `…`)를 넣되, 그건 또 `highlightGo("")` 처리/`n` 부재 문제. 차라리 1단계는 **심볼당 단일 연속 범위만 한 카드**로 하고, 떨어진 헝크는 같은 심볼이라도 별 카드로 두는 게 단순·정확. 결정 필요.

### M12 [minor] 바이너리 파일 — 미언급
git2 `Delta`가 binary면 `line.content()`가 무의미/비-UTF8. 설계에 바이너리 가드가 없다.
- 수정: `delta.flags().contains(DiffFlags::BINARY)` 또는 `is_binary()` 체크 → "바이너리 변경" 파일레벨 카드(lines 비움, summary만). `String::from_utf8` 강제 시 비-UTF8 소스에서 패닉/손실 가능 → `from_utf8_lossy` 사용 명시.

### M13 [minor] 새 파일 — 동작은 OK지만 작은파일 병합 규칙과 충돌
신규 파일은 전부 add라 `total_changed`가 커서 §5-5 병합이 안 되고 심볼별로 쪼개짐. 신규 파일을 메서드별로 잘게 쪼개면 "새 파일 통독" 흐름이 깨진다.
- 수정: `is_new_file`이면 병합 임계를 우회해 파일 1카드(또는 파일카드 + 큰 함수만 분리) — 의도적 결정으로 §5에 명시.

---

## (4) 태스크 분해의 독립 검증성

대체로 좋다(T1/T2 수직 슬라이스, T3 병렬). 그러나:

### M14 [major] T2가 `greet`를 안 지우면서 T7로 미룸 — 그 사이 손상 없음, 단 검증 누락
`lib.rs`는 현재 `greet`만 등록. 설계 §4.1은 `load_review`를 추가하지만 T7에서야 `greet` 제거. 문제는 **T2의 검증 방법("브라우저 콘솔에서 invoke")이 프론트 dev 환경 가정**인데, 1단계 IPC는 `@tauri-apps/api/core`가 Tauri 윈도우 안에서만 동작 → 순수 브라우저 콘솔(vite dev)에서는 `invoke`가 없어 검증 불가. 
- 수정: T2 검증을 "임시 버튼 + `tauri dev`로 실행"으로 교정. "브라우저 콘솔" 문구 삭제.

### M15 [minor] T3가 T1과 독립이라 했으나 `Symbol` 좌표계 계약(0-base row, new 좌표)이 T1/T4와 공유
T3 단독 테스트는 가능하나, T4가 의존하는 "row가 new 소스 0-base inclusive"라는 불변식이 T3에서 고정되지 않으면 T4에서 off-by-one. 
- 수정: T3 산출물에 "start_row/end_row는 `node.end_position().row` = **마지막 라인 inclusive**" 단언 케이스(여러 줄 함수)를 못 박을 것. tree-sitter `end_position().row`는 노드의 마지막 바이트가 있는 행이라, 닫는 `}`가 다음 줄이면 그 줄이 됨 — 경계 테스트 필수.

### 검증성 총평
T1~T7는 각각 산출물이 있어 **독립 검증 가능**. 유일한 결함은 T2의 검증 환경 오기재(M14)와 T3/T4 사이 좌표계 계약 미고정(M15).

---

## (5) 1단계 스코프

**대체로 적정.** OUT 목록(정렬/AI/Kotlin/영속화/dialog/workdir)이 명확하고 확장점이 격리됨. 다만:

- **과한 부분:** §5-5의 "작은 파일 병합 임계값(2 심볼 / 12 라인)" 휴리스틱은 1단계에 **과조숙(premature)**이다. 튜닝 상수를 두 개나 도입했는데 1단계 목표("fixtures를 실제 diff로 교체")엔 "파일당 심볼별 카드 + 심볼 밖은 파일카드"만으로 충분. 병합 규칙은 카드가 너무 잘게 쪼개지는 게 **실측으로 확인된 뒤** 추가하는 게 맞다. → 1단계에서 빼고 §7 확장점으로 이동 권장 (minor).
- **부족한 부분:** repo/base/target **하드코딩**(`'/path/to/repo'`)이 1단계 인수 기준을 모호하게 만든다. `App.jsx`의 effect가 빈 deps `[]`라 retry/onboarding 입력과 연결이 안 됨. 최소한 Onboarding 텍스트 입력 → state → invoke 경로는 1단계에 넣어야 "실행 가능한 데모"가 된다(현재 `Onboarding.jsx`는 `onFinish`만 있음). → IN으로 끌어올리거나, 최소 env var로라도 외부 주입 (minor).

---

## 핵심 리스크 Top 3

1. **B1 — `aiSeed`의 `summary.charAt` 무방비 호출** (blocker). 폴백/빈 카드에서 throw. 모든 카드 `summary` non-empty 불변식으로 차단.
2. **M10/M9 — del 라인 & 심볼 경계 교차 귀속의 헝크-단위 처리** (major, 정확성 핵심). target만 파싱하므로 삭제 좌표가 트리에 없고, context 3줄이 심볼 경계를 넘으면 귀속이 틀어진다. **라인 단위 귀속 + 직전 ctx 앵커**로 재설계 필요. 이게 엔진의 정확성을 좌우.
3. **M4/M5 — tree-sitter ABI 스큐 + `TAGS_QUERY` 상수 존재 미검증** (major, 설계 근간 가정). java 0.23 vs 코어 0.25, 그리고 go/java crate의 `TAGS_QUERY` export 여부가 확인 안 됨. **T3 전 30분 스파이크**로 둘 다 실증하지 않으면 설계 전제가 무너질 수 있다.

부차적으로 B2(del 거터 번호 단조성), M8(삭제 파일 카드화)도 데모 품질에 바로 보이는 결함이라 1단계 내 처리 권장.
