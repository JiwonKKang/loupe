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

Hard rules:
  - Use ONLY the card ids present in the input. Never invent a symbol or a card id.
  - Do not assume side effects that are not in the cards.
  - Do not claim tests exist that are not provided.
  - When uncertain, do not assert — keep symbols in their seed grouping rather than \
guessing a merge.
  - Every input card id must appear in exactly one cluster's `memberCardIds`, OR in \
`unclustered`. Never drop a card id.

Output only the structured JSON the schema defines.";

/// AI2 — ordering prompt (Stage ⑤). Placeholder.
pub const ORDER_SYSTEM: &str = "";
/// AI3 — labelling prompt (Stage ⑥). Placeholder.
pub const LABEL_SYSTEM: &str = "";

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
