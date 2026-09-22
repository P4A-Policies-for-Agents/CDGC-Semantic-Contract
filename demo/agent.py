#!/usr/bin/env python3
"""
Agent simulation for the CDGC Semantic Contract demo.

The headline is a **before/after annotation diff** on the SAME MCP `tools/call`:

  * RAW (upstream A2D mock, no gateway)      → the tool result carries only its payload.
    An agent reading `email_address` or `unit_cost` has no idea what those fields *mean*,
    how sensitive they are, or how it is allowed to handle them. `inputSchema` says how to
    call; `outputSchema` says the abstract shape; neither says what is TRUE about this data.
  * GOVERNED (same call, through the Flex Gateway with CDGC Semantic Contract applied)
    → the result is byte-for-byte the same payload PLUS a gateway-attached semantic contract,
    resolved live from Informatica CDGC: per field present in the response, the governing
    Business Term, its catalog *meaning* (definition), its IDMC Security Level, and the
    handling obligation that level implies. Attached in two places clients disagree on:
    `result.structuredContent._semanticContract` and a delimited block in `result.content[]`.

There is no model in the data path; the upstream payload is never rewritten, only annotated.
A field with no governed Business Term, and a payload where nothing governed appears, are left
byte-identical (the policy stays quiet).

Two tools live on the SAME server, each routed (via the policy's `toolSchemas` map) to a
different scanned CDGC schema:
  * get_customer_profiles → "Customer 360 Profile"  (email_address → Restricted; other fields → catalog meaning)
  * get_product_catalog   → "Product Catalog"       (unit_cost / list_price → Confidential)

Usage:
    CSC_GW_URL="https://<host>/semantic-contract-demo/mcp" \\
    CSC_RAW_URL="https://www.a2d-ai.com/api/platform/<serverId>/mcp" \\
    python3 agent.py
"""
import json, os, ssl, sys, urllib.request, urllib.error

GW = (sys.argv[1] if len(sys.argv) > 1 else os.environ.get("CSC_GW_URL", "")).strip()
RAW = os.environ.get("CSC_RAW_URL", "").strip()
if not GW:
    sys.exit("Set CSC_GW_URL (governed MCP endpoint). See demo/env.local.sh.example")
_CTX = ssl.create_default_context(); _CTX.check_hostname = False; _CTX.verify_mode = ssl.CERT_NONE

TOOLS = [
    {"tool": "get_customer_profiles", "product": "Customer 360 Profile", "arg": "segment",
     "note": "email_address → Restricted (PII); customer_id / marketing_consent → catalog meaning"},
    {"tool": "get_product_catalog", "product": "Product Catalog", "arg": "category",
     "note": "unit_cost, list_price → Confidential"},
]
DELIM_OPEN = "===== BEGIN CDGC SEMANTIC CONTRACT"


def _post(url, body, sid=None):
    h = {"Content-Type": "application/json",
         "Accept": "application/json, text/event-stream",
         "Accept-Encoding": "identity"}
    if sid:
        h["mcp-session-id"] = sid
    req = urllib.request.Request(url, data=json.dumps(body).encode(), method="POST", headers=h)
    try:
        resp = urllib.request.urlopen(req, timeout=25, context=_CTX)
        return resp.status, resp.headers, resp.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.headers, e.read().decode()


def _parse_sse(raw):
    data = []
    for line in raw.splitlines():
        if line.startswith("data:"):
            data.append(line[len("data:"):].lstrip())
        elif not line and data:
            break
    blob = "\n".join(data) if data else raw
    try:
        return json.loads(blob)
    except Exception:
        return {}


def handshake(url):
    init = {"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-03-26", "capabilities": {},
                       "clientInfo": {"name": "csc-demo", "version": "1"}}}
    _, hdrs, _ = _post(url, init)
    sid = hdrs.get("mcp-session-id")
    if sid:
        _post(url, {"jsonrpc": "2.0", "method": "notifications/initialized"}, sid)
    return sid


def call_tool(url, spec):
    sid = handshake(url)
    body = {"jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": spec["tool"], "arguments": {spec["arg"]: "all"}}}
    status, _, raw = _post(url, body, sid)
    return status, _parse_sse(raw)


def extract_contract(rpc):
    """Return (contract_dict_or_None, content_block_or_None) from a tools/call result."""
    result = rpc.get("result") or {}
    contract = ((result.get("structuredContent") or {}).get("_semanticContract"))
    block = None
    for c in result.get("content") or []:
        t = c.get("text") or ""
        if DELIM_OPEN in t:
            block = t
    return contract, block


def print_contract(contract, block):
    if contract:
        print("  structuredContent._semanticContract:")
        print(f"    source={contract.get('source')}  schemaId={contract.get('schemaId')}  "
              f"fields={len(contract.get('fields') or [])}")
        for f in contract.get("fields") or []:
            term = f.get("term") or "(governed field)"
            cls = f.get("classification") or "unclassified"
            meaning = (f.get("meaning") or "").strip()
            print(f"      • {f.get('field')} — \"{term}\" [{cls}]")
            if meaning:
                print(f"          meaning : {meaning}")
            if f.get("obligation"):
                print(f"          handling: {f.get('obligation')}")
    if block:
        print("  content[] block (for clients that read content.text):")
        for line in block.splitlines():
            print(f"    {line}")
    if not contract and not block:
        print("  (no semantic contract attached)")


def main():
    print(f"🎯  cdgc semantic contract  →  {GW}\n")
    print("Same MCP tools/call, seen twice: RAW (upstream mock) vs GOVERNED (through the")
    print("gateway). The gateway attaches — live from Informatica CDGC — what is TRUE about")
    print("each field in THIS payload: its governing Business Term, the term's meaning, its")
    print("Security Level, and the handling obligation. No model in the data path.\n")
    for spec in TOOLS:
        print("╔" + "═" * 72)
        print(f"║  {spec['tool']}  →  {spec['product']}")
        print(f"║  {spec['note']}")
        print("╚" + "─" * 72)

        if RAW:
            rstatus, rrpc = call_tool(RAW, spec)
            rc, rb = extract_contract(rrpc)
            print(f"── RAW (upstream mock, no gateway) — HTTP {rstatus} ──")
            print_contract(rc, rb)
            print()

        gstatus, grpc = call_tool(GW, spec)
        gc, gb = extract_contract(grpc)
        print(f"── GOVERNED (gateway + CDGC Semantic Contract) — HTTP {gstatus} ──")
        print_contract(gc, gb)
        print()

    print("The upstream payload is identical in both cases — the gateway adds a semantic")
    print("contract resolved live from the enterprise catalog (owned by data governance,")
    print("versioned there, changing independently of the API), so the agent reads the")
    print("data's meaning and handling rules instead of guessing from field names.")


if __name__ == "__main__":
    main()
