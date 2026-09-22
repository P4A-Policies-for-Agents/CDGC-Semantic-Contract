// Copyright 2026 Salesforce, Inc. All rights reserved.
//! CDGC Semantic Contract — outbound Omni/Flex Gateway policy.
//!
//! Attaches **machine-evaluated semantic guidance to tool results**, resolved
//! live from Informatica CDGC, so an AI agent reading an enterprise API response
//! cannot confidently act on a meaning that is wrong. It completes the triad the
//! MCP protocol half-defines: `inputSchema` (how to call), `outputSchema` (what
//! the payload means in the abstract) and — attached here — the **semantic
//! contract**: what is true about *this* payload's fields right now per the
//! enterprise catalog, which is owned by data governance and versioned on a
//! different clock than the API.
//!
//! This is the response-leg, **fail-open** sibling of CDGC Purpose Binding.
//! Purpose Binding folds the shared CDGC resolution core (Login → JWT →
//! ccgf-searchv2, cached, single-flight) into a request-leg fail-closed
//! *decision*; Semantic Contract folds the same core into response-leg
//! *annotation*. There is **no model in the data path** and the upstream payload
//! is **never rewritten** — only annotated, additively.
//!
//! On the REQUEST leg it captures the routing key (MCP `tools/call` tool name, or
//! REST path), resolves the CDGC schema this call binds to (toolSchemas /
//! pathSchemas → schemaIdClaim → schemaIdHeader → default schemaId), and threads
//! that to the response leg. On the RESPONSE leg, for a successful `tools/call`
//! result (or a REST JSON object), it resolves the schema's field → Business Term
//! → classification/meaning map from CDGC, builds the guidance for the governed
//! fields that actually appear in the payload, and attaches it to both
//! `structuredContent._semanticContract` and a delimited block in `content[]`
//! (clients disagree on which is canonical).
//!
//! It **stays quiet**: error results are never annotated, `structuredContent` is
//! never created where the upstream returned none, a document with no governed
//! field present is left byte-identical, and any runtime failure (CDGC error,
//! oversized body, parse failure) passes the response through unchanged.

mod claims;
mod generated;
mod semantic;

use std::rc::Rc;
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, Result};
use pdk::data_storage::{DataStorage, DataStorageBuilder, DataStorageError, StoreMode};
use pdk::hl::timer::Clock;
use pdk::hl::*;
use pdk::logger;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::generated::config::Config;
use crate::semantic::{
    build_contract, build_markers, build_obligations, collect_present_keys, nonce_from_time,
    render_block, resolve_classification, resolve_mapped_schema, resolve_path_schema, CachedClassMap,
    ContractField, GovernedField, MarkerRules, Obligations, RefreshLock,
};

const CLASS_CACHE_NAMESPACE: &str = "csc-classmap";
const REFRESH_LOCK_NAMESPACE: &str = "csc-refresh-lock";
const CLASS_CACHE_KEY_PREFIX: &str = "csc-classmap-";
const REFRESH_LOCK_KEY_PREFIX: &str = "csc-lock-";
const REFRESH_LOCK_TTL_SECONDS: i64 = 30;
const REFRESH_LOCK_TTL_MS: u32 = (REFRESH_LOCK_TTL_SECONDS as u32) * 1000;
const CLASS_STORE_MIN_TTL_MS: u64 = 30 * 24 * 60 * 60 * 1000;
const CAS_MAX_RETRIES: u32 = 3;
const DEFAULT_TIMEOUT_MS: i64 = 5_000;
// Six chained CDGC calls (login, jwt, file, columns, term-links, terms).
const CDGC_REFRESH_BUDGET_MS: i64 = 15_000;
const DEFAULT_REFRESH_INTERVAL_SECONDS: i64 = 86_400;
const SEARCH_PATH: &str = "/ccgf-searchv2/api/v1/search";
const CT_FLATFIELD: &str = "com.infa.odin.models.file.flat.FlatField";
const REL_TECH_GLOSSARY: &str = "com.infa.ccgf.models.governance.IClassTechnicalGlossaryBase";
// The structured IDMC "Security Level" classification on a Business Term.
const ATTR_SECURITY_CLASS: &str = "com.infa.ccgf.models.governance.securityClassification";

const DEFAULT_SCHEMA_HEADER: &str = "x-dp-schema-id";

