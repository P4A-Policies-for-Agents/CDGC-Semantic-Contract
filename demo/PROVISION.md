# Demo provisioning runbook (semantic annotation)

Stands up the live **CDGC Semantic Contract** demo with `anypoint-cli-v4` + the A2D MCP tools.
The policy is a response-leg, **fail-open** annotator: it attaches to each successful
`tools/call` result — as `structuredContent._semanticContract` and a delimited `content[]`
block — the CDGC-resolved semantic guidance for the governed fields present in the payload
(governing Business Term, its catalog meaning, its IDMC Security Level, and the handling
obligation). The upstream payload is never rewritten, only annotated; nothing governed present
⇒ byte-identical passthrough. You need a real IDMC/CDGC tenant with a scanned schema whose
columns are linked to Business Terms carrying meanings + a Confidential/Restricted Security
Level (or a description-keyword marker) so the attached contract shows real catalog content.

## Things this build wires up (fill each with your own tenant's values)

| Thing | Value |
|---|---|
| Anypoint org / env | `<orgId>` / Sandbox `<envId>` |
| A2D mock MCP server | `<mockServerId>` (two tools: `get_customer_profiles`, `get_product_catalog`) |
| Mock URL | `https://www.a2d-ai.com/api/platform/<mockServerId>/mcp` |
| Exchange asset (MCP server) | `semantic-contract-demo/1.0.0` (type `mcp`) |
| API Manager instance | `<apiInstanceId>` (label `semantic-contract-demo`) |
| Flex gateway target | omni-gw-small (has a public URL) |
| Applied policies | **MCP Support** (order 1) + `cdgc-semantic-contract` **1.0.2** (id `<policyId>`) |
| Governed schema — `get_customer_profiles` | Customer 360 Profile `<customer360SchemaId>` |
| Governed schema — `get_product_catalog` | Product Catalog `<productCatalogSchemaId>` |
| Tool→schema routing | policy `toolSchemas`: `get_customer_profiles=<c360>`, `get_product_catalog=<prod>` |
| Governed endpoint | `https://<gatewayPublicHost>/semantic-contract-demo/mcp` |

## 0. Confirm the schema's Business-Term meanings + classifications (CDGC ccgf-searchv2)

```bash
# Bearer via /identity-service/api/v1/Login -> /jwt/Token (same chain the policy uses)
JWT="<jwt>"; ORG="<orgId>"; SCHEMA="<schemaId>"

# Resolve the schema asset -> core.location, then enumerate columns under that location,
# their Business-Term links, and each linked term's core.description (the MEANING this policy
# attaches) + securityClassification — the same chain lib.rs runs in fetch_class_map().
curl -s "https://<cdgc-search-host>/ccgf-searchv2/api/v1/search" \
  -H "Authorization: Bearer ${JWT}" -H "X-INFA-ORG-ID: ${ORG}" \
  -H "X-INFA-SEARCH-LANGUAGE: elasticsearch" -H "Content-Type: application/json" \
  -d "{\"from\":0,\"size\":1,\"query\":{\"bool\":{\"must\":[{\"terms\":{\"elementType\":[\"OBJECT\"]}},{\"terms\":{\"core.identity\":[\"${SCHEMA}\"]}}]}}}"
```
Pick a `schemaId` whose columns link to Business Terms that have a **description** (so the
attached `meaning` is non-empty) and ideally a Confidential/Restricted Security Level (so the
`classification` + `obligation` are non-trivial). See `../cdgc-semantic-contract-flex/README.md`
for the full 5-step chain.

## 1. A2D mock (two tools, same records for everyone)

Reuse the existing A2D mock (`get_customer_profiles` / `get_product_catalog`), or create one
with `design_mcp_server` (type `mock`) + `add_mcp_tool`. Each tool returns a few sample rows
whose field names match the governed columns (`email_address`, `marketing_consent`,
`unit_cost`, `list_price`, …) so the annotator has present fields to bind. See
[`mcp-metadata.json`](mcp-metadata.json) for the tool contracts this demo publishes. If the
mock returns `structuredContent`, the contract lands there too; otherwise it lands only in the
`content[]` block — both are demonstrated by the agent.

## 2. Publish the policy (definition + flex impl) to Exchange

```bash
# from the repo root — each dir has a Makefile; `make release` publishes via anypoint-cli-v4 pdk.
# Publish the DEFINITION FIRST — `make release` on the flex impl runs config-gen against it.
make -C cdgc-semantic-contract-definition release
make -C cdgc-semantic-contract-flex       release
```
(If API Manager reports "no implementation for flexGateway version" when applying, wait
~10 min for Exchange indexing and retry.)

## 3. Publish + deploy the MCP Flex instance

