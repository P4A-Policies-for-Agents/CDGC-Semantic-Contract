# Live demo walkthrough — CDGC Semantic Contract

## The story (60 seconds)

An AI agent calls an MCP tool and gets back `{"email_address": "...", "marketing_consent": false,
"unit_cost": 42.0}`. MCP's `inputSchema` told it *how to call*; an `outputSchema` would tell it
the *abstract shape*. Neither tells it what is **true** about these fields right now: that
`email_address` is personal data it must not disclose externally, that `marketing_consent` is
the affirmative permission gate for outreach, that `unit_cost` is a confidential internal figure
and not the customer price. So the agent guesses from field names — and acts confidently on a
meaning that may be wrong.

**CDGC Semantic Contract** closes that gap at the gateway. Two tools live on one MCP server, each
routed — via the policy's `toolSchemas` map — to a *different* scanned Informatica CDGC schema:

- **`get_customer_profiles`** → the **Customer 360 Profile** schema. `email_address` binds to the
  Business Term **"Email Address"** (Restricted, via a "personal data" description marker);
  `customer_id` → **"Customer Identifier"** and `marketing_consent` → **"Marketing Consent"** carry
  catalog *meanings* but no structured Security Level, so they attach unclassified (meaning only).
- **`get_product_catalog`** → the **Product Catalog** schema. `unit_cost` and `list_price` bind to
  **"Unit Cost"** / **"List Price"** (Confidential, via the structured IDMC Security Level).

For each **successful** `tools/call`, the gateway resolves those fields' Business Terms live from
CDGC (Login → JWT → ccgf-searchv2, one resolution per schema, cached) and attaches — additively —
a **semantic contract** covering the governed fields *present in that payload*: the term, its
catalog **meaning**, its IDMC **Security Level**, and the **handling obligation** that level
implies. It lands in two places clients disagree on: `result.structuredContent._semanticContract`
and a delimited block in `result.content[]`. The payload is passed through byte-for-byte; there
is **no model in the data path**.

The headline is the **before/after diff**: the raw upstream result vs. the same result through
the gateway. Same bytes of payload, plus governance-authored meaning the agent can actually read.
The catalog is the source of truth — edit a Business Term's description or reclassify it, and the
attached contract changes at the next cache refresh, with no change to the tool, the route, or
the agent. It is **fail-open**: a CDGC error, an error result, or a payload with nothing governed
present all pass through unchanged.

## What to show

1. `source demo/env.local.sh && ./demo/demo.sh` — both tools, RAW vs GOVERNED, side by side.
2. The GOVERNED result's `_semanticContract`: `email_address — "Email Address" [restricted]` with
   its meaning + "Do not disclose externally…" obligation; catalog `unit_cost` / `list_price` as
   `[confidential]` with their obligations; `customer_id` / `marketing_consent` attach their
   meaning unclassified.
3. The `content[]` block (for clients that read `content.text`), fenced with the gateway-authored
   trust delimiter — and note that any forged fence in the payload is neutralised.
4. That the RAW upstream result has **no** `_semanticContract` — the gateway added it, live.

## Captured transcript (verified live 2026-09-22, omni-gw-small, policy 1.0.2)

> **1.0.2 adds config-driven `termAttributes`:** extra CDGC Business Term fields
> (Reference ID, Business Logic, Examples, Format Type/Description, Critical Data
> Element — Alias Names via a config slot) are surfaced in the contract when
> populated on the term, present-only. Live, the Customer 360 / Product Catalog
> terms carry e.g. `Reference ID: BT-34`, `Format Type: Text`,
> `Critical Data Element: true`; unpopulated attributes are omitted. Values keep
> their catalog type (Examples an array, Critical Data Element a boolean).