const DEFAULT_MAX_CONTRACT_ENTRIES: i64 = 24;
const DEFAULT_MAX_MEANING_CHARS: i64 = 400;
const DEFAULT_MAX_ANNOTATE_BYTES: i64 = 262_144; // 256 KiB — buffer + rewrite ceiling.

// Description-keyword markers that infer a classification when a term carries no
// structured Security Level (this tenant encodes sensitivity in descriptions).
const DEFAULT_RESTRICTED_MARKERS: &[&str] = &["personal data", "pii", "restricted", "ssn", "national id", "sensitive personal"];
const DEFAULT_CONFIDENTIAL_MARKERS: &[&str] = &["confidential", "consent", "proprietary"];
const DEFAULT_INTERNAL_MARKERS: &[&str] = &["internal use", "internal only"];
const DEFAULT_PUBLIC_MARKERS: &[&str] = &[];

// Default handling obligations per IDMC Security Level (open free-text; override in config).
const DEFAULT_RESTRICTED_OBLIGATION: &str =
    "Restricted / PII. Do not disclose externally; use only for the stated purpose; minimise retention.";
const DEFAULT_CONFIDENTIAL_OBLIGATION: &str =
    "Confidential. Need-to-know internal use; do not disclose to customers or third parties.";
const DEFAULT_INTERNAL_OBLIGATION: &str = "Internal use only. Not for external distribution.";
const DEFAULT_PUBLIC_OBLIGATION: &str = ""; // Public: nothing to state.

const CONTRACT_VERSION: &str = "1.0";
const CONTRACT_SOURCE: &str = "informatica-cdgc";

#[derive(Deserialize)]
struct CdgcLoginResponse {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "orgId")]
    org_id: String,
}
#[derive(Deserialize)]
struct CdgcJwtResponse {
    jwt_token: String,
}

/// Threaded from the request leg to the response leg: whether this response is a
/// candidate for annotation, whether it is JSON-RPC (MCP/A2A) vs REST, and the
/// CDGC schema resolved for the call.
#[derive(Clone, Debug, Default)]
struct Ctx {
    annotate: bool,
    is_rpc: bool,
    schema_id: Option<String>,
    tool: Option<String>,
}

