# CDGC Semantic Contract — MuleSoft Omni/Flex Gateway Policy

An **outbound, fail-open semantic annotator** for a data product on the MuleSoft Omni/Flex
Gateway that **resolves the product's CDGC-governed fields live** and attaches, to each
successful tool result, the catalog's **meaning, classification and handling obligation** for
the governed fields present in the payload — so an AI agent reading an enterprise API response
does not act confidently on a meaning that is wrong.

MCP half-defines the contract an agent needs: `inputSchema` says *how to call* a tool;
`outputSchema` says the *abstract shape* of what comes back. Neither says what is **true** about
*this* payload's fields right now — that `email_address` is personal data that must not leave
the org, that `unit_cost` is a confidential internal figure and not the customer price. That
truth lives in the enterprise catalog, owned by data governance and versioned on a different
clock than the API. This policy attaches it at the gateway, as the **semantic contract**: the
third schema, resolved live from Informatica CDGC. **No model sits in the data path; the
upstream payload is never rewritten, only annotated.**

It is the response-leg sibling of **CDGC Purpose Binding** — the same CDGC `ccgf-searchv2`
resolution core, but folded into fail-open *annotation* (what is true about this payload)
instead of request-leg fail-closed *decision* (may this call happen?). It evolves the P4A
**MCP Semantic Contract** policy by binding its guidance to **live CDGC** rather than static
per-field config.

Built with the PDK, Rust → `wasm32-wasip1`, split-model. Applies to **MCP** (`tools/call`),
**A2A**, and **REST/HTTP APIs** (`assetTypes: mcp,rest,http`). For JSON-RPC the handshake
(`initialize`/`tools/list`/`notifications/*`) is never annotated — only successful `tools/call`
results are; a REST response is annotated as a top-level `_semanticContract`.

---

## How it annotates — CDGC schema → Business Term → meaning + classification

A data product's tool is bound to a scanned Informatica CDGC **schema**. A single MCP server (or
API) can expose **several tools, each fronting a different schema** — the policy's
**`toolSchemas`** map (`<toolName>=<schemaId>`, with a **`pathSchemas`** analog for REST) routes
each call to the schema it binds to; the optional single **`schemaId`** is the default/fallback.
A call that resolves to no schema passes through **unannotated** (fail-open — nothing to bind
against).

On the **request leg** the policy captures the routing key and resolves the schema, threading it
to the response. On the **response leg**, for a successful result, on a cache miss (keyed per
schema) it authenticates to IDMC (**Login → JWT**) and walks the catalog via **ccgf-searchv2**:

1. resolve the schema asset by `core.identity` → its `core.location`,
2. enumerate the schema's columns (children of that location),
3. resolve each column's Business-Term link (`IClassTechnicalGlossaryBase` relationship),
4. resolve the linked terms' names, **description (the meaning)**, and structured **IDMC
   Security Level** (`securityClassification`),
5. build the field → term → meaning/classification map: one entry per column.

**Classification resolution** per field: the term's structured Security Level
(`Public`/`Internal`/`Confidential`/`Restricted`) wins when present; otherwise it is inferred
from the term's **description** via keyword markers (`restrictedMarkers`/… , most-restrictive
wins). The result is cached in PDK DataStorage (lazy refresh, single-flight, `distributed` for
cross-replica) so most requests pay no extra CDGC round trip.

Each classification maps, by config, to a **handling obligation** attached to matching fields:

| Classification | Meaning (example) | Handling obligation (default) |
|---|---|---|
| `Restricted` | direct identifiers, PII (SSN, email) | *Do not disclose externally; use only for the stated purpose; minimise retention.* |
| `Confidential` | sensitive-but-not-identifying (consent, cost) | *Need-to-know internal use; do not disclose to customers or third parties.* |
| `Internal` | internal-use-only | *Internal use only. Not for external distribution.* |
| `Public` | open data | *(none — nothing to state)* |

All four are configurable free-text strings (empty = say nothing for that tier).

## What it attaches

For each successful result, the semantic contract covers the governed fields **present in the
payload** (when `annotateFieldsPresentOnly`, the default — so a payload with nothing governed is
left byte-identical). Per field: the governing **term**, its **meaning** (catalog description,
clipped to `maxMeaningChars`), its **classification**, and the **obligation**. Entries are
ordered most-restrictive first and capped at `maxContractEntries`. It lands in **two** places
clients disagree on:

- `result.structuredContent._semanticContract` — a structured object (only when the upstream
  returned a `structuredContent`; never fabricated).
- a delimited block appended to `result.content[]` — for clients that read `content.text`. The
  block is explicitly **gateway-authored** and fenced; any forged fence smuggled up from the
  payload is neutralised so a payload cannot impersonate gateway-authored guidance.

A REST JSON object gets a top-level `_semanticContract`.

## Fail-open

The policy **never blocks a response**. A CDGC error, an oversized body (`maxAnnotateBytes`), a
non-JSON body, an error result (`error` / `isError`), a handshake message, or a payload with no
governed field present — all pass through **byte-identical**. Annotation is additive guidance;
its absence must never break the tool.

---

## Live demo

The live demo fronts **two tools on one MCP server**, each routed by `toolSchemas` to its own
CDGC schema, and shows the **before/after diff** on the same `tools/call`:

- **`get_customer_profiles`** → **Customer 360 Profile** — `email_address` binds to **"Email
  Address"** (Restricted); `marketing_consent` binds to **"Marketing Consent"** (Confidential).
