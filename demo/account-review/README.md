# Demo: Account Review — semantic understanding with an A2D-published MCP server

The agent gets one question and three tools. Without the policy it reads the field names literally and
gets four of four answers wrong. With **CDGC Semantic Contract 1.1.0** on the gateway, every response
carries the meaning of its fields from the CDGC glossary, and the same agent answers correctly.

> **User:** Prepare the Q3 account review for Velo Club Lausanne (ACC-2041): are they an active
> customer, what revenue did we make, what's their loyalty standing, and can we ship their pending
> 30-helmet order from stock?

## The four traps

| Field (tool) | What the name suggests | What CDGC says (term) | Correct answer |
|---|---|---|---|
| `is_active: true` (get_account) | an active customer | **Account Open Flag**: the account is open. **Active Customer** = invoiced within 90 days of the as-of date | last invoice 2026-07-21, as-of 2026-10-30 = 101 days → **lapsed**, not active |
| `orders[].order_total` (list_account_orders) | sum the orders: **CHF 32,862.40** | **Gross Order Value**: Rappen (1/100 CHF), incl. 8.1 % VAT; **Net Revenue** counts invoiced orders only, net of VAT | (486 450 + 594 550) / 100 / 1.081 = **CHF 10,000.00** |
| `loyalty_score: 1` (get_account) | a low score | **Loyalty Tier Rank**: 1 = Platinum (best) … 4 = Bronze | **Platinum**, the top tier |
| `qty_available: 45` (get_stock_position) | 45 ≥ 30, ship it | **Physical Stock On Hand**; **Available To Promise** = on hand − committed − quality hold | 45 − 20 − 10 = **15** → cannot ship 30 |

The API specs keep the descriptions deliberately thin ("Account flag.", "Order total.", "Quantity."),
the way most real APIs are. The meaning lives only in the catalog.

## What is in this folder

Everything is generated from one model by `build.py` (`python3 -I build.py`), so the REST APIs, the
A2D MCP server design and the glossary cannot drift apart. All data is fictitious.

| File | What it is |
|---|---|
| `exchange/<asset>.json` | OAS 3.0.3 for Exchange: `customer-account-api` (2 endpoints), `order-ledger-api` (2), `stock-position-api` (1) |
| `a2d/<asset>.json` | the same three APIs as A2D mock REST APIs |
| `mcp-server.json` | **Account Review MCP**, the A2D MCP server design with 3 tools, published to CDGC |
| `glossary.json` | business area **Account Review**, 14 business terms, related-term links and the field → term map |

The tool names are the names an MCP Bridge gives them, `<exchange-asset-id>_<operationId>`:
`customer-account-api_get_account`, `order-ledger-api_list_account_orders`,
`stock-position-api_get_stock_position`. The bridge also exposes `list_accounts` and `get_order`;
they are not catalogued and pass through unannotated.

## Setup

1. **A2D**: import `a2d/*.json` as mock REST APIs and `mcp-server.json` as an MCP server.
2. **Exchange**: upload `exchange/*.json` as `rest-api` assets, version 1.0.0.
3. **CDGC**: publish the A2D MCP server to IDMC (A2D "Publish to IDMC"). Then create the glossary and
   link each `McpToolOutputField` to its term (`IClassTechnicalGlossaryBase`, curation ACCEPTED).
4. **Gateway**: deploy the three REST APIs with the A2D mocks as upstreams
   (`https://www.a2d-ai.com/api/platform/<apiId>/api/<base path>`), then an **MCP Bridge** over them.
5. **Policies on the bridge**: **MCP Support**, then `cdgc-semantic-contract` **1.1.0** with
   `mcpServerAssetId` = the CDGC id of the *Account Review MCP* server asset (plus the usual IDMC login
   settings). No `toolSchemas` needed.

The tenant ids (A2D, Exchange org, CDGC) go in a git-ignored `*.local.json`, never in this folder's
tracked files.

## Run of show

1. **Before**: point the agent at the bridge without the policy (or at the raw MCP server). Ask the
   question. Expected answer: *"active customer, CHF 32,862.40 revenue, low loyalty score of 1, and yes,
   45 helmets are available."* Four wrong answers, all of them plausible.
2. **Apply** `cdgc-semantic-contract` 1.1.0 to the bridge.
3. **After**: ask the same question. Each tool result now carries a `_semanticContract` with the term,
   meaning, business logic and examples for every governed field. Expected answer: *"Open account but
   a lapsed customer (last invoice 101 days ago), net revenue CHF 10,000.00 from two invoiced orders,
   Platinum tier, and only 15 helmets available to promise, so the 30-helmet order cannot ship in full."*
4. **Show CDGC**: open a term such as *Gross Order Value* and its linked output field
   `orders[].order_total` under the MCP tool. Changing the definition in the catalog changes what the
   agent is told, after the cache refresh, with no change to the API, the bridge or the agent.

## Mock data

| Account | is_active | last_invoice_date | loyalty_score |
|---|---|---|---|
| ACC-2041 Velo Club Lausanne | true | 2026-07-21 | 1 |

| Order | status | order_total (Rappen, incl. VAT) |
|---|---|---|
| SO-7101 | invoiced | 486 450 |
| SO-7102 | invoiced | 594 550 |
| SO-7103 | cancelled | 1 621 500 |
| SO-7104 | pending (30 × HELM-TRAIL-26) | 583 740 |

| SKU | qty_available | qty_committed | qty_blocked |
|---|---|---|---|
| HELM-TRAIL-26 | 45 | 20 | 10 |