fn now_secs(clock: &Clock) -> i64 {
    clock.now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
fn elapsed_ms(start: SystemTime, now: SystemTime) -> i64 {
    now.duration_since(start).map(|d| d.as_millis() as i64).unwrap_or(0)
}
fn next_call_timeout(per_call_ms: i64, elapsed: i64) -> Option<Duration> {
    let remaining = CDGC_REFRESH_BUDGET_MS - elapsed;
    if remaining <= 0 {
        return None;
    }
    Some(Duration::from_millis(per_call_ms.min(remaining).max(1) as u64))
}
fn class_store_ttl_ms(config: &Config) -> u32 {
    let refresh = config.refresh_interval_seconds.unwrap_or(DEFAULT_REFRESH_INTERVAL_SECONDS).max(0) as u64;
    refresh.saturating_mul(2).saturating_mul(1000).max(CLASS_STORE_MIN_TTL_MS).min(u32::MAX as u64) as u32
}
fn marker_rules(config: &Config) -> MarkerRules {
    build_markers(
        config.restricted_markers.as_deref(),
        config.confidential_markers.as_deref(),
        config.internal_markers.as_deref(),
        config.public_markers.as_deref(),
        DEFAULT_RESTRICTED_MARKERS,
        DEFAULT_CONFIDENTIAL_MARKERS,
        DEFAULT_INTERNAL_MARKERS,
        DEFAULT_PUBLIC_MARKERS,
    )
}
fn obligations(config: &Config) -> Obligations {
    build_obligations(
        config.restricted_obligation.as_deref(),
        config.confidential_obligation.as_deref(),
        config.internal_obligation.as_deref(),
        config.public_obligation.as_deref(),
        DEFAULT_RESTRICTED_OBLIGATION,
        DEFAULT_CONFIDENTIAL_OBLIGATION,
        DEFAULT_INTERNAL_OBLIGATION,
        DEFAULT_PUBLIC_OBLIGATION,
    )
}
/// Per-call schema routing: pick the CDGC schemaId this call binds to (toolSchemas
/// for MCP tool name, pathSchemas for REST path). Returns `None` when no mapping
/// matches (the caller then falls back to a claim/header/default).
fn mapped_schema(config: &Config, tool: Option<&str>, path: Option<&str>) -> Option<String> {
    if let (Some(t), Some(entries)) = (tool, config.tool_schemas.as_deref()) {
        if let Some(s) = resolve_mapped_schema(entries, t) {
            return Some(s);
        }
    }
    if let (Some(p), Some(entries)) = (path, config.path_schemas.as_deref()) {
        if let Some(s) = resolve_path_schema(entries, p) {
            return Some(s);
        }
    }
    None
}

// ─── CDGC resolution core (shared shape with the sibling CDGC Purpose Binding) ──

/// Authenticate to IDMC (Login → JWT). Returns (jwt, orgId).
async fn cdgc_auth(client: &HttpClient, config: &Config, clock: &Clock, start: SystemTime) -> Result<(String, String)> {
    let per_call = config.timeout.unwrap_or(DEFAULT_TIMEOUT_MS);
    let login_body = serde_json::to_vec(&json!({
        "username": config.cdgc_org_username, "password": config.cdgc_org_password,
    }))?;
    let t = next_call_timeout(per_call, elapsed_ms(start, clock.now())).ok_or_else(|| anyhow!("budget before Login"))?;
    let login_resp = client.request(&config.cdgc_login_url).path("/identity-service/api/v1/Login")
        .headers(vec![("Content-Type", "application/json")]).body(&login_body).timeout(t).post().await
        .map_err(|e| anyhow!("CDGC login failed: {e}"))?;
    if login_resp.status_code() >= 300 {
        return Err(anyhow!("CDGC login status {}", login_resp.status_code()));
    }
    let login: CdgcLoginResponse = serde_json::from_slice(login_resp.body()).map_err(|e| anyhow!("parse login: {e}"))?;
    let nonce = nonce_from_time(clock.now());
    let cookie = format!("USER_SESSION={}", login.session_id);
    let t = next_call_timeout(per_call, elapsed_ms(start, clock.now())).ok_or_else(|| anyhow!("budget before JWT"))?;
    let jwt_resp = client.request(&config.cdgc_login_url)
        .path(&format!("/identity-service/api/v1/jwt/Token?client_id=idmc_api&nonce={nonce}"))
        .headers(vec![("cookie", cookie.as_str()), ("IDS-SESSION-ID", login.session_id.as_str())])
        .timeout(t).get().await.map_err(|e| anyhow!("CDGC JWT failed: {e}"))?;
    if jwt_resp.status_code() >= 300 {
        return Err(anyhow!("CDGC JWT status {}", jwt_resp.status_code()));
    }
    let jwt: CdgcJwtResponse = serde_json::from_slice(jwt_resp.body()).map_err(|e| anyhow!("parse jwt: {e}"))?;
    Ok((jwt.jwt_token, login.org_id))
}

/// One ccgf-searchv2 Elasticsearch query. Returns the `hits.hits[]` `sourceAsMap`s.
async fn cdgc_search(
    client: &HttpClient, config: &Config, clock: &Clock, start: SystemTime,
    jwt: &str, org: &str, body: &Value,
) -> Result<Vec<Value>> {
    let per_call = config.timeout.unwrap_or(DEFAULT_TIMEOUT_MS);
    let authz = format!("Bearer {jwt}");
    let payload = serde_json::to_vec(body)?;
    let t = next_call_timeout(per_call, elapsed_ms(start, clock.now())).ok_or_else(|| anyhow!("budget before search"))?;
    let resp = client.request(&config.cdgc_search_url).path(SEARCH_PATH)
        .headers(vec![
            ("Authorization", authz.as_str()),
            ("X-INFA-ORG-ID", org),
            ("X-INFA-SEARCH-LANGUAGE", "elasticsearch"),
            ("Content-Type", "application/json"),
        ])
        .body(&payload).timeout(t).post().await
        .map_err(|e| anyhow!("CDGC search failed: {e}"))?;
    if resp.status_code() >= 300 {
        return Err(anyhow!("CDGC search status {}", resp.status_code()));
    }
    let v: Value = serde_json::from_slice(resp.body()).map_err(|e| anyhow!("parse search: {e}"))?;
    Ok(v.get("hits").and_then(|h| h.get("hits")).and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|h| h.get("sourceAsMap").cloned()).collect())
        .unwrap_or_default())
}

