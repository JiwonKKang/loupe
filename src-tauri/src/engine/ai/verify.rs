//! Stage-④ — output verification (M4 whitelist, planning §8.3).
//!
//! "Don't make things up" is not enforceable by a prompt alone. The card ids fed to the
//! AI are the **whitelist**; this module is the real safety net (the prompt is only
//! supporting). Three checks (planning §3.1 / §8.3, v2-critique M4):
//!
//!  1. **Hallucination reject** — every `memberCardId` (and every `unclustered` id) the
//!     AI returns must be in the input whitelist. An id outside it ⇒ `Err` (caller
//!     retries once, then falls back).
//!  2. **No-drop absorption** — any whitelist id the AI omitted (never placed in a
//!     cluster nor in `unclustered`) is **auto-absorbed into `unclustered`** so nothing
//!     vanishes ("all changes are visible", §3.1). This is a normalization, not an error.
//!  3. **Token whitelist (M4 reinforcement)** — a helper that extracts *code-identifier
//!     tokens* from free text (a future title/summary) and intersects them with the
//!     whitelist of allowed identifier names, letting natural-language words through.
//!     Wired into the labelling step later; unit-tested here now.

use super::LlmError;
use super::steps::ClusterResult;
use std::collections::BTreeSet;

/// Verify + normalize a clustering result against the input card-id whitelist.
///
/// On success returns a `ClusterResult` where:
///  - every member/unclustered id is in `whitelist` (else `Err(Parse)` — hallucination),
///  - every whitelist id appears exactly once (omitted ids absorbed into `unclustered`,
///    duplicate placements de-duplicated keeping the first cluster that claimed the id).
///
/// Deterministic: absorbed ids are appended to `unclustered` in sorted order.
pub fn verify_clusters(
    mut result: ClusterResult,
    whitelist: &BTreeSet<String>,
) -> Result<ClusterResult, LlmError> {
    // 1. Hallucination reject: any returned id not in the whitelist is fatal.
    for c in &result.clusters {
        for id in &c.member_card_ids {
            if !whitelist.contains(id) {
                return Err(LlmError::Parse(format!(
                    "hallucinated card id in cluster {}: {id}",
                    c.cluster_id
                )));
            }
        }
    }
    for id in &result.unclustered {
        if !whitelist.contains(id) {
            return Err(LlmError::Parse(format!(
                "hallucinated card id in unclustered: {id}"
            )));
        }
    }

    // 2a. De-duplicate: a card id may only be claimed once. The first cluster (in input
    // order) that names an id keeps it; later duplicates are dropped. This keeps the
    // "exactly one placement" invariant without rejecting an otherwise-valid answer.
    let mut placed: BTreeSet<String> = BTreeSet::new();
    for c in &mut result.clusters {
        c.member_card_ids.retain(|id| placed.insert(id.clone()));
    }
    result.clusters.retain(|c| !c.member_card_ids.is_empty());

    // 2b. No-drop absorption: any whitelist id neither placed nor already unclustered is
    // appended to `unclustered` (sorted) so every change stays visible (§3.1).
    let mut unclustered_set: BTreeSet<String> = result.unclustered.iter().cloned().collect();
    // An id can't be both placed and unclustered; placed wins (it was clustered).
    unclustered_set.retain(|id| !placed.contains(id));
    for id in whitelist {
        if !placed.contains(id) {
            unclustered_set.insert(id.clone());
        }
    }
    result.unclustered = unclustered_set.into_iter().collect();

    Ok(result)
}

