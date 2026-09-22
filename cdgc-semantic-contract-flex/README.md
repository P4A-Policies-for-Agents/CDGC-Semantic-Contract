# cdgc-semantic-contract (flex implementation)

Rust → `wasm32-wasip1` implementation of the **CDGC Semantic Contract** Omni/Flex Gateway
policy. Published as the implementation asset `cdgc-semantic-contract-flex`, paired with the
definition asset `cdgc-semantic-contract`.

A response-leg, **fail-open** annotator: it attaches CDGC-resolved semantic guidance
(governing Business Term, its catalog meaning, its IDMC Security Level, the handling
obligation) to the governed fields present in a successful tool result, and passes everything
else through byte-identical. It is the sibling of **CDGC Purpose Binding** — same CDGC
resolution core, but folded into response-leg *annotation* instead of request-leg *decision*.

## Layout

| File | Role |
|---|---|
| `src/lib.rs` | The annotator. Request-leg filter captures the routing key (MCP `tools/call` tool name / REST path) and resolves the CDGC schema, threading a `Ctx` to the response leg via `Flow::Continue`. Response-leg filter buffers a successful JSON/SSE body, resolves the schema's field→term map from CDGC (cached), builds the contract for the fields present, attaches it, and rewrites the body — fail-open on any error. Includes the CDGC Login→JWT→ccgf-searchv2 core and the DataStorage cache (lazy refresh, single-flight, CAS). |
| `src/semantic.rs` | Pure logic (no PDK): resolve a field's classification (structured Security Level, else description-keyword markers), route a call to its bound schema (`resolve_mapped_schema`/`resolve_path_schema`), collect the keys present in a payload, build the per-field contract (meaning clipped, most-restrictive first, capped), and render the delimited `content[]` block (with trust-delimiter escaping). Fully unit-tested (15 tests). |
| `src/claims.rs` | Opt-in Bearer-JWT decode (payload only; verification is an upstream JWT Validation policy's job). Shared shape with the sibling policies. |
| `src/generated/config.rs` | Config struct matching `gcl.yaml`. Hand-maintained here; `config-gen` regenerates it at release. |
| `playground/` | Local Flex Gateway playground (docker-compose) for manual runs. |

## Why NO `enable_stop_iteration`

Unlike the sibling gate, this policy **never `Break`s** the flow — it only additively rewrites
a successful response body. So it does not need the `enable_stop_iteration` feature (which
exists to make a request-leg `Flow::Break` win the upstream race). It buffers a *single-shot*
`tools/call` response (bounded by `maxAnnotateBytes`) and rewrites it via the GA
`into_body_state()` / `set_body` path — no experimental streaming write.

## Request-leg flow (`request_filter`)

1. Read the headers we need (`content-type`, `authorization`, `:path`, the configured schema
   header) up front, then optionally buffer the request body.
2. Opt-in JWT-claims source: decode the Bearer token only when `schemaIdClaim` is configured.
3. Detect JSON-RPC (MCP/A2A) vs REST from `content-type` + body shape, and capture the
   `tools/call` `params.name` (MCP) or the request path (REST).
4. **Per-call schema routing:** a `toolSchemas` (tool → schemaId) / `pathSchemas` (path prefix
   → schemaId) mapping wins, else a `schemaIdClaim` JWT claim, else the `schemaIdHeader`, else
   the optional default `schemaId`. Thread `Ctx { annotate, is_rpc, schema_id, tool }` to the
   response via `Flow::Continue`. Only an MCP `tools/call` (with a resolved schema) or a REST
   request is a candidate; the handshake is never annotated.

## Response-leg flow (`response_filter`)

1. Only on `RequestData::Continue(ctx)` with `annotate` + a resolved `schema_id`; require a 2xx
   response with a body and an `application/json` or `text/event-stream` content-type.
2. Remove `content-length` (headers freeze once in body state), buffer the body. Over
   `maxAnnotateBytes` ⇒ pass through byte-identical.
3. Parse JSON directly, or the first JSON-RPC message from a single-shot SSE frame.
4. **MCP:** require a `result` object with no `error` / `isError` (else passthrough). **REST:**
   require a top-level JSON object. Collect the governed field names present
   (`structuredContent` keys + any JSON inside `content[].text`).
5. `get_class_map` — fetch (or serve cached) the schema's field → term → meaning/classification
   map via the 5-step CDGC ccgf-searchv2 chain in `fetch_class_map`: Login → JWT → resolve
   schema asset → enumerate columns → resolve column→Business-Term links → resolve terms'
   Security Level + **description (the meaning)** → build the per-field map. Cached in PDK
   DataStorage keyed by `schemaId`, TTL `refreshIntervalSeconds`, lazy refresh with a
   single-flight CAS lock. Any CDGC failure ⇒ `None` ⇒ passthrough (fail-open).
6. `semantic::build_contract` — the governed fields present (when
   `annotateFieldsPresentOnly`), most-restrictive first, meaning clipped to `maxMeaningChars`,
   capped at `maxContractEntries`. Empty ⇒ passthrough (stay quiet).
7. Attach: `result.structuredContent._semanticContract` (only if `structuredContent` exists —
   never fabricated) + a delimited block appended to `result.content[]`; REST gets a top-level
   `_semanticContract`. Re-serialize (re-wrap SSE as one `data:` frame) and `set_body`. Any
   `set_body` failure restores the original body (fail-open).

## Build & test

```bash
cargo test --lib                                  # semantic.rs + claims.rs unit tests
cargo check --target wasm32-wasip1 --release      # wasm build
make release                                      # publish impl asset (cargo-anypoint)
```

`MIN_FLEX_VERSION` 1.9.3, `cargo-anypoint@1.9.0`. Egress hosts (`cdgcLoginUrl`,
`cdgcSearchUrl`) are created as services in the `#[entrypoint_flex] init` in
`generated/config.rs`.