fn s(map: &Value, key: &str) -> Option<String> {
    map.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Catalog-driven semantic map: Login → JWT, then via ccgf-searchv2 resolve the
/// schema asset, enumerate its columns and their linked Business Terms, and build
/// the per-field map (name + Security Level classification + governing term name +
/// term meaning/description). Identical resolution chain to CDGC Purpose Binding,
/// but it also keeps the term *description* (the semantic content this policy
/// attaches), which the sibling only uses as a marker-classification fallback.
async fn fetch_class_map(
    client: &HttpClient,
    config: &Config,
    clock: &Clock,
    markers: &MarkerRules,
    schema_id: &str,
) -> Result<(Vec<GovernedField>, Option<String>, Option<String>)> {
    let start = clock.now();
    let (jwt, org) = cdgc_auth(client, config, clock, start).await?;

    // 1. Resolve the schema asset → location + identity.
    let files = cdgc_search(client, config, clock, start, &jwt, &org, &json!({
        "from":0,"size":1,"query":{"bool":{"must":[
            {"terms":{"elementType":["OBJECT"]}},
            {"terms":{"core.identity":[schema_id]}}]}}
    })).await?;
    let file = files.into_iter().next().ok_or_else(|| anyhow!("schema asset '{schema_id}' not found"))?;
    let location = s(&file, "core.location").ok_or_else(|| anyhow!("schema asset has no core.location"))?;
    let file_name = s(&file, "core.name");
    let external_id = s(&file, "core.externalId");

    // 2. Enumerate columns (children of the schema location).
    let cols = cdgc_search(client, config, clock, start, &jwt, &org, &json!({
        "from":0,"size":1000,"query":{"bool":{
            "must":[{"terms":{"core.classType":[CT_FLATFIELD]}}],
            "filter":[{"terms":{"core.location::path_hierarchy.parent":[location]}}]}}
    })).await?;
    if cols.is_empty() {
        return Err(anyhow!("no columns found for schema asset '{schema_id}'"));
    }
    let col_ids: Vec<String> = cols.iter().filter_map(|c| s(c, "core.identity")).collect();

    // 3. Column → Business Term links.
    let rels = cdgc_search(client, config, clock, start, &jwt, &org, &json!({
        "from":0,"size":5000,"query":{"bool":{"must":[
            {"terms":{"elementType":["RELATIONSHIP"]}},
            {"terms":{"type":[REL_TECH_GLOSSARY]}},
            {"terms":{"core.sourceIdentity":col_ids}}]}}
    })).await?;
    let mut col_to_term: Map<String, Value> = Map::new();
    let mut term_ids: Vec<String> = Vec::new();
    for r in &rels {
        if let (Some(src), Some(tgt)) = (s(r, "core.sourceIdentity"), s(r, "core.targetIdentity")) {
            col_to_term.entry(src).or_insert(Value::String(tgt.clone()));
            if !term_ids.contains(&tgt) {
                term_ids.push(tgt);
            }
        }
    }

    // 4. Resolve the linked terms → name + structured Security Level + description.
    let mut terms: Map<String, Value> = Map::new(); // termId → {name, level, description}
    if !term_ids.is_empty() {
        let tdocs = cdgc_search(client, config, clock, start, &jwt, &org, &json!({
            "from":0,"size":5000,"query":{"bool":{"must":[
                {"terms":{"elementType":["OBJECT"]}},
                {"terms":{"core.identity":term_ids}}]}}
        })).await?;
        for t in &tdocs {
            if let Some(id) = s(t, "core.identity") {
                terms.insert(id, json!({
                    "name": s(t, "core.name"),
                    "level": s(t, ATTR_SECURITY_CLASS).unwrap_or_default(),
                    "description": s(t, "core.description").unwrap_or_default(),
                }));
            }
        }
    }

    // 5. Build the field map: name + classification + governing term + meaning.
    let mut fields = Vec::new();
    for c in &cols {
        let (Some(id), Some(name)) = (s(c, "core.identity"), s(c, "core.name")) else { continue };
        let mut classification: Option<String> = None;
        let mut term_name: Option<String> = None;
        let mut meaning: Option<String> = None;
        if let Some(Value::String(tid)) = col_to_term.get(&id) {
            if let Some(term) = terms.get(tid) {
                term_name = term.get("name").and_then(Value::as_str).map(str::to_string);
                let level = term.get("level").and_then(Value::as_str).unwrap_or("");
                let description = term.get("description").and_then(Value::as_str).unwrap_or("");
                classification = resolve_classification(level, description, markers);
                let d = description.trim();
                if !d.is_empty() {
                    meaning = Some(d.to_string());
                }
            }
        }
        fields.push(GovernedField { name, classification, term: term_name, meaning });
    }
    Ok((fields, file_name, external_id))
}

// ─── DataStorage cache (lazy refresh, single-flight, CAS) ───────────────────

async fn read_cached<S: DataStorage>(store: &S, key: &str) -> Option<CachedClassMap> {
    match store.get::<CachedClassMap>(key).await {
        Ok(Some((c, _))) => Some(c),
        Ok(None) => None,
        Err(e) => {
            logger::warn!("csc: cache read failed: {e}");
            None
        }
    }
}
async fn write_cached<S: DataStorage>(store: &S, key: &str, entry: &CachedClassMap) {
    for _ in 0..CAS_MAX_RETRIES {
        match store.get::<CachedClassMap>(key).await {
            Ok(Some((_, v))) => match store.store(key, &StoreMode::Cas(v), entry).await {
                Ok(()) => return,
                Err(DataStorageError::CasMismatch) => continue,
                Err(e) => { logger::warn!("csc: persist failed: {e}"); return; }
            },
            Ok(None) => match store.store(key, &StoreMode::Absent, entry).await {
                Ok(()) => return,
                Err(DataStorageError::CasMismatch) => continue,
                Err(e) => { logger::warn!("csc: persist failed: {e}"); return; }
            },
            Err(e) => { logger::warn!("csc: read-before-persist failed: {e}"); return; }
        }
    }
}
async fn try_acquire_refresh_lock<S: DataStorage>(store: &S, key: &str, now: i64) -> Result<bool, DataStorageError> {
    let entry = RefreshLock { acquired_at: now };
    match store.store(key, &StoreMode::Absent, &entry).await {
        Ok(()) => Ok(true),
        Err(DataStorageError::CasMismatch) => match store.get::<RefreshLock>(key).await? {
            Some((existing, v)) => {
                if now - existing.acquired_at < REFRESH_LOCK_TTL_SECONDS { Ok(false) }
                else {
                    match store.store(key, &StoreMode::Cas(v), &entry).await {
                        Ok(()) => Ok(true),
                        Err(DataStorageError::CasMismatch) => Ok(false),
                        Err(e) => Err(e),
                    }
                }
            }
            None => match store.store(key, &StoreMode::Absent, &entry).await {
                Ok(()) => Ok(true),
                Err(DataStorageError::CasMismatch) => Ok(false),
                Err(e) => Err(e),
            },
        },
        Err(e) => Err(e),
    }
}

/// Get the (cached, lazily refreshed) semantic map for a schema. `None` = could
/// not resolve it authoritatively (CDGC error, no cached map) — the annotator then
/// passes the response through unchanged (fail-open).
async fn get_class_map<S: DataStorage>(
    client: &HttpClient, config: &Config, clock: &Clock, markers: &MarkerRules,
    map_store: &S, lock_store: &S, schema_id: &str,
) -> Option<CachedClassMap> {
    let key = format!("{CLASS_CACHE_KEY_PREFIX}{schema_id}");
    let ttl = config.refresh_interval_seconds.unwrap_or(DEFAULT_REFRESH_INTERVAL_SECONDS).max(0);
    let now = now_secs(clock);
    let cached = read_cached(map_store, &key).await;
    if let Some(c) = &cached {
        if now - c.timestamp < ttl {
            return cached;
        }
    }
    let lock_key = format!("{REFRESH_LOCK_KEY_PREFIX}{schema_id}");
    if !try_acquire_refresh_lock(lock_store, &lock_key, now).await.unwrap_or(true) {
        return cached; // someone else is refreshing; serve stale if any.
    }
    match fetch_class_map(client, config, clock, markers, schema_id).await {
        Ok((fields, name, external_id)) => {
            let entry = CachedClassMap { fields, name, external_id, timestamp: now };
            write_cached(map_store, &key, &entry).await;
            Some(entry)
        }
        Err(e) => {
            logger::warn!("csc: class-map refresh failed for '{schema_id}': {e}");
            cached
        }
    }
}

// ─── contract shaping + attachment ──────────────────────────────────────────

fn contract_json(entries: &[ContractField], schema_label: &str, generated_at: i64) -> Value {
    json!({
        "version": CONTRACT_VERSION,
        "source": CONTRACT_SOURCE,
        "schemaId": schema_label,
        "generatedAt": generated_at,
        "note": "Gateway-attached semantic guidance resolved live from Informatica CDGC. Not part of the tool payload; no model in the data path.",
        "fields": entries,
    })
}

/// Collect the (normalised) field names present in an MCP `tools/call` result:
/// the keys of `structuredContent`, plus the keys inside any `content[].text`
/// that parses as JSON. Used to keep the annotation to fields that appear.
fn present_keys_in_result(result: &Value) -> std::collections::BTreeSet<String> {
    let mut keys = std::collections::BTreeSet::new();
    if let Some(sc) = result.get("structuredContent") {
        keys.extend(collect_present_keys(sc));
    }
    if let Some(arr) = result.get("content").and_then(Value::as_array) {
        for c in arr {
            if let Some(t) = c.get("text").and_then(Value::as_str) {
                if let Ok(v) = serde_json::from_str::<Value>(t) {
                    keys.extend(collect_present_keys(&v));
                }
            }
        }
    }
    keys
}

/// Attach guidance to an MCP `tools/call` result. Sets
/// `structuredContent._semanticContract` **only if `structuredContent` exists**
/// (never fabricates it) and appends the delimited block to `content[]` when it
/// is an array. Returns whether anything was attached.
fn attach_to_mcp_result(
    result: &mut Value,
    entries: &[ContractField],
    block: &str,
    schema_label: &str,
    generated_at: i64,
) -> bool {
    let mut changed = false;
    if let Some(sc) = result.get_mut("structuredContent").and_then(Value::as_object_mut) {
        sc.insert("_semanticContract".to_string(), contract_json(entries, schema_label, generated_at));
        changed = true;
    }
    if let Some(arr) = result.get_mut("content").and_then(Value::as_array_mut) {
        arr.push(json!({ "type": "text", "text": block }));
        changed = true;
    }
    changed
}

// ─── SSE helpers (single-shot tools/call response) ──────────────────────────

/// Extract the first JSON-RPC message from an SSE (`text/event-stream`) body. MCP
/// `tools/call` responses are single-shot — one `data:` event then close — so a
/// bounded buffer + first-event parse is sufficient (we never buffer a long-lived
/// stream; the response filter guards total size with `maxAnnotateBytes`).
fn sse_first_json(text: &str) -> Option<Value> {
    let mut data_lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() {
            if !data_lines.is_empty() {
                if let Ok(v) = serde_json::from_str::<Value>(&data_lines.join("\n")) {
                    return Some(v);
                }
                data_lines.clear();
            }
            continue;
        }
        if let Some(payload) = line.strip_prefix("data:") {
            data_lines.push(payload.strip_prefix(' ').unwrap_or(payload).to_string());
        }
    }
    if !data_lines.is_empty() {
        if let Ok(v) = serde_json::from_str::<Value>(&data_lines.join("\n")) {
            return Some(v);
        }
    }
    None
}