/// M4 reinforcement — extract code-identifier tokens from free text and keep only those
/// that are **not** in the allowed-name whitelist (i.e. the suspicious, possibly
/// hallucinated identifiers). Returns the offending tokens (empty ⇒ text is clean).
///
/// "Code-identifier token" = a run that looks like a symbol: contains a `.` (method
/// access like `OrderService.create`) or is CamelCase / snake_case-with-a-digit / ends
/// with `()`. Plain natural-language words (`order`, `creates`, `the`) are deliberately
/// let through — this is a *loose* check (planning §8.3 admits perfect coverage is
/// impossible), a second net behind the "don't assert" prompt rule.
///
/// `allowed` is the set of bare symbol names present in the input (lower-cased compare).
pub fn suspicious_identifiers(text: &str, allowed: &BTreeSet<String>) -> Vec<String> {
    let allowed_lc: BTreeSet<String> = allowed.iter().map(|s| s.to_ascii_lowercase()).collect();
    let mut out: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for tok in tokenize_identifiers(text) {
        if !looks_like_code_identifier(&tok) {
            continue; // natural-language word — let it through.
        }
        // Compare the *base* name (strip a trailing "()" and any "Owner." prefix) so
        // "OrderService.create()" is checked against both "OrderService.create",
        // "create", and "OrderService".
        let bases = identifier_bases(&tok);
        // The token is suspicious only if NONE of its bases is an allowed name.
        let any_allowed = bases.iter().any(|b| allowed_lc.contains(&b.to_ascii_lowercase()));
        if !any_allowed && seen.insert(tok.clone()) {
            out.push(tok);
        }
    }
    out
}

/// Split text into candidate identifier tokens. We keep `.`, `_`, and alphanumerics
/// together (so `OrderService.create_order` stays one token) and treat a trailing `()`
/// as part of the token. Everything else is a delimiter.
fn tokenize_identifiers(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_alphanumeric() || c == '_' || c == '.' {
            cur.push(c);
        } else if c == '(' && chars.peek() == Some(&')') {
            // Consume the "()" call marker as part of the current token.
            chars.next();
            cur.push_str("()");
            tokens.push(std::mem::take(&mut cur));
        } else {
            if !cur.is_empty() {
                tokens.push(std::mem::take(&mut cur));
            }
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
        .into_iter()
        .map(|t| t.trim_matches('.').to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Heuristic: does this token *look* like a code identifier (vs. a plain word)? True when
/// it contains a `.`, ends with `()`, is snake_case (contains `_`), or is **interior**
/// CamelCase (a case change *after the first character* — `createOrder`, `OrderService`,
/// `OrderId`). A single capitalized English word (`Creates`, `The`) has no interior case
/// change, so it is NOT flagged — sentence-leading capitals must pass through (§8.3 loose
/// check: only obviously code-shaped tokens are candidates).
fn looks_like_code_identifier(tok: &str) -> bool {
    if tok.contains('.') || tok.ends_with("()") || tok.contains('_') {
        return true;
    }
    let chars: Vec<char> = tok.chars().collect();
    // An interior case transition (lower→upper OR upper→lower at position ≥1) means the
    // token mixes case past its first letter: `createOrder` (l→U), `OrderId` (r→I),
    // `Money` alone (U then all lower) is NOT interior-mixed, `IOError` (O→r) is. A
    // sentence-leading `Creates` is upper then all-lower from index 1 ⇒ no interior
    // upper ⇒ not code.
    chars
        .iter()
        .enumerate()
        .skip(1)
        .any(|(i, &c)| c.is_uppercase() && chars[i - 1].is_lowercase())
}

/// The base names to check for a token: the whole token (minus a trailing `()`), the part
/// after the last `.`, and the part before the first `.` (the owner). Lets
/// "OrderService.create()" match an allowed "create" OR an allowed "OrderService".
fn identifier_bases(tok: &str) -> Vec<String> {
    let stripped = tok.trim_end_matches("()");
    let mut bases = vec![stripped.to_string()];
    if let Some((owner, member)) = stripped.split_once('.') {
        bases.push(owner.to_string());
        // member may itself be `a.b.c`; take the final segment.
        if let Some(last) = member.rsplit('.').next() {
            bases.push(last.to_string());
        }
    }
    bases
}

#[cfg(test)]
#[path = "verify_tests.rs"]
mod tests;
