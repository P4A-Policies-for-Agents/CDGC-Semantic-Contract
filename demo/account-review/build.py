#!/usr/bin/env python3
"""Build the "Account Review" demo assets from one model, so they cannot drift apart:

  a2d/<api>.json        A2D REST API specs with mock scenarios (import_rest_api)
  exchange/<api>.json   OpenAPI 3.0 specs for Anypoint Exchange (rest-api assets)
  mcp-server.json       A2D MCP server design with the three tools (import_mcp_server)
  glossary.json         CDGC Business Terms and the output field -> term links

The MCP Bridge names a tool `<Exchange asset id>_<operationId>`, so the MCP server design uses
exactly those names and the outputSchema of each tool is the OpenAPI 200 response schema of its
operation. Everything here is fictitious test data; no tenant names or ids.

Usage: python3 build.py   (writes next to this file)
"""
import copy, json, os

HERE = os.path.dirname(os.path.abspath(__file__))

# ─── the data behind the mocks ──────────────────────────────────────────────
AS_OF = "2026-10-30"
ACCOUNT = {
    "account_id": "ACC-2041",
    "account_name": "Velo Club Lausanne",
    "segment": "club",
    "is_active": True,
    "last_invoice_date": "2026-07-21",
    "loyalty_score": 1,
    "credit_limit": 50000,
    "account_manager": "Lea Moret",
    "as_of_date": AS_OF,
}
OTHER_ACCOUNTS = [
    {"account_id": "ACC-2042", "account_name": "Genève Cycling Academy", "segment": "club", "is_active": True,
     "last_invoice_date": "2026-10-14", "loyalty_score": 3, "credit_limit": 20000, "account_manager": "Lea Moret",
     "as_of_date": AS_OF},
    {"account_id": "ACC-2043", "account_name": "Rhône Valley Riders", "segment": "club", "is_active": False,
     "last_invoice_date": "2025-11-02", "loyalty_score": 4, "credit_limit": 5000, "account_manager": "Marc Favre",
     "as_of_date": AS_OF},
]
# order_total is in minor units (Rappen) and includes 8.1 % Swiss VAT; only invoiced orders are revenue.
ORDERS = [
    {"order_id": "SO-7101", "order_date": "2026-07-04", "order_status": "invoiced", "order_total": 486450,
     "currency": "CHF", "line_summary": "25 x Club Jersey 26"},
    {"order_id": "SO-7102", "order_date": "2026-07-21", "order_status": "invoiced", "order_total": 594550,
     "currency": "CHF", "line_summary": "10 x Team Bib Shorts, 20 x Bottle Cage"},
    {"order_id": "SO-7103", "order_date": "2026-08-12", "order_status": "cancelled", "order_total": 1621500,
     "currency": "CHF", "line_summary": "6 x Carbon Wheelset 50mm"},
    {"order_id": "SO-7104", "order_date": "2026-09-29", "order_status": "pending", "order_total": 583740,
     "currency": "CHF", "line_summary": "30 x Trail Helmet 26", "sku": "HELM-TRAIL-26", "quantity": 30},
]
STOCK = {
    "sku": "HELM-TRAIL-26",
    "product_name": "Trail Helmet 26",
    "warehouse_id": "WH-LAU-01",
    "qty_available": 45,
    "qty_committed": 20,
    "qty_blocked": 10,
    "unit_price": 180.0,
    "as_of_date": AS_OF,
}

# ─── schemas (shared by OpenAPI responses and MCP outputSchemas) ────────────
def obj(props, required=None):
    o = {"type": "object", "properties": props}
    if required:
        o["required"] = required
    return o

S, I, N, B = ({"type": t} for t in ("string", "integer", "number", "boolean"))
def d(base, text):
    return dict(base, description=text)