```bash
anypoint-cli-v4 exchange:asset:upload --name "Semantic Contract Demo" \
  --type mcp --status published --properties='{"platform":"a2d"}' \
  --files='{"mcp-metadata.json":"./mcp-metadata.json"}' semantic-contract-demo/1.0.0

anypoint-cli-v4 api-mgr:api:manage semantic-contract-demo 1.0.0 \
  --environment Sandbox --isFlex --type mcp --withProxy \
  --scheme http --port 8081 --path "/semantic-contract-demo/" \
  --uri "https://www.a2d-ai.com/api/platform/<mockServerId>/" \
  --apiInstanceLabel semantic-contract-demo

anypoint-cli-v4 api-mgr:api:deploy <apiInstanceId> --environment Sandbox \
  --target <omni-gw-small-id> --gatewayVersion <ver> --overwrite
```

## 4. Apply the policies (MCP Support order 1, then the annotator — OUTBOUND)

The MCP Support policy is required for the gateway to speak MCP framing:

```bash
anypoint-cli-v4 api-mgr:policy:apply <apiInstanceId> mcp-support \
  --environment Sandbox --groupId 68ef9520-24e9-4cf2-b2f5-620025690913 \
  --policyVersion 1.0.1 --order 1
```

CDGC Semantic Contract is an **outbound** policy (`injectionPoint: outbound`), so
`api-mgr:policy:apply` requires an **`--upstreamId`** — the id of the API instance's upstream.
Fetch it first, then apply:

```bash
# Find the upstream id of the deployed MCP instance:
anypoint-cli-v4 api-mgr:api:describe <apiInstanceId> --environment Sandbox
#   ... look for the upstream/route entry -> its id (a UUID)

cp config.json.example config.json   # fill creds + toolSchemas map
anypoint-cli-v4 api-mgr:policy:apply <apiInstanceId> cdgc-semantic-contract \
  --environment Sandbox --groupId <orgId> --policyVersion 1.0.2 \
  --upstreamId <upstreamId> --configFile ./config.json
anypoint-cli-v4 api-mgr:api:redeploy <apiInstanceId> --environment Sandbox
```
`config.json` mirrors the `gcl.yaml` schema — see [`config.json.example`](config.json.example).
The key field for a multi-tool server is **`toolSchemas`** (`<toolName>=<schemaId>`). The
obligation strings, markers and budget fields can be omitted to take the built-in defaults.

> Upgrading an already-applied policy to a new version: remove the old application
> (`api-mgr:policy:remove <apiInstanceId> <oldPolicyId>`) then `apply` the new version with the
> same `--upstreamId`. Never delete + re-publish the *same* Exchange asset version — API
> Manager then fails `policy:apply` with `Schema … not found to validate instance`; bump the
> version instead.

## 5. Run

```bash
cp env.local.sh.example env.local.sh   # set CSC_GW_URL (+ optional CSC_RAW_URL for the diff)
source env.local.sh
./demo.sh
```

Expected — for each tool the agent prints the RAW upstream result (no `_semanticContract`) and
the GOVERNED result carrying the attached contract: `email_address — "Email Address"
[restricted]` with its meaning + "Do not disclose externally…" obligation; `marketing_consent`
and the catalog `unit_cost` / `list_price` as `[confidential]`. Fields with no governed term,
and any payload where nothing governed appears, are left byte-identical.

Quick manual check:

```bash
GW="https://<gatewayPublicHost>/semantic-contract-demo/mcp"
# initialize -> capture mcp-session-id -> notifications/initialized -> tools/call over SSE.
curl -sS -X POST "$GW" -H "Content-Type: application/json" \
  -H "Accept: application/json, text/event-stream" -H "Accept-Encoding: identity" \
  -H "mcp-session-id: s-demo" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_customer_profiles","arguments":{"segment":"all"}}}' \
  | sed -n 's/^data: //p' | python3 -m json.tool
```

## Notes
- Egress: the policy calls `cdgcLoginUrl` + `cdgcSearchUrl` (both `format:service`); confirm
  the gateway can reach `*.informaticacloud.com`.
- The resolved field→meaning/classification map is cached and lazily refreshed
  (`refreshIntervalSeconds`, default 86400s), so a just-edited Business-Term description can
  take up to that long to appear in the attached contract.
- **Fail-open**: a CDGC error, an oversized body (`maxAnnotateBytes`), a non-JSON body, an
  error result (`error` / `isError`), or a handshake message all pass through unchanged. The
  policy never blocks a response.
- `structuredContent._semanticContract` is only set when the upstream returned a
  `structuredContent` object (never fabricated); the `content[]` block is appended whenever
  `content` is an array. Forged trust-delimiter text in the payload is neutralised.
- Keep the definition's top-level `description` ≤256 chars (the flex-impl publish enforces it).
