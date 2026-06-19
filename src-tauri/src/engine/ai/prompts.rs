//! Prompt + JSON-schema constants for the AI pipeline.
//!
//! Stage-④ fills the **clustering** prompt + schema (this file). Ordering (AI2) and
//! labelling (AI3) prompts remain placeholders until Stages ⑤/⑥. What lives here:
//!
//!  - the fixed M1 rules for building schemas (no dynamic key maps; flatten to arrays;
//!    `additionalProperties: false` on every object; no unsupported constraints such
//!    as recursion / `minimum` / `maxLength`),
//!  - a tiny schema builder used by the tests to prove the structured-output plumbing,
//!  - [`CLUSTER_SYSTEM`]: the v2.1 **seed-correction** clustering system prompt,
//!  - [`cluster_output_schema`]: the M1 flattened clustering-output schema.

use serde_json::{json, Value};

/// AI1 — clustering system prompt (v2.1 **seed correction**, planning §4 / §10).
///
/// The model is told the seeds are *algorithmic proposals* it may freely reconstruct
/// (merge / split / move), then constrained by the §10 hard rules. The real safety net
/// is the whitelist verifier (`ai::verify`), so this prompt is supporting, not load-
/// bearing. Identifiers are **card ids** throughout (the AI must echo card ids).
pub const CLUSTER_SYSTEM: &str = "\
You group the changed symbols of a code review into clusters a reviewer can read as one \
coherent change of behaviour.

You are given `seeds`: first-pass groupings the algorithm inferred from STRONG relations \
(same class, direct call chains, Repository↔Entity, signature types). Treat each seed as \
a STARTING POINT, not a verdict. You are free to reconstruct them:

(a) MERGE seeds that are actually one flow but static analysis could not connect — the \
    meaning-level links it misses: event publish↔subscribe, interface↔implementation, \
    dependency injection, request→command→domain→entity→response transforms.
(b) SPLIT a seed that is really two distinct behaviours bundled together.
(c) PLACE any unplaced symbol into the cluster whose behaviour it belongs to, then \
    finalize the meaning clusters.

Some seeds also carry BASE-vs-HEAD signals (the algorithm compared the old and new code, \
not just the diff). Use them:
  - `renamePairs` / a symbol's `renamedFrom`: that symbol is the SAME code under a new \
    name (its body matched a symbol deleted from the base). Treat the rename as ONE change \
    — keep the renamed symbol in the cluster the old name belonged to; never split a rename \
    into a separate \"new symbol\" cluster.
  - `deletedSymbols`: symbols removed from a file that still exists. They have no card id \
    (do NOT put them in any `memberCardIds` or in `unclustered`); use them only as context \
    — keep the surviving change that replaced or depended on them in the related flow.
  - a symbol's `signatureChange` (`old → new`): its contract changed before→after; cluster \
    it with the callers/types affected by that signature.

Classify each cluster's `kind` as exactly one of:
  - flow              : one user/system action end to end (entrypoint→controller→\
usecase→domain→repo→test).
  - contract          : API schema / DTO / event payload / migration / config contract \
change.
  - domain-concept    : a new domain concept or policy introduced (e.g. a new policy \
class).
  - shared-foundation : shared base logic several flows depend on (do NOT force it into \
one flow).
  - infra             : configuration / build / DI / feature-flag / environment change.

INFRA / CONFIG FILES (no code symbols — name is a file PATH, kind=config/migration/file): \
group them by TOOL and PURPOSE into `infra` clusters (or `contract` for a \
migration/schema), do NOT leave them in `unclustered` just because they have no call graph. \
Put files that serve the SAME tool or goal together, e.g.:
  - `.github/workflows/*.yml`, CI/deploy scripts → one CI/CD cluster.
  - `Cargo.toml` + `Cargo.lock` (or package.json + lockfile) → one dependency cluster.
  - `Caddyfile` / `*.caddy` / reverse-proxy + TLS config → one caddy/proxy cluster.
  - `Dockerfile` / `docker-compose*` / `.dockerignore` → one container cluster.
  - migrations / `*.sql` / `.sqlx/*` → one DB/schema cluster (kind=contract).
A file that shares NO tool or purpose with any other change is the only thing that may go \
to `unclustered`. Prefer a small, well-named infra cluster over Unclustered.

Hard rules:
  - Use ONLY the card ids present in the input. Never invent a symbol or a card id.
  - Do not assume side effects that are not in the cards.
  - Do not claim tests exist that are not provided.
  - When uncertain, do not assert — keep symbols in their seed grouping rather than \
guessing a merge.
  - Every input card id must appear in exactly one cluster's `memberCardIds`, OR in \
`unclustered`. Never drop a card id.

Output only the structured JSON the schema defines.";