ACCOUNT_SCHEMA = obj({
    "account_id": d(S, "Account identifier (ACC-nnnn)."),
    "account_name": d(S, "Registered name of the customer account."),
    "segment": d(S, "Customer segment: club, retail, corporate."),
    "is_active": d(B, "Account flag."),
    "last_invoice_date": d(S, "Date (YYYY-MM-DD)."),
    "loyalty_score": d(I, "Loyalty value."),
    "credit_limit": d(N, "Credit limit."),
    "account_manager": d(S, "Name of the account manager."),
    "as_of_date": d(S, "Date (YYYY-MM-DD)."),
}, ["account_id", "account_name"])
ACCOUNT_LIST_SCHEMA = obj({"accounts": {"type": "array", "items": ACCOUNT_SCHEMA}, "count": I})
ORDER_SCHEMA = obj({
    "order_id": d(S, "Sales order number (SO-nnnn)."),
    "order_date": d(S, "Date (YYYY-MM-DD)."),
    "order_status": d(S, "Status: pending, invoiced, cancelled."),
    "order_total": d(I, "Order total."),
    "currency": d(S, "ISO 4217 currency code."),
    "line_summary": d(S, "Short summary of the order lines."),
    "sku": d(S, "Stock keeping unit of a single-SKU order line, for the stock lookup."),
    "quantity": d(I, "Ordered quantity of that SKU."),
}, ["order_id"])
ORDER_LIST_SCHEMA = obj({
    "account_id": d(S, "Account identifier (ACC-nnnn)."),
    "period": d(S, "Reporting period, e.g. 2026-Q3."),
    "orders": {"type": "array", "items": ORDER_SCHEMA},
    "count": I,
})
STOCK_SCHEMA = obj({
    "sku": d(S, "Stock keeping unit."),
    "product_name": d(S, "Customer-facing product name."),
    "warehouse_id": d(S, "Warehouse identifier (WH-xxx-nn)."),
    "qty_available": d(I, "Quantity."),
    "qty_committed": d(I, "Quantity."),
    "qty_blocked": d(I, "Quantity."),
    "unit_price": d(N, "List price per unit in CHF, excluding VAT."),
    "as_of_date": d(S, "Date (YYYY-MM-DD)."),
}, ["sku"])
ERROR_SCHEMA = obj({"error": S, "message": S})

# ─── the three APIs and their five endpoints ────────────────────────────────
def ok(body):
    return {"statusCode": 200, "headers": {"Content-Type": "application/json"}, "body": json.dumps(body)}
def not_found(msg):
    return {"statusCode": 404, "headers": {"Content-Type": "application/json"},
            "body": json.dumps({"error": "not_found", "message": msg})}
def path_is(field, value):
    return [{"field": field, "value": value, "source": "path_param", "operator": "==="}]
def query_is(field, value):
    return [{"field": field, "value": value, "source": "query_param", "operator": "==="}]

def account_orders(account_id):
    return {"account_id": account_id, "period": "2026-Q3", "orders": ORDERS, "count": len(ORDERS)}

