# cdgc-semantic-contract (definition)

Policy **definition** asset for the **CDGC Semantic Contract** Omni/Flex Gateway policy — the
Exchange-facing metadata and configuration schema. The Rust implementation lives in the
sibling `../cdgc-semantic-contract-flex/`.

## Files

| File | Role |
|---|---|
| `gcl.yaml` | Policy metadata (title, category `MCP`, injection point `outbound`, asset types `mcp,rest,http`, interface scope `api,resource`) and the full configuration schema. |
| `exchange.json` | Exchange coordinates (groupId `030e0aac-30d9-460f-9234-428c16a123c4` / assetId `cdgc-semantic-contract` / version `1.0.0`). |
| `Makefile` | `make release` builds and publishes the definition asset via `anypoint-cli-v4 pdk policy-definition`. |

> **Outbound.** This is a response-leg policy (`injectionPoint: outbound`), so applying it in
> API Manager requires an `--upstreamId`. See `../demo/PROVISION.md`.

## Configuration schema (see `gcl.yaml`)

Required: `cdgcLoginUrl`, `cdgcSearchUrl` (both `format:service`), `cdgcOrgUsername`,
`cdgcOrgPassword` (both `security:sensitive`).

Optional: `schemaId` (default/fallback governed CDGC schema asset id — set it for a single-tool
server, or omit it for a pure multi-tool server routed entirely by `toolSchemas`; a call that
resolves to no schema passes through unannotated — fail-open), `toolSchemas` (per-tool schema
routing — `<toolName>=<schemaId>` entries, default `[]`), `pathSchemas` (REST analog —
`<pathPrefix>=<schemaId>`, longest prefix wins, default `[]`), `schemaIdHeader` (default
`x-dp-schema-id`), `schemaIdClaim`, the four per-Security-Level handling obligations
`restrictedObligation` / `confidentialObligation` / `internalObligation` / `publicObligation`
(free-text with sensible defaults; `""` = say nothing for that tier), the four
description-keyword marker lists `restrictedMarkers` / `confidentialMarkers` / `internalMarkers`
/ `publicMarkers` (fallback used when a Business Term carries no structured Security Level; each
has tenant-informed defaults), `annotateFieldsPresentOnly` (default `true` — attach only for
governed fields that appear in the payload), `maxContractEntries` (default 24),
`maxMeaningChars` (default 400), `maxAnnotateBytes` (default 262144 — bodies larger pass through
unannotated), `refreshIntervalSeconds` (default 86400, min 30), `distributed` (default `false`),
`timeout` (default 5000ms).

The schema id(s) — a default `schemaId` and/or the `toolSchemas`/`pathSchemas` maps — are the
only catalog-driven input: no per-field configuration is needed. The policy derives the
governed fields, their Business Terms, meanings and classifications from CDGC at runtime via the
implementation's ccgf-searchv2 resolution chain. `toolSchemas` routing is admin-owned and
coarse; the catalog stays authoritative for the meanings and classifications.

## Publish

```bash
make release
```

Publish the definition first, then the implementation asset from
`../cdgc-semantic-contract-flex`. See the repo root `README.md` for the policy overview and
`../demo/PROVISION.md` for the live-demo runbook.