- **`get_product_catalog`** → **Product Catalog** — `unit_cost` / `list_price` bind to **"Unit
  Cost"** / **"List Price"** (Confidential).

The raw upstream result carries only the payload; the same call through the gateway carries the
payload **plus** the CDGC-resolved semantic contract — the meaning, Security Level and handling
obligation for each governed field, resolved live and attached at the gateway with no change to
the tool, the route, or the agent.

Full runbook and driver: [`demo/PROVISION.md`](demo/PROVISION.md), [`demo/agent.py`](demo/agent.py).
The `<gatewayPublicHost>` and schema ids live only in the git-ignored `demo/config.json` /
`demo/env.local.sh`.

```text
$ python demo/agent.py          # initialize → notifications/initialized → tools/call <tool>

get_customer_profiles → Customer 360 Profile
  RAW (upstream mock)                     → (no semantic contract attached)
  GOVERNED (gateway + CDGC Semantic Contract):
    • email_address    — "Email Address"    [restricted]   meaning + "Do not disclose externally…"
    • marketing_consent— "Marketing Consent" [confidential] meaning + "Need-to-know internal use…"
```

---

## Configuration

| Property | Req | Default | Notes |
|---|---|---|---|
| `cdgcLoginUrl` | ✓ | — | IDMC login host (`format:service`) |
| `cdgcSearchUrl` | ✓ | — | CDGC search API host (`format:service`); policy appends `/ccgf-searchv2/api/v1/search` |
| `cdgcOrgUsername` | ✓ | — | IDMC org user (`security:sensitive`) |
| `cdgcOrgPassword` | ✓ | — | IDMC org password (`security:sensitive`) |
| `schemaId` | | — | optional default governed CDGC schema asset id — fallback for any tool/path with no mapping. A call resolving to no schema passes through unannotated |
| `toolSchemas` | | `[]` | per-tool schema routing: `<toolName>=<schemaId>` entries; the invoked MCP tool selects its bound schema |
| `pathSchemas` | | `[]` | REST analog: `<pathPrefix>=<schemaId>` entries; longest matching path prefix wins |
| `schemaIdHeader` | | `x-dp-schema-id` | header the schema id may arrive on (used when no mapping matches) |
| `schemaIdClaim` | | — | JWT claim to source the schema id from (opt-in) |
| `restrictedObligation` | | *see table* | handling obligation attached to Restricted fields (free-text; `""` = none) |
| `confidentialObligation` | | *see table* | handling obligation for Confidential fields |
| `internalObligation` | | *see table* | handling obligation for Internal fields |
| `publicObligation` | | `""` | handling obligation for Public fields (empty = none) |
| `restrictedMarkers` / `confidentialMarkers` / `internalMarkers` / `publicMarkers` | | tier-specific keyword lists | description-keyword fallback used when a term has no structured Security Level; structured level always wins |
| `annotateFieldsPresentOnly` | | `true` | attach only for governed fields present in the payload; false attaches every governed field |
| `maxContractEntries` | | `24` | cap on field entries attached (most-restrictive first) |
| `maxMeaningChars` | | `400` | per-field cap (chars) on the attached meaning; longer clipped |
| `maxAnnotateBytes` | | `262144` | responses larger pass through unannotated (bounds the rewrite buffer) |
| `refreshIntervalSeconds` | | `86400` | meaning/classification-cache TTL (min 30) |
| `distributed` | | `false` | true = shared DataStorage across replicas |
| `timeout` | | `5000` | per-call CDGC timeout (ms), clamped to a ~15s chained refresh budget |

---

## Repository layout (split-model)

```
cdgc-semantic-contract-definition/   # policy definition (gcl.yaml, exchange.json)
cdgc-semantic-contract-flex/         # Rust implementation (Cargo, src/, playground)
demo/                                # live demo: A2D mock + anypoint-cli runbook
```

- **definition/** — `gcl.yaml` (config schema, metadata; `injectionPoint: outbound`),
  `exchange.json`, `Makefile`.
- **flex/** — `src/lib.rs` (request→response threading + the annotator + CDGC core),
  `src/semantic.rs` (pure contract-shaping logic, unit-tested — 15 tests), `src/claims.rs`
  (opt-in JWT decode), `src/generated/config.rs`, `Makefile`, `playground/`.
- **demo/** — `PROVISION.md` (runbook), `WALKTHROUGH.md` (story), `agent.py` + `demo.sh`
  (annotation-diff driver), `mcp-metadata.json`, `*.example` config (no secrets committed).

## Build & test

```bash
cd cdgc-semantic-contract-flex
cargo test --lib                                  # pure-logic unit tests (semantic.rs + claims.rs)
cargo check --target wasm32-wasip1 --release      # wasm build
```

Publish (definition then impl) and stand up the live demo per **demo/PROVISION.md**.

---

## Related policies

Sibling of **CDGC Purpose Binding**, with which it shares the CDGC `ccgf-searchv2` resolution
core (Login → JWT → schema → column → Business Term). Purpose Binding is a **request-leg
fail-closed gate** — deny the call when the declared purpose isn't permitted by the data's
classification. Semantic Contract is a **response-leg fail-open annotator** — attach what the
data *means* and how it must be handled to the result. Purpose Binding governs *whether the call
may happen*; Semantic Contract governs *whether the agent understands what it got back*. Both
move with the CDGC catalog, not with code.
