#!/usr/bin/env bash
# test-e2e.sh — coco-gateway end-to-end integration tests
#
# Tests each registered route end-to-end through a running gateway instance.
# The script starts the gateway via docker compose, runs all checks, and tears
# down on exit regardless of outcome.
#
# Usage:
#   export COCO_PHANTOM_TOKEN=<token>        # required
#   export OPENAI_API_KEY=sk-...             # required for OpenAI test
#   export HTTPBIN_TOKEN=any-string          # required for httpbin test (any value works)
#   ./scripts/test-e2e.sh
#
# To skip a test, leave its credential unset — it will be marked SKIP.

set -euo pipefail

GATEWAY_PORT=8080
COMPOSE_PROJECT="coco-validate-$$"
PASS=0; FAIL=0; SKIP=0
GW_ALREADY_RUNNING=false

# ── Colour helpers ────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
pass() { echo -e "  ${GREEN}PASS${NC}  $*"; PASS=$((PASS+1)); }
fail() { echo -e "  ${RED}FAIL${NC}  $*"; FAIL=$((FAIL+1)); }
skip() { echo -e "  ${YELLOW}SKIP${NC}  $*"; SKIP=$((SKIP+1)); }
info() { echo -e "${CYAN}──${NC} $*"; }
section() { echo; echo -e "${CYAN}$*${NC}"; }

# ── Cleanup ───────────────────────────────────────────────────────────────────
GW_TMPFILE=""
cleanup() {
  echo
  [[ -n "$GW_TMPFILE" && -f "$GW_TMPFILE" ]] && rm -f "$GW_TMPFILE"

  if [[ "$GW_ALREADY_RUNNING" == true ]]; then
    info "Gateway was already running — leaving it up"
  else
    info "Tearing down gateway"
    docker compose -p "$COMPOSE_PROJECT" down --remove-orphans 2>/dev/null || true
  fi
}
trap cleanup EXIT
GW_TMPFILE=$(mktemp)

# ── Prerequisite checks ───────────────────────────────────────────────────────
section "Prerequisites"

[[ -z "${COCO_PHANTOM_TOKEN:-}" ]] && { echo -e "${RED}ERROR: COCO_PHANTOM_TOKEN is not set${NC}"; exit 1; }

for cmd in docker curl jq; do
  if command -v "$cmd" &>/dev/null; then
    pass "$cmd found"
  else
    echo -e "${RED}ERROR: '$cmd' not found on PATH${NC}"; exit 1
  fi
done

# ── Start gateway ─────────────────────────────────────────────────────────────
section "Starting gateway"

# Check if something is already listening on the gateway port
if curl -s -o /dev/null --connect-timeout 1 "http://localhost:${GATEWAY_PORT}/" 2>/dev/null; then
  GW_ALREADY_RUNNING=true
  pass "Gateway is already running (port $GATEWAY_PORT) — skipping docker compose up"
else
  info "Running: docker compose up --build --detach"
  docker compose -p "$COMPOSE_PROJECT" up --build --detach 2>&1 | tail -5

  info "Waiting for gateway to respond"
  for i in $(seq 1 30); do
    status=$(curl -s -o /dev/null -w "%{http_code}" \
      "http://localhost:${GATEWAY_PORT}/openai/" 2>/dev/null || echo "000")
    if [[ "$status" =~ ^(407|200|404|503)$ ]]; then
      pass "Gateway is up (port $GATEWAY_PORT)"
      break
    fi
    [[ $i -eq 30 ]] && { echo -e "${RED}ERROR: Gateway did not start after 30s${NC}"; exit 1; }
    sleep 1
  done
fi

# ── Helper ────────────────────────────────────────────────────────────────────
# gw_request <method> <path> [extra curl args...]
# Sets GW_STATUS (HTTP code) and GW_BODY (response body).
GW_STATUS=""; GW_BODY=""
gw_request() {
  local method="$1"; shift
  local path="$1"; shift
  GW_STATUS=$(curl -s -o "$GW_TMPFILE" -w "%{http_code}" \
    -X "$method" \
    -H "Proxy-Authorization: Bearer ${COCO_PHANTOM_TOKEN}" \
    "http://localhost:${GATEWAY_PORT}${path}" \
    "$@" 2>/dev/null)
  GW_BODY=$(cat "$GW_TMPFILE")
}

# ── Test: auth enforcement ────────────────────────────────────────────────────
section "Auth enforcement"

status=$(curl -s -o /dev/null -w "%{http_code}" \
  "http://localhost:${GATEWAY_PORT}/openai/" 2>/dev/null)
[[ "$status" == "407" ]] && pass "Missing token → 407" || fail "Missing token → expected 407, got $status"

status=$(curl -s -o /dev/null -w "%{http_code}" \
  -H "Proxy-Authorization: Bearer wrong-token" \
  "http://localhost:${GATEWAY_PORT}/openai/" 2>/dev/null)
[[ "$status" == "407" ]] && pass "Wrong token → 407" || fail "Wrong token → expected 407, got $status"

# ── Test: routing edge cases ──────────────────────────────────────────────────
section "Routing edge cases"