```
╔════════════════════════════════════════════════════════════════════════
║  get_customer_profiles  →  Customer 360 Profile
╚────────────────────────────────────────────────────────────────────────
── RAW (upstream mock, no gateway) — HTTP 200 ──
  (no semantic contract attached)

── GOVERNED (gateway + CDGC Semantic Contract) — HTTP 200 ──
  structuredContent._semanticContract:
    source=informatica-cdgc  schemaId=b75b6eb4-…  fields=3
      • email_address — "Email Address" [restricted]
          meaning : The deliverable electronic mail address a customer has given for contact,
                    one per customer after deduplication. Personal data: governed for
                    deliverability and format, never offered as a way to search for a person.
          handling: Restricted / PII. Do not disclose externally; use only for the stated
                    purpose; minimise retention.
      • customer_id — "Customer Identifier" [unclassified]
          meaning : The durable surviving identifier for a customer party after match and merge.
      • marketing_consent — "Marketing Consent" [unclassified]
          meaning : Whether the customer has given affirmative, unexpired permission to be
                    contacted for marketing, per the jurisdiction that applies to them.
  content[] block: ===== BEGIN CDGC SEMANTIC CONTRACT (gateway-attached from Informatica CDGC…) =====
      … the same three fields, fenced, for clients that read content.text …
    ===== END CDGC SEMANTIC CONTRACT =====

╔════════════════════════════════════════════════════════════════════════
║  get_product_catalog  →  Product Catalog
╚────────────────────────────────────────────────────────────────────────
── RAW (upstream mock, no gateway) — HTTP 200 ──
  (no semantic contract attached)

── GOVERNED (gateway + CDGC Semantic Contract) — HTTP 200 ──
  structuredContent._semanticContract:
    source=informatica-cdgc  schemaId=cb2345f7-…  fields=2
      • list_price — "List Price" [confidential]
          meaning : The recommended retail price per unit in reporting currency, before any
                    promotion or contract discount. Not what was charged.
          handling: Confidential. Need-to-know internal use; do not disclose to customers or
                    third parties.
      • unit_cost — "Unit Cost" [confidential]
          meaning : The landed cost of one unit in reporting currency, including freight and duty.
                    Confidential: it discloses supplier terms, and margin can be derived from it
                    and the list price together.
          handling: Confidential. Need-to-know internal use; do not disclose to customers or
                    third parties.
```

Both tools ran the same `initialize` → `notifications/initialized` → `tools/call` handshake, first
against the raw A2D mock (no contract) then through the governed gateway. The A2D mock returns a
`structuredContent`, so the contract landed in **both** `structuredContent._semanticContract` and
the `content[]` block. Every meaning, term name, classification and obligation was resolved live
from CDGC (Login → JWT → ccgf-searchv2) at the first call and cached; the upstream payload was
byte-identical in both cases. `email_address` classified Restricted via a description marker;
`unit_cost`/`list_price` Confidential via the structured Security Level; the two customer fields
with no structured level and no marker hit attached their catalog meaning unclassified.

## Talking points

- **The missing third schema.** `inputSchema` (how to call) + `outputSchema` (abstract shape) +
  **semantic contract** (what is true about *this* payload, per the enterprise catalog). The
  gateway supplies the third — the one owned by data governance and versioned on its own clock.
- **Per-tool binding, one server.** `toolSchemas` routes each MCP tool to its own scanned CDGC
  schema, so one gateway route annotates several data products at once, each with its own
  catalog-derived meanings.
- **Additive, never destructive.** The upstream payload is byte-identical; the contract is extra
  guidance. Nothing governed present ⇒ nothing attached. Fail-open on every error.
- **Faithful to the catalog.** Meaning = the Business Term's description; classification = its
  structured IDMC Security Level when present, description-keyword marker as fallback. No bespoke
  seeding, no hardcoded field list.
- **Trust boundary.** The attached block is explicitly gateway-authored and fenced; a payload
  cannot forge the fence (the sentinel is escaped in all embedded strings).
- **Sibling of CDGC Purpose Binding.** Same CDGC resolution core — one folds it into a
  request-leg fail-closed *decision*, this folds it into a response-leg fail-open *annotation*.
- **REST and A2A too.** A REST JSON object gets a top-level `_semanticContract`; MCP/A2A JSON-RPC
  results get both attach points. The handshake (`initialize`/`tools/list`) is never touched.