APIS = [
    {
        "asset": "customer-account-api",
        "name": "Customer Account API",
        "description": "Mock Customer Account API for the CDGC Semantic Contract 'Account Review' demo. Customer "
                       "accounts with status flags, loyalty and credit data. Field names are deliberately terse; "
                       "their business meaning is governed in CDGC. All data is fictitious test data.",
        "base_path": "/customer-account-api/v1",
        "endpoints": [
            {"method": "GET", "path": "/accounts/{accountId}", "operationId": "get_account",
             "summary": "Get a customer account",
             "description": "Returns one customer account: status flag, last invoice date, loyalty and credit data.",
             "params": [("accountId", "path", True, "Account identifier, e.g. ACC-2041")],
             "schema": ACCOUNT_SCHEMA, "tool": True,
             "scenarios": [
                 ("Velo Club Lausanne (ACC-2041)", 10, path_is("accountId", "ACC-2041"), ok(ACCOUNT)),
                 ("Genève Cycling Academy (ACC-2042)", 10, path_is("accountId", "ACC-2042"), ok(OTHER_ACCOUNTS[0])),
                 ("Rhône Valley Riders (ACC-2043)", 10, path_is("accountId", "ACC-2043"), ok(OTHER_ACCOUNTS[1])),
                 ("Account not found", 100, [], not_found("No account with this id")),
             ]},
            {"method": "GET", "path": "/accounts", "operationId": "list_accounts",
             "summary": "List customer accounts",
             "description": "Lists customer accounts, optionally by segment (club | retail | corporate).",
             "params": [("segment", "query", False, "Customer segment")],
             "schema": ACCOUNT_LIST_SCHEMA, "tool": False,
             "scenarios": [
                 ("All accounts", 100, [], ok({"accounts": [ACCOUNT] + OTHER_ACCOUNTS, "count": 3})),
             ]},
        ],
    },
    {
        "asset": "order-ledger-api",
        "name": "Order Ledger API",
        "description": "Mock Order Ledger API for the CDGC Semantic Contract 'Account Review' demo. Sales orders "
                       "per customer account and period, in every status. Amounts follow the ledger convention "
                       "governed in CDGC. All data is fictitious test data.",
        "base_path": "/order-ledger-api/v1",
        "endpoints": [
            {"method": "GET", "path": "/accounts/{accountId}/orders", "operationId": "list_account_orders",
             "summary": "List an account's orders for a period",
             "description": "Returns the sales orders of one account for a reporting period (default 2026-Q3).",
             "params": [("accountId", "path", True, "Account identifier, e.g. ACC-2041"),
                        ("period", "query", False, "Reporting period, e.g. 2026-Q3")],
             "schema": ORDER_LIST_SCHEMA, "tool": True,
             "scenarios": [
                 ("Velo Club Lausanne Q3 orders (ACC-2041)", 10, path_is("accountId", "ACC-2041"),
                  ok(account_orders("ACC-2041"))),
                 ("No orders in period", 100, [],
                  ok({"account_id": "unknown", "period": "2026-Q3", "orders": [], "count": 0})),
             ]},
            {"method": "GET", "path": "/orders/{orderId}", "operationId": "get_order",
             "summary": "Get a sales order",
             "description": "Returns one sales order by number.",
             "params": [("orderId", "path", True, "Sales order number, e.g. SO-7104")],
             "schema": ORDER_SCHEMA, "tool": False,
             "scenarios": [(f"Order {o['order_id']}", 10, path_is("orderId", o["order_id"]), ok(o)) for o in ORDERS]
                          + [("Order not found", 100, [], not_found("No order with this number"))]},
        ],
    },
    {
        "asset": "stock-position-api",
        "name": "Stock Position API",
        "description": "Mock Stock Position API for the CDGC Semantic Contract 'Account Review' demo. Stock "
                       "quantities per SKU in the warehouse ledger's own terms. All data is fictitious test data.",
        "base_path": "/stock-position-api/v1",
        "endpoints": [
            {"method": "GET", "path": "/stock/{sku}", "operationId": "get_stock_position",
             "summary": "Get the stock position of a SKU",
             "description": "Returns the warehouse quantities of one SKU.",
             "params": [("sku", "path", True, "Stock keeping unit, e.g. HELM-TRAIL-26")],
             "schema": STOCK_SCHEMA, "tool": True,
             "scenarios": [
                 ("Trail Helmet 26 (HELM-TRAIL-26)", 10, path_is("sku", "HELM-TRAIL-26"), ok(STOCK)),
                 ("SKU not found", 100, [], not_found("No stock position for this SKU")),
             ]},
        ],
    },
]