gw_request GET /unknown-route/
[[ "$GW_STATUS" == "404" ]] && pass "Unknown route → 404" || fail "Unknown route → expected 404, got $GW_STATUS"

# ── Test: httpbin ─────────────────────────────────────────────────────────────
section "Route: httpbin"

if [[ -z "${HTTPBIN_TOKEN:-}" ]]; then
  skip "HTTPBIN_TOKEN not set — skipping httpbin tests"
else
  # /headers echoes back exactly the headers httpbin received
  gw_request GET /httpbin/headers

  [[ "$GW_STATUS" == "200" ]] \
    && pass "Request reached httpbin (200)" \
    || fail "httpbin request failed — status $GW_STATUS"

  # Verify credential was injected with the right value
  injected=$(echo "$GW_BODY" | jq -r '.headers.Authorization // empty' 2>/dev/null)
  [[ "$injected" == "Bearer ${HTTPBIN_TOKEN}" ]] \
    && pass "Authorization header injected correctly" \
    || fail "Authorization header wrong — got: '$injected', expected: 'Bearer ${HTTPBIN_TOKEN}'"

  # Verify phantom token was stripped (httpbin must NOT see Proxy-Authorization)
  proxy_auth=$(echo "$GW_BODY" | jq -r '.headers["Proxy-Authorization"] // empty' 2>/dev/null)
  [[ -z "$proxy_auth" ]] \
    && pass "Proxy-Authorization stripped before forwarding" \
    || fail "Proxy-Authorization was NOT stripped — httpbin saw: '$proxy_auth'"
fi

# ── Test: anthropic ───────────────────────────────────────────────────────────
section "Route: anthropic"

if [[ -z "${ANTHROPIC_AUTH_TOKEN:-}" && -z "${ANTHROPIC_API_KEY:-}" ]]; then
  skip "ANTHROPIC_AUTH_TOKEN and ANTHROPIC_API_KEY not set — skipping Anthropic tests"
else
  # Determine which credential is active and which phantom auth header to use.
  # Mirrors the gateway's credential_sources priority: OAuth first, then API key.
  if [[ -n "${ANTHROPIC_AUTH_TOKEN:-}" ]]; then
    ANTHROPIC_PHANTOM_HEADER="Authorization"
    ANTHROPIC_PHANTOM_VALUE="Bearer ${COCO_PHANTOM_TOKEN}"
  else
    ANTHROPIC_PHANTOM_HEADER="x-api-key"
    ANTHROPIC_PHANTOM_VALUE="${COCO_PHANTOM_TOKEN}"
  fi

  # Send a minimal messages request (claude-haiku-4-5 is the cheapest model)
  GW_STATUS=$(curl -s -o "$GW_TMPFILE" -w "%{http_code}" \
    -X POST "http://localhost:${GATEWAY_PORT}/anthropic/v1/messages" \
    -H "${ANTHROPIC_PHANTOM_HEADER}: ${ANTHROPIC_PHANTOM_VALUE}" \
    -H "Content-Type: application/json" \
    -H "anthropic-version: 2023-06-01" \
    -d '{
      "model": "claude-haiku-4-5-20251001",
      "max_tokens": 8,
      "messages": [{"role": "user", "content": "Reply: OK"}]
    }' 2>/dev/null)
  GW_BODY=$(cat "$GW_TMPFILE")

  [[ "$GW_STATUS" == "200" ]] \
    && pass "Request reached Anthropic (200)" \
    || { fail "Anthropic request failed — status $GW_STATUS"; echo "    Body: $(echo "$GW_BODY" | head -3)"; }

  content_len=$(echo "$GW_BODY" | jq -r '.content | length' 2>/dev/null || echo 0)
  [[ "$content_len" -ge 1 ]] \
    && pass "Response has content ($content_len block(s))" \
    || fail "Response missing content field"
fi

# ── Test: OpenAI ──────────────────────────────────────────────────────────────
section "Route: openai"

if [[ -z "${OPENAI_API_KEY:-}" ]]; then
  skip "OPENAI_API_KEY not set — skipping OpenAI tests"
else
  gw_request POST /openai/v1/chat/completions \
    -H "Content-Type: application/json" \
    -d '{"model":"gpt-4o-mini","messages":[{"role":"user","content":"Reply: OK"}],"max_tokens":4}'

  [[ "$GW_STATUS" == "200" ]] \
    && pass "Request reached OpenAI (200)" \
    || { fail "OpenAI request failed — status $GW_STATUS"; echo "    Body: $(echo "$GW_BODY" | head -3)"; }

  choices=$(echo "$GW_BODY" | jq -r '.choices | length' 2>/dev/null || echo 0)
  [[ "$choices" -ge 1 ]] \
    && pass "Response has choices ($choices)" \
    || fail "Response missing choices field"
fi

# ── Summary ───────────────────────────────────────────────────────────────────
echo
echo "════════════════════════════════════"
echo -e "  ${GREEN}PASS${NC}: $PASS   ${RED}FAIL${NC}: $FAIL   ${YELLOW}SKIP${NC}: $SKIP"
echo "════════════════════════════════════"

[[ $FAIL -gt 0 ]] && exit 1
exit 0