/// Re-wrap a single JSON-RPC message as one SSE event (MCP clients read the
/// `data:` payload; `event: message` is the standard MCP frame type).
fn wrap_sse(json_str: &str) -> String {
    format!("event: message\ndata: {json_str}\n\n")
}

// ─── request leg: capture routing + resolve schema, thread to response ──────

async fn request_filter(request_state: RequestState, config: Rc<Config>) -> Flow<Ctx> {
    let headers = request_state.into_headers_state().await;
    let has_body = headers.contains_body();

    let schema_header = config.schema_id_header.as_deref().unwrap_or(DEFAULT_SCHEMA_HEADER).to_ascii_lowercase();
    // Read every header we need up front (the handler borrow ends before we
    // consume `headers` to enter body state).
    let (ct, authz, path, schema_hdr_val) = {
        let h = headers.handler();
        (
            h.header("content-type").unwrap_or_default(),
            h.header("authorization"),
            h.header(":path"),
            h.header(&schema_header),
        )
    };

    // Opt-in JWT-claims source for the schema id (verification is an upstream JWT
    // Validation policy's job); a configured claim wins over the header.
    let jwt_claims = if config.schema_id_claim.is_some() {
        claims::decode_bearer_claims(authz.as_deref())
    } else {
        None
    };
    let from_claim = |name: &Option<String>| -> Option<String> {
        claims::claim_str(jwt_claims.as_ref()?, name.as_deref()?)
    };

    // JSON-RPC (MCP/A2A) detection + tool name (for tools/call). REST has no envelope.
    let mut is_rpc = false;
    let mut tool: Option<String> = None;
    if ct.starts_with("application/json") && has_body {
        let body_state = headers.into_body_state().await;
        if let Ok(v) = serde_json::from_slice::<Value>(&body_state.handler().body()) {
            if let Some(m) = v.get("method").and_then(Value::as_str) {
                is_rpc = true;
                if m == "tools/call" {
                    tool = v.pointer("/params/name").and_then(Value::as_str).map(str::to_string);
                }
            }
        }
    }

    // Route by tool (MCP) or path (REST): mapping wins, else JWT claim, else header, else default.
    let req_path = if is_rpc { None } else { path.filter(|v| !v.trim().is_empty()) };
    let schema_id = mapped_schema(&config, tool.as_deref(), req_path.as_deref())
        .or_else(|| from_claim(&config.schema_id_claim))
        .or_else(|| schema_hdr_val.filter(|v| !v.trim().is_empty()))
        .or_else(|| config.schema_id.clone());

    // Annotate only a resolvable call we can bind: an MCP tools/call, or any REST
    // request (its response is guarded on the response leg by status + content-type).
    let annotate = schema_id.is_some() && (if is_rpc { tool.is_some() } else { true });

    Flow::Continue(Ctx { annotate, is_rpc, schema_id, tool })
}