# ─── the glossary: terms, and which output field each one governs ───────────
G = "com.infa.ccgf.models.governance."
TERMS = [
    # name, security level, description (the meaning), business logic, examples, format description, CDE, governs
    ("Account Open Flag", "Internal",
     "True when the customer account is open for ordering (not closed or blocked). It does NOT mean the customer "
     "is an Active Customer: an open account can have bought nothing for months. Use Last Invoice Date to decide "
     "whether a customer is active.",
     "Active Customer = Last Invoice Date within 90 days of the reporting as-of date. is_active only says the "
     "account is open.",
     ["true: account open, may or may not be an Active Customer", "false: account closed or blocked"],
     "Boolean", False, [("get_account", "is_active")]),
    ("Last Invoice Date", "Internal",
     "Date of the customer's most recent invoiced order. This is the date that decides whether a customer is an "
     "Active Customer (invoiced within the last 90 days of the as-of date).",
     "days_since_invoice = as_of_date - last_invoice_date; Active Customer when days_since_invoice <= 90, "
     "otherwise Lapsed.",
     ["2026-07-21 with as-of 2026-10-30: 101 days, Lapsed (not active)"],
     "ISO 8601 date, YYYY-MM-DD", True, [("get_account", "last_invoice_date")]),
    ("Loyalty Tier Rank", "Internal",
     "Rank of the customer's loyalty tier, where 1 is the HIGHEST tier: 1 = Platinum, 2 = Gold, 3 = Silver, "
     "4 = Bronze. A lower number is a better customer; it is a rank, not a score.",
     "Lower is better. Never average or compare it as a score.",
     ["1 = Platinum (top tier)", "4 = Bronze (entry tier)"],
     "Integer 1-4", False, [("get_account", "loyalty_score")]),
    ("Credit Limit", "Confidential",
     "Maximum open receivables allowed for the account, in CHF. Internal credit risk data.",
     "", [], "Decimal, CHF", False, [("get_account", "credit_limit")]),
    ("Reporting As-Of Date", "Public",
     "The date the figures in this response were computed for. Use it, not today's date, for any day count.",
     "", [], "ISO 8601 date, YYYY-MM-DD", False, [("get_account", "as_of_date"), ("get_stock_position", "as_of_date")]),
    ("Order Booking Date", "Internal",
     "Date the order was booked (entered). It is not the invoice date and does not by itself make the order "
     "revenue.",
     "", ["2026-09-29"], "ISO 8601 date, YYYY-MM-DD", False, [("list_account_orders", "orders[].order_date")]),
    ("Order Status", "Internal",
     "Lifecycle status of a sales order. Only orders with status invoiced count as revenue; pending orders are "
     "open (not yet revenue) and cancelled orders are never revenue.",
     "Revenue includes order_status = invoiced only.",
     ["invoiced: revenue", "pending: open order, not revenue", "cancelled: never revenue"],
     "Text, one of pending | invoiced | cancelled", True, [("list_account_orders", "orders[].order_status")]),
    ("Gross Order Value", "Confidential",
     "Order value in MINOR currency units (Rappen: 100 = CHF 1.00), INCLUDING 8.1 % Swiss VAT. It is not net "
     "revenue: Net Revenue = sum of order_total over invoiced orders / 100 / 1.081.",
     "net_revenue_chf = sum(order_total where order_status = invoiced) / 100 / 1.081",
     ["486450 = CHF 4,864.50 incl. VAT = CHF 4,500.00 net"],
     "Integer, minor units (Rappen), VAT included", True, [("list_account_orders", "orders[].order_total")]),
    ("Physical Stock On Hand", "Internal",
     "Units physically in the warehouse, INCLUDING units already committed to other orders and units on quality "
     "hold. It is not what can be promised to a customer: Available To Promise = qty_available - qty_committed "
     "- qty_blocked.",
     "available_to_promise = qty_available - qty_committed - qty_blocked",
     ["45 on hand - 20 committed - 10 on hold = 15 available to promise"],
     "Integer, units", True, [("get_stock_position", "qty_available")]),
    ("Committed Quantity", "Internal",
     "Units on hand already allocated to other confirmed orders. Not available for a new or pending order.",
     "", [], "Integer, units", False, [("get_stock_position", "qty_committed")]),
    ("Quality Hold Quantity", "Internal",
     "Units on hand blocked by quality control (inspection, damage, recall). They cannot be shipped.",
     "", [], "Integer, units", False, [("get_stock_position", "qty_blocked")]),
    # glossary concepts, related to the field terms above (not linked to a field)
    ("Active Customer", "Internal",
     "A customer with at least one invoiced order in the 90 days before the reporting as-of date. Derived from "
     "Last Invoice Date, never from Account Open Flag.",
     "Last Invoice Date within 90 days of the as-of date.", [], "Derived", True, []),
    ("Net Revenue", "Confidential",
     "Invoiced sales in CHF, excluding VAT, excluding pending and cancelled orders.",
     "sum(order_total where order_status = invoiced) / 100 / 1.081", ["CHF 10,000.00 for ACC-2041 in 2026-Q3"],
     "Decimal, CHF, VAT excluded", True, []),
    ("Available To Promise", "Internal",
     "Units that can be promised to a customer now: on hand minus committed minus quality hold.",
     "qty_available - qty_committed - qty_blocked", ["15 for HELM-TRAIL-26 on 2026-10-30"],
     "Integer, units", True, []),
]
RELATED = [  # term -> term (relatedBusinessTerm)
    ("Account Open Flag", "Active Customer"), ("Last Invoice Date", "Active Customer"),
    ("Gross Order Value", "Net Revenue"), ("Order Status", "Net Revenue"),
    ("Physical Stock On Hand", "Available To Promise"), ("Committed Quantity", "Available To Promise"),
    ("Quality Hold Quantity", "Available To Promise"),
]
BUSINESS_AREA = ("Account Review", "Glossary for the account review use case: customer status, revenue and "
                 "stock terms whose meaning differs from what the API field names suggest.")


# ─── emitters ───────────────────────────────────────────────────────────────
def tool_name(api, ep):
    return f"{api['asset']}_{ep['operationId']}"

