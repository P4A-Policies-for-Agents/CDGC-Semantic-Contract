# Changelog

## 1.1.2
- **Fix: fast calls still came back without the contract.** 1.1.1 alone did not fix it on the gateway:
  calls that skipped the CDGC lookup were still unannotated, so the shared store did not hand the map
  back to the other workers. Each worker now keeps the maps it has resolved in memory and checks them
  first. The shared store is still read and written.
- A request with no copy of the map at all now does the CDGC lookup itself, even when another request
  holds the refresh lock. The lock now only decides who refreshes a stale map.
- Verified on a Flex Gateway 1.14.2 MCP Bridge: 36 of 36 repeated calls annotated, at about 0.7 s each.
- No configuration changes.

## 1.1.1
- **Fix: the class-map cache never served a hit.** `TermAttribute.value` is a `serde_json::Value`, which
  the PDK DataStorage default format (fixint) writes but cannot read back. Every cache read failed, so
  a response was annotated only when its request did a live CDGC lookup (roughly one call in three).
  The cache now stores the class map as a JSON string. Cache keys move to `csc-classmap-v2-` so 1.1.0
  entries in a shared store are not read.
- Regression tests round-trip the cached entry through the gateway's storage format.
- No configuration changes.

## 1.1.0
- **MCP servers catalogued from A2D.** New optional `mcpServerAssetId`: the called tool is found by exact
  name among the server's `a2d.agentic.McpTool` assets and its `McpToolOutputField` children are annotated.
- A `toolSchemas` / `schemaId` id may now be an MCP Tool asset instead of a schema.
- Output fields match the payload on their leaf key (`orders[].order_total` → `order_total`); schema
  columns keep exact matching.
- New optional `fieldClassTypes` (default `["a2d.agentic.McpToolOutputField"]`).
- Unchanged when the new settings are unset: same routing, same cache keys, same matching.
- Demo: [Account Review](demo/account-review/README.md).

## 1.0.2
- Config-driven `termAttributes`.
