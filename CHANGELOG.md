# Changelog

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