// ─── response leg: annotate a successful result, else pass through ──────────

async fn response_filter<S: DataStorage>(
    response_state: ResponseState,
    request_data: RequestData<Ctx>,
    config: Rc<Config>,
    client: Rc<HttpClient>,
    clock: Rc<Clock>,
    map_store: Rc<S>,
    lock_store: Rc<S>,
) {
    let RequestData::Continue(ctx) = request_data else { return };
    let (Some(schema_id), true) = (ctx.schema_id.clone(), ctx.annotate) else { return };

    let headers = response_state.into_headers_state().await;
    let status = headers.status_code();
    if !(200..300).contains(&status) || !headers.contains_body() {
        return; // errors / bodiless responses are never annotated
    }
    let ct = headers.handler().header("content-type").unwrap_or_default().to_ascii_lowercase();
    let is_sse = ct.contains("text/event-stream");
    let is_json = ct.contains("json");
    if !is_sse && !is_json {
        return; // unknown shape — pass through
    }

    // We are about to (possibly) rewrite the body, so drop content-length now —
    // headers are frozen once we enter body state.
    headers.handler().remove_header("content-length");
    let body_state = headers.into_body_state().await;
    let orig = body_state.handler().body();

    let max_bytes = config.max_annotate_bytes.unwrap_or(DEFAULT_MAX_ANNOTATE_BYTES).max(0) as usize;
    // Oversized (or a stream we shouldn't buffer): pass through byte-identical.
    if orig.len() > max_bytes {
        let _ = body_state.handler().set_body(&orig);
        logger::debug!("csc: passthrough (body {} > maxAnnotateBytes {max_bytes}) schema={schema_id}", orig.len());
        return;
    }

    // Parse the response into a JSON value we can inspect.
    let text = match std::str::from_utf8(&orig) {
        Ok(t) => t,
        Err(_) => { let _ = body_state.handler().set_body(&orig); return; }
    };
    let parsed: Option<Value> = if is_sse { sse_first_json(text) } else { serde_json::from_str(text).ok() };
    let Some(mut value) = parsed else {
        let _ = body_state.handler().set_body(&orig);
        return;
    };

    let markers = marker_rules(&config);
    let obligations = obligations(&config);
    let present_only = config.annotate_fields_present_only.unwrap_or(true);
    let max_entries = config.max_contract_entries.unwrap_or(DEFAULT_MAX_CONTRACT_ENTRIES).max(0) as usize;
    let meaning_max = config.max_meaning_chars.unwrap_or(DEFAULT_MAX_MEANING_CHARS).max(16) as usize;
    let generated_at = now_secs(&clock);

    // Locate the object to annotate and the fields present in it.
    // MCP: the JSON-RPC `result` (skip errors / isError). REST: the top-level object.
    let (present, is_mcp_result) = if ctx.is_rpc {
        let Some(result) = value.get("result") else {
            let _ = body_state.handler().set_body(&orig); return; // error / notification — quiet
        };
        if value.get("error").is_some() || result.get("isError").and_then(Value::as_bool).unwrap_or(false) {
            let _ = body_state.handler().set_body(&orig); return;
        }
        (present_keys_in_result(result), true)
    } else {
        if !value.is_object() {
            let _ = body_state.handler().set_body(&orig); return;
        }
        (collect_present_keys(&value), false)
    };

    // Resolve the schema's semantic map (cached) and build the guidance. Any CDGC
    // failure yields None → pass through unchanged (fail-open).
    let cc = match get_class_map(&client, &config, &clock, &markers, &*map_store, &*lock_store, &schema_id).await {
        Some(cc) => cc,
        None => { let _ = body_state.handler().set_body(&orig); return; }
    };
    let present_ref = if present_only { Some(&present) } else { None };
    let entries = build_contract(&cc.fields, present_ref, &obligations, max_entries, meaning_max);
    if entries.is_empty() {
        // Nothing governed appears in this payload — leave it byte-identical.
        let _ = body_state.handler().set_body(&orig);
        logger::debug!("csc: quiet (no governed field present) schema={schema_id}");
        return;
    }

    let block = render_block(&entries, &schema_id);
    let attached = if is_mcp_result {
        // Safe: `result` is present (checked above).
        attach_to_mcp_result(value.get_mut("result").unwrap(), &entries, &block, &schema_id, generated_at)
    } else if let Some(obj) = value.as_object_mut() {
        obj.insert("_semanticContract".to_string(), contract_json(&entries, &schema_id, generated_at));
        true
    } else {
        false
    };

    if !attached {
        let _ = body_state.handler().set_body(&orig);
        logger::debug!("csc: quiet (no attach target in result) schema={schema_id}");
        return;
    }

    let new_json = value.to_string();
    let new_body = if is_sse { wrap_sse(&new_json) } else { new_json };
    match body_state.handler().set_body(new_body.as_bytes()) {
        Ok(()) => {
            logger::info!(
                "csc: ANNOTATED schema={schema_id} tool={} fields={} rpc={}",
                ctx.tool.as_deref().unwrap_or("-"), entries.len(), ctx.is_rpc
            );
        }
        Err(e) => {
            // Could not write the rewritten body — restore the original (fail-open).
            logger::warn!("csc: set_body failed ({e}) — passing through unchanged schema={schema_id}");
            let _ = body_state.handler().set_body(&orig);
        }
    }
}

