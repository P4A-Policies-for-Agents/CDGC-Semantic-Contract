#!/usr/bin/env bash
# One-shot live demo driver for CDGC Semantic Contract.
#
#   source demo/env.local.sh   # sets CSC_GW_URL (+ optional CSC_RAW_URL) — see env.local.sh.example
#   ./demo/demo.sh
#
# Runs the annotation-diff agent against the governed MCP endpoint (and, when
# CSC_RAW_URL is set, the raw upstream mock) to show the SAME tools/call payload
# with vs. without the gateway-attached, CDGC-resolved semantic contract.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ -z "${CSC_GW_URL:-}" ]]; then
  echo "CSC_GW_URL is not set. Run:  source demo/env.local.sh" >&2
  exit 1
fi

echo "════════════════════════════════════════════════════════════════════════"
echo " CDGC Semantic Contract — live demo"
echo " governed endpoint: ${CSC_GW_URL}"
[[ -n "${CSC_RAW_URL:-}" ]] && echo " raw upstream     : ${CSC_RAW_URL}"
echo "════════════════════════════════════════════════════════════════════════"
echo
python3 "${HERE}/agent.py" "${CSC_GW_URL}"