def a2d_spec(api):
    eps = []
    for ep in api["endpoints"]:
        eps.append({
            "method": ep["method"], "path": ep["path"], "summary": ep["summary"], "description": ep["description"],
            "mock_scenarios": [{"name": n, "priority": p, "conditions": c, "response": r}
                               for n, p, c, r in ep["scenarios"]],
        })
    return {"type": "rest_api", "version": "1.0", "api": {
        "name": api["name"], "description": api["description"], "base_path": api["base_path"],
        "auth_enabled": False, "endpoints": eps}}

def openapi(api):
    paths = {}
    for ep in api["endpoints"]:
        params = [{"name": n, "in": where, "required": req, "description": desc, "schema": {"type": "string"}}
                  for n, where, req, desc in ep["params"]]
        op = {"operationId": ep["operationId"], "summary": ep["summary"], "description": ep["description"],
              "parameters": params,
              "responses": {"200": {"description": "OK", "content": {"application/json": {"schema": ep["schema"]}}}}}
        if any(s[3]["statusCode"] == 404 for s in ep["scenarios"]):
            op["responses"]["404"] = {"description": "Not found",
                                      "content": {"application/json": {"schema": ERROR_SCHEMA}}}
        paths.setdefault(ep["path"], {})[ep["method"].lower()] = op
    return {"openapi": "3.0.3",
            "info": {"title": api["name"], "version": "1.0.0", "description": api["description"]},
            "servers": [{"url": "{{GATEWAY_URL}}" + api["base_path"], "description": "Omni Gateway"}],
            "paths": paths}

def input_schema(ep):
    props = {n: {"type": "string", "description": desc} for n, _, _, desc in ep["params"]}
    req = [n for n, _, r, _ in ep["params"] if r]
    s = {"type": "object", "properties": props}
    if req:
        s["required"] = req
    return s

def mcp_server():
    tools = []
    for api in APIS:
        for ep in api["endpoints"]:
            if not ep["tool"]:
                continue
            key = next(n for n, where, _, _ in ep["params"] if where == "path")
            mocks = []
            for n, _, c, r in ep["scenarios"]:
                if c:
                    mocks.append({"name": n, "condition": {"field": key, "operator": "===", "value": c[0]["value"]},
                                  "response": r["body"], "responseType": "json"})
            tools.append({"name": tool_name(api, ep), "description": ep["description"],
                          "inputSchema": input_schema(ep), "outputSchema": copy.deepcopy(ep["schema"]),
                          "mock_scenarios": mocks})
    return {"name": "Account Review MCP", "type": "mock",
            "description": "Account Review MCP: customer account, order ledger and stock position tools for the "
                           "CDGC Semantic Contract demo. Tool names match the MCP Bridge over the three Exchange "
                           "REST APIs (<asset>_<operationId>).",
            "tools": tools}

def glossary():
    tools = {ep["operationId"]: tool_name(api, ep) for api in APIS for ep in api["endpoints"]}
    terms = []
    for name, level, desc, logic, examples, fmt, cde, governs in TERMS:
        terms.append({"name": name, "securityClassification": level, "description": desc, "businessLogic": logic,
                      "examples": examples, "formatDescription": fmt, "isCDE": cde,
                      "fields": [{"tool": tools[op], "path": path} for op, path in governs]})
    return {"businessArea": {"name": BUSINESS_AREA[0], "description": BUSINESS_AREA[1]},
            "terms": terms, "related": [list(r) for r in RELATED]}

def write(rel, doc):
    p = os.path.join(HERE, rel)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "w") as f:
        json.dump(doc, f, indent=1, ensure_ascii=False)
        f.write("\n")
    print("wrote", rel)

def check():
    """The MCP tools are exactly the bridge names, and every governed path exists in its outputSchema."""
    server = mcp_server()
    assert len(server["tools"]) == 3, "the MCP server has three tools"
    schemas = {t["name"]: t["outputSchema"] for t in server["tools"]}
    def has(schema, path):
        node = schema
        for part in path.split("."):
            arr = part.endswith("[]")
            node = node["properties"][part[:-2] if arr else part]
            if arr:
                node = node["items"]
        return True
    for t in glossary()["terms"]:
        for f in t["fields"]:
            assert has(schemas[f["tool"]], f["path"]), f
    n = sum(len(a["endpoints"]) for a in APIS)
    assert 3 <= n <= 6, n

if __name__ == "__main__":
    check()
    for api in APIS:
        write(f"a2d/{api['asset']}.json", a2d_spec(api))
        write(f"exchange/{api['asset']}.json", openapi(api))
    write("mcp-server.json", mcp_server())
    write("glossary.json", glossary())