fn launch_policy<S: DataStorage + 'static>(
    launcher: Launcher, config: Rc<Config>, client: Rc<HttpClient>, clock: Rc<Clock>,
    map_store: Rc<S>, lock_store: Rc<S>,
) -> impl std::future::Future<Output = Result<()>> {
    let cfg_req = config.clone();
    let filter = on_request(move |rs| {
        let c = cfg_req.clone();
        async move { request_filter(rs, c).await }
    })
    .on_response(move |rs, rd| {
        let c = config.clone();
        let cl = client.clone();
        let ck = clock.clone();
        let ms = map_store.clone();
        let ls = lock_store.clone();
        async move { response_filter(rs, rd, c, cl, ck, ms, ls).await }
    });
    async move { launcher.launch(filter).await.map_err(Into::into) }
}

#[entrypoint]
async fn configure(
    launcher: Launcher,
    Configuration(bytes): Configuration,
    client: HttpClient,
    storage_builder: DataStorageBuilder,
    clock: Clock,
) -> Result<()> {
    let config: Config = serde_json::from_slice(&bytes)
        .map_err(|err| anyhow!("Failed to parse configuration '{}'. Cause: {}", String::from_utf8_lossy(&bytes), err))?;
    let config = Rc::new(config);
    let client = Rc::new(client);
    let clock = Rc::new(clock);
    if config.distributed.unwrap_or(false) {
        let ms = Rc::new(storage_builder.remote(CLASS_CACHE_NAMESPACE, class_store_ttl_ms(&config)));
        let ls = Rc::new(storage_builder.remote(REFRESH_LOCK_NAMESPACE, REFRESH_LOCK_TTL_MS));
        launch_policy(launcher, config, client, clock, ms, ls).await
    } else {
        let ms = Rc::new(storage_builder.local(CLASS_CACHE_NAMESPACE));
        let ls = Rc::new(storage_builder.local(REFRESH_LOCK_NAMESPACE));
        launch_policy(launcher, config, client, clock, ms, ls).await
    }
}