/// AI2 — ordering system prompt (Stage ⑤, planning §4.5 / §6.2).
///
/// Receives the clustering result (each cluster's `memberCardIds`) plus the per-pair
/// relation hints, and decides **two** orders: inside each cluster (flow / top-down,
/// caller→callee, code-appearance order when one function calls several changed ones)
/// and across clusters (entrypoint / flow first). Identifiers are **card ids**; the
/// output must be a permutation of the input ids (no new ids, none dropped) — the
/// whitelist verifier (`ai::verify`) enforces this, so this prompt is supporting.
pub const ORDER_SYSTEM: &str = "\
You order the changed symbols of a code review so a reviewer can follow them in execution \
flow. The clusters are already decided — do NOT re-cluster. You only decide ORDER.
You are given `clusters` (each with `memberCardIds`) and `relationHints` (`strong`/`weak` \
caller↔callee / type pairs, by card id).

Decide two things:

1. ORDER WITHIN each cluster — top-down execution flow:
   - caller BEFORE callee (a function appears before the functions it calls).
   - when one function calls several changed functions, follow the order they appear in \
the code (the relation hints list a caller's callees in code order).
   - a request/response/DTO/type used in a function's signature comes before that function.
   This is the order a human reads code: enter at the top, follow each call down, then come \
back up for the next flow.

2. ORDER ACROSS clusters (`clusterOrder`) — entrypoint / flow first:
   - a cluster that contains an entrypoint or starts a user/system flow comes first;
   - shared-foundation / infra clusters that several flows depend on come later (or first \
only if a flow can't be read without them);
   - keep clusters that belong to the same flow adjacent.

Hard rules:
  - Use ONLY the card ids present in the input. Never invent or drop a card id.
  - Every input card id must appear exactly once in its cluster's `cardIds`.
  - `clusterOrder` must list every input `clusterId` exactly once.
  - When uncertain about two symbols' order, keep their input order.

Output only the structured JSON the schema defines.";

/// AI1+AI2 combined system prompt (small-PR branch, planning §4.1). One call clusters
/// AND orders. Built by concatenating the clustering rules with the ordering rules so a
/// single Haiku call returns clusters whose `memberCardIds` are already in flow order
/// plus a `clusterOrder`. Used only when the PR is small (≤ `SMALL_PR_SYMBOLS`).
pub const CLUSTER_AND_ORDER_SYSTEM: &str = "\
You both GROUP and ORDER the changed symbols of a small code review in one pass.

PART A — GROUP the symbols into clusters a reviewer reads as one coherent change of \
behaviour. You are given `seeds`: first-pass groupings the algorithm inferred from STRONG \
relations (same class, direct call chains, Repository↔Entity, signature types). Treat each \
seed as a STARTING POINT, not a verdict. You may MERGE seeds that are one flow static \
analysis could not connect (event publish↔subscribe, interface↔implementation, dependency \
injection, request→command→domain→entity→response), SPLIT a seed that is really two \
behaviours, and PLACE any unplaced symbol where it belongs.

Use the BASE-vs-HEAD signals when present: `renamePairs` / a symbol's `renamedFrom` means \
that symbol is the SAME code renamed — keep it as ONE change in the old name's cluster, do \
not split rename into delete+add. `deletedSymbols` (no card id — never place them in \
`memberCardIds`/`unclustered`) are context: keep the surviving change that replaced them in \
the related flow. A symbol's `signatureChange` (`old → new`) is a before→after contract \
change — cluster it with the callers/types it affects.

Classify each cluster's `kind` as exactly one of: flow, contract, domain-concept, \
shared-foundation, infra.

INFRA / CONFIG FILES (no code symbols — the name is a file PATH): group them by TOOL and \
PURPOSE into `infra` clusters (or `contract` for a migration/schema) instead of leaving \
them `unclustered`. Same tool/goal together — `.github/workflows`=CI, Cargo.toml+Cargo.lock=\
dependencies, Caddyfile/*.caddy=caddy/proxy, Dockerfile/docker-compose=container, \
migrations/*.sql/.sqlx=DB schema. Only a file sharing no tool/purpose with anything else \
may go to `unclustered`.

PART B — ORDER, in the SAME response:
  - `memberCardIds` inside each cluster must be in top-down execution flow (caller before \
callee; a signature type before the function using it; a caller's several changed callees \
in code-appearance order).
  - `clusterOrder` lists clusters entrypoint/flow first, shared-foundation/infra later.

Hard rules:
  - Use ONLY the card ids present in the input. Never invent, never drop a card id.
  - Every input card id appears in exactly one cluster's `memberCardIds`, or in \
`unclustered`. `clusterOrder` lists every emitted `clusterId` exactly once.
  - Do not assume side effects or tests that are not in the cards.
  - When uncertain, keep symbols in their seed grouping and keep input order.

LANGUAGE: this call emits only ids/kinds (no prose); the later labelling call writes \
titles/summaries in 한국어 with code identifiers (심볼명/타입명) kept verbatim in 영문 원문.

Output only the structured JSON the schema defines.";

/// AI3 — labelling system prompt (Stage ⑥, planning §6.2 / §6.3 / §10).
///
/// Batched: receives ALL clusters' changed symbols in ONE call and returns a title +
/// 1–3-sentence summary per cluster (never per-cluster N calls). Title format =
/// `[target] + [change action]`, short. Also emits merge/split suggestions, but only
/// when clearly warranted (display-only, §6.3). Identifiers in the text must come from
/// the input symbols; a loose token check (`ai::verify::suspicious_identifiers`) is the
/// safety net behind the "don't assert" rule.
pub const LABEL_SYSTEM: &str = "\
You name and summarize each cluster of a code review, AND write a one-sentence summary for \
each individual changed card. You are given `clusters`, each with its `clusterId`, an \
algorithmic `kind`, and its `changedSymbols` (each: `cardId` + name + kind + change + an \
optional `snippet` of the actual added/removed diff lines). For EVERY cluster return:

  - `title`: the change in the form [target] + [change action], short (e.g. \"주문 생성 시 \
쿠폰 할인 적용\", \"결제 실패 이벤트 재시도 정책 변경\"). Never empty.
  - `summary`: 1 to 2 sentences stating the cluster's ONE overall INTENT — what behaviour \
or capability this group of changes achieves, as a reviewer would describe its purpose. \
Never empty.
  - `cardSummaries`: one entry `{cardId, summary}` for EACH `cardId` in this cluster's \
`changedSymbols`. Each `summary` is ONE Korean sentence describing what THAT specific \
card's change does — the behaviour/intent of that single symbol's change (what it was \
made to do), grounded in its `snippet`. This is per-card detail, distinct from the \
cluster `summary`.

The cluster `summary` is the cluster's INTENT, not an inventory of its parts. Per-symbol \
detail (which functions changed, what each one does, how many lines, +N/−M counts) belongs \
in `cardSummaries` and on each card — do NOT repeat per-card detail in the cluster `summary`.

LANGUAGE (한국어 확정): Write `title`, `summary`, and every `cardSummaries[].summary` in \
KOREAN (자연어 설명은 한국어로). But keep all code identifiers VERBATIM in their original \
form — symbol names, method names, class/type names, field names, file paths, and API \
routes stay as written in the code (영문 식별자 원문 유지), never translated or \
transliterated. Example cluster summary: \"주문 생성 흐름에 쿠폰 할인을 도입한다\". Example \
card summary: \"createOrder가 주문 생성 시 쿠폰 할인을 계산해 합계에 반영하도록 바뀐다\".

You MAY also suggest cluster merges/splits in `mergeSuggestions` / `splitSuggestions`, but \
ONLY when clearly warranted — leave them empty otherwise (these are display-only hints, \
never applied automatically). Reference clusters by `clusterId`.

Hard rules:
  - The cluster `summary` describes the cluster's single overall intent/behaviour change. \
Do NOT enumerate individual symbols, do NOT list every changed function/type one by one, \
and do NOT include line counts or +N/−M / added/removed statistics — those live in \
`cardSummaries` and on the cards.
  - Each `cardSummaries[].summary` is exactly ONE Korean sentence about that one card's \
change. State what the symbol does now (its behaviour/intent), not statistics — no line \
counts, no +N/−M. Base it on the `cardId`'s name/change/`snippet`; if the snippet is \
absent or unclear, describe the change plainly rather than guessing.
  - Use ONLY the `cardId` values present in the cluster's `changedSymbols`. Never invent a \
cardId, a symbol, a class, a method, or a side effect (M4).
  - Do not claim tests exist that are not provided.
  - Keep the cluster `summary` to 1–2 sentences and each card summary to 1 sentence; do not pad.
  - When uncertain, describe plainly rather than asserting a cause/effect you can't see.

Output only the structured JSON the schema defines.";

/// The set of `kind` enum strings the clustering output may use (kebab-case, matching
/// `ClusterKind`'s serde rename). Kept here so the schema and the deserializer agree.
pub const CLUSTER_KINDS: &[&str] = &[
    "flow",
    "contract",
    "domain-concept",
    "shared-foundation",
    "infra",
];

/// AI1 — clustering output schema (M1). **Flattened, no dynamic key maps**:
///
/// ```json
/// {
///   "clusters": [ { "clusterId": str, "memberCardIds": [str], "kind": <enum> } ],
///   "unclustered": [ str ]
/// }
/// ```
///
/// Every object carries `additionalProperties: false` and requires all fields (via
/// `object_schema`). `kind` is an `enum` over [`CLUSTER_KINDS`]. No recursion / `minItems`
/// / `maxLength` (unsupported — never added).
pub fn cluster_output_schema() -> Value {
    let kind = enum_schema(CLUSTER_KINDS);
    let cluster_item = object_schema(&[
        ("clusterId", string_schema()),
        ("memberCardIds", array_schema(string_schema())),
        ("kind", kind),
    ]);
    object_schema(&[
        ("clusters", array_schema(cluster_item)),
        ("unclustered", array_schema(string_schema())),
    ])
}

/// AI2 — ordering output schema (Stage ⑤, M1 **flattened**). The natural shape is a
/// dynamic key map `{clusterId: [cardId]}`, which json_schema forbids (M1), so it is
/// modelled as an **array of fixed-key objects**:
///
/// ```json
/// {
///   "clusterOrder":    [ str ],
///   "orderedByCluster": [ { "clusterId": str, "cardIds": [str] } ]
/// }
/// ```
///
/// Every object is `additionalProperties: false` + all-required. No dynamic key maps.
pub fn order_output_schema() -> Value {
    let ordered_item = object_schema(&[
        ("clusterId", string_schema()),
        ("cardIds", array_schema(string_schema())),
    ]);
    object_schema(&[
        ("clusterOrder", array_schema(string_schema())),
        ("orderedByCluster", array_schema(ordered_item)),
    ])
}

/// AI1+AI2 combined output schema (small-PR branch, M1). One object carries the
/// clustering (`clusters` with ordered `memberCardIds` + `kind`, plus `unclustered`)
/// AND the inter-cluster order (`clusterOrder`). All flattened, all fixed-key.
pub fn cluster_and_order_output_schema() -> Value {
    let kind = enum_schema(CLUSTER_KINDS);
    let cluster_item = object_schema(&[
        ("clusterId", string_schema()),
        ("memberCardIds", array_schema(string_schema())),
        ("kind", kind),
    ]);
    object_schema(&[
        ("clusters", array_schema(cluster_item)),
        ("unclustered", array_schema(string_schema())),
        ("clusterOrder", array_schema(string_schema())),
    ])
}

/// AI3 — labelling output schema (Stage ⑥, M1). title/summary + per-card summaries per
/// cluster + display-only merge/split suggestions. `cardSummaries` is an array of fixed-key
/// `{cardId, summary}` objects (M1-flat — never a dynamic `{cardId: summary}` map):
///
/// ```json
/// {
///   "clusters": [ {
///       "clusterId": str, "title": str, "summary": str,
///       "cardSummaries": [ { "cardId": str, "summary": str } ]
///   } ],
///   "mergeSuggestions": [ { "clusterIds": [str], "reason": str } ],
///   "splitSuggestions": [ { "clusterIds": [str], "reason": str } ]
/// }
/// ```
pub fn label_output_schema() -> Value {
    let card_summary_item = object_schema(&[
        ("cardId", string_schema()),
        ("summary", string_schema()),
    ]);
    let label_item = object_schema(&[
        ("clusterId", string_schema()),
        ("title", string_schema()),
        ("summary", string_schema()),
        ("cardSummaries", array_schema(card_summary_item)),
    ]);
    let suggestion_item = object_schema(&[
        ("clusterIds", array_schema(string_schema())),
        ("reason", string_schema()),
    ]);
    object_schema(&[
        ("clusters", array_schema(label_item)),
        ("mergeSuggestions", array_schema(suggestion_item.clone())),
        ("splitSuggestions", array_schema(suggestion_item)),
    ])
}

/// A string `enum` schema (M1-allowed). Used for the cluster `kind` field.
pub fn enum_schema(values: &[&str]) -> Value {
    json!({
        "type": "string",
        "enum": values.iter().map(|v| Value::String((*v).to_string())).collect::<Vec<_>>(),
    })
}

/// Build a flat object schema from `(field_name, field_schema)` pairs, always setting
/// `type: "object"`, `additionalProperties: false`, and requiring every field. This is
/// the only sanctioned way to build object schemas (M1): callers never hand-write
/// `additionalProperties` and never introduce dynamic key maps.
pub fn object_schema(fields: &[(&str, Value)]) -> Value {
    let mut props = serde_json::Map::new();
    let mut required = Vec::with_capacity(fields.len());
    for (name, schema) in fields {
        props.insert((*name).to_string(), schema.clone());
        required.push(Value::String((*name).to_string()));
    }
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": Value::Object(props),
        "required": Value::Array(required),
    })
}

/// An array-of-`item` schema. Used to *flatten* what would otherwise be a dynamic key
/// map: e.g. `{clusterId: [...]}` is forbidden (M1), so the AI2 output is modelled as
/// `[{clusterId, cardIds: [...]}]` — an array of fixed-key objects.
pub fn array_schema(item: Value) -> Value {
    json!({ "type": "array", "items": item })
}

/// A bare string schema.
pub fn string_schema() -> Value {
    json!({ "type": "string" })
}
