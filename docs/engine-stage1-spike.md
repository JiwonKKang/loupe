# Tree-sitter crate 스파이크 결과 (실증 완료)

`/tmp/loupe-ts-spike`에서 실제 빌드·실행으로 검증함. 설계의 "all three expose TAGS_QUERY, core 0.25" 가정 = **TRUE**.

## 확정 crate 버전 (그대로 resolve/build 됨)
```toml
tree-sitter      = "0.25"   # resolved 0.25.10
tree-sitter-go   = "0.25"   # resolved 0.25.0   (ABI 15)
tree-sitter-java = "0.23"   # resolved 0.23.5   (ABI 14 — core 0.25가 수용, set_language Ok)
tree-sitter-rust = "0.24"   # resolved 0.24.2   (ABI 15)
```
- 셋 다 `set_language(&LANGUAGE.into())` → `Ok(())`. ABI 스큐 없음.
- 셋 다 `pub const TAGS_QUERY: &str` + `HIGHLIGHTS_QUERY` 노출. vendoring 불필요.

## 반드시 지킬 API 사실 (틀리기 쉬움)
1. **Loader = `tree_sitter_go::LANGUAGE` (`LanguageFn`)**, NOT `language()`. 변환: `tree_sitter::Language::from(tree_sitter_go::LANGUAGE)` 또는 `LANGUAGE.into()`.
2. **`QueryCursor::matches`는 `StreamingIterator`** (std Iterator 아님). `use tree_sitter::StreamingIterator;` 후 `while let Some(m) = it.next() { ... }`. (`.map`/`for` 안 됨.)
3. **capture 이름이 언어마다 다름** — 고정 triad 가정 금지. 언어별 매핑 필요:
   - Go:   `@definition.function`, `@definition.method`, `@definition.type` (class/interface 없음)
   - Java: `@definition.class`, `@definition.interface`, `@definition.method` (function 없음)
   - Rust: `@definition.{function,method,class,interface,macro,module}`
   - **`@name`은 셋 다 일관** → 식별자 텍스트는 항상 `@name`에서.
4. `end_position().row`는 0-base이고 **노드의 마지막 바이트가 있는 행**(닫는 `}`가 다음 줄이면 그 줄). 멀티라인 심볼 경계 테스트로 확정할 것.

## 심볼 추출 전략
- 각 언어의 `TAGS_QUERY`를 `Query::new(&lang, TAGS_QUERY)`로 컴파일.
- 매치에서 `@definition.*` 캡처 노드 = 심볼 범위(start/end row), 같은 매치의 `@name` 캡처 = 표시명.
- 언어별로 "심볼로 칠 capture 이름 집합"을 매핑 테이블로 둠.
