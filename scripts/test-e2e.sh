#!/usr/bin/env bash
# test-e2e.sh — coco-gateway end-to-end tests
#
# Starts the gateway with docker compose, creates scoped registry tokens through
# the admin API, validates routing/auth behavior, checks CLI activation flows,
# and tears the compose project down on exit unless a gateway was already up.
#
# Usage:
#   export COCO_ADMIN_TOKEN=test-admin       # optional; defaults to test-admin
#   export HTTPBIN_TOKEN=anything           # optional, enables httpbin injection test
#   export OPENAI_API_KEY=sk-...            # optional, enables live OpenAI test
#   export ANTHROPIC_API_KEY=sk-ant-api-... # optional, enables live Anthropic test
#   ./scripts/test-e2e.sh
#
# To skip a live upstream test, leave its real credential unset.

set -euo pipefail

GATEWAY_PORT="${COCO_E2E_PORT:-8080}"
COMPOSE_PROJECT="coco-validate-$$"
COCO_ADMIN_TOKEN="${COCO_ADMIN_TOKEN:-test-admin}"
REAL_HOME="${HOME:-}"
PASS=0; FAIL=0; SKIP=0
GW_ALREADY_RUNNING=false

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
pass() { echo -e "  ${GREEN}PASS${NC}  $*"; PASS=$((PASS+1)); }
fail() { echo -e "  ${RED}FAIL${NC}  $*"; FAIL=$((FAIL+1)); }
skip() { echo -e "  ${YELLOW}SKIP${NC}  $*"; SKIP=$((SKIP+1)); }
info() { echo -e "${CYAN}──${NC} $*"; }
section() { echo; echo -e "${CYAN}$*${NC}"; }

GW_TMPFILE=""
CLI_HOME=""
COMPOSE_OVERRIDE=""
cleanup() {
  echo
  [[ -n "$GW_TMPFILE" && -f "$GW_TMPFILE" ]] && rm -f "$GW_TMPFILE"
  [[ -n "$CLI_HOME" && -d "$CLI_HOME" ]] && rm -rf "$CLI_HOME"

  if [[ "$GW_ALREADY_RUNNING" == true ]]; then
    info "Gateway was already running — leaving it up"
  else
    info "Tearing down gateway"
    if [[ -n "$COMPOSE_OVERRIDE" ]]; then
      docker compose -p "$COMPOSE_PROJECT" -f docker-compose.yml -f "$COMPOSE_OVERRIDE" down --remove-orphans 2>/dev/null || true
    else
      docker compose -p "$COMPOSE_PROJECT" down --remove-orphans 2>/dev/null || true
    fi
  fi
  [[ -n "$COMPOSE_OVERRIDE" && -f "$COMPOSE_OVERRIDE" ]] && rm -f "$COMPOSE_OVERRIDE"
}
trap cleanup EXIT
GW_TMPFILE=$(mktemp)

section "Prerequisites"

for cmd in docker curl jq cargo; do
  if command -v "$cmd" &>/dev/null; then
    pass "$cmd found"
  else
    echo -e "${RED}ERROR: '$cmd' not found on PATH${NC}"; exit 1
  fi
done

section "Starting gateway"

if curl -s -o /dev/null --connect-timeout 1 "http://localhost:${GATEWAY_PORT}/" 2>/dev/null; then
  GW_ALREADY_RUNNING=true
  pass "Gateway is already running (port $GATEWAY_PORT) — skipping docker compose up"
  status=$(curl -s -o "$GW_TMPFILE" -w "%{http_code}" \
    -H "Authorization: Bearer ${COCO_ADMIN_TOKEN}" \
    "http://localhost:${GATEWAY_PORT}/admin/tokens" 2>/dev/null)
  if [[ "$status" != "200" ]]; then
    if [[ -z "${COCO_E2E_PORT:-}" && "$GATEWAY_PORT" == "8080" ]]; then
      info "Existing gateway rejected COCO_ADMIN_TOKEN; starting isolated compose gateway on port 18080"
      GW_ALREADY_RUNNING=false
      GATEWAY_PORT=18080
      if curl -s -o /dev/null --connect-timeout 1 "http://localhost:${GATEWAY_PORT}/" 2>/dev/null; then
        echo -e "${RED}ERROR: fallback port ${GATEWAY_PORT} is also occupied.${NC}"
        echo "Set COCO_E2E_PORT to a free port or stop the process using 8080."
        exit 1
      fi
    else
      echo -e "${RED}ERROR: existing gateway rejected COCO_ADMIN_TOKEN (expected admin probe 200, got $status).${NC}"
      echo "Stop the process on port ${GATEWAY_PORT} or rerun with the admin token for that gateway."
      exit 1
    fi
  else
    pass "Existing gateway accepted COCO_ADMIN_TOKEN"
  fi
fi

if [[ "$GW_ALREADY_RUNNING" != true ]]; then
  COMPOSE_OVERRIDE=$(mktemp)
  cat > "$COMPOSE_OVERRIDE" <<EOF
services:
  coco-gateway:
    ports:
      - "${GATEWAY_PORT}:8080"
  caddy:
    profiles:
      - manual-caddy
EOF

  info "Running: docker compose up --build --detach"
  COCO_ADMIN_TOKEN="$COCO_ADMIN_TOKEN" docker compose -p "$COMPOSE_PROJECT" -f docker-compose.yml -f "$COMPOSE_OVERRIDE" up --build --detach 2>&1 | tail -5

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

GW_STATUS=""; GW_BODY=""
gw_request_token() {
  local method="$1"; shift
  local path="$1"; shift
  local token="$1"; shift
  GW_STATUS=$(curl -s -o "$GW_TMPFILE" -w "%{http_code}" \
    -X "$method" \
    -H "Authorization: Bearer ${token}" \
    "http://localhost:${GATEWAY_PORT}${path}" \
    "$@" 2>/dev/null)
  GW_BODY=$(cat "$GW_TMPFILE")
}

gw_request() {
  local method="$1"; shift
  local path="$1"; shift
  gw_request_token "$method" "$path" "$ALL_TOKEN" "$@"
}

CREATED_TOKEN=""; CREATED_TOKEN_ID=""
create_token() {
  local name="$1"
  local scope_json="$2"
  GW_STATUS=$(curl -s -o "$GW_TMPFILE" -w "%{http_code}" \
    -X POST "http://localhost:${GATEWAY_PORT}/admin/tokens" \
    -H "Authorization: Bearer ${COCO_ADMIN_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "{\"name\":\"${name}\",\"scope\":${scope_json}}" 2>/dev/null)
  GW_BODY=$(cat "$GW_TMPFILE")

  if [[ "$GW_STATUS" == "200" ]]; then
    CREATED_TOKEN=$(echo "$GW_BODY" | jq -r '.token')
    CREATED_TOKEN_ID=$(echo "$GW_BODY" | jq -r '.id')
    pass "Created token '$name' with scope $scope_json"
  else
    fail "Create token '$name' — expected 200, got $GW_STATUS"
    echo "    Body: $(echo "$GW_BODY" | head -3)"
  fi
}

section "Registry tokens"

create_token "e2e-all" '[]'
ALL_TOKEN="$CREATED_TOKEN"
create_token "e2e-httpbin" '["httpbin"]'
HTTPBIN_SCOPED_TOKEN="$CREATED_TOKEN"
create_token "e2e-github" '["github"]'
GITHUB_SCOPED_TOKEN="$CREATED_TOKEN"
create_token "e2e-revoked" '["httpbin"]'
REVOKED_TOKEN="$CREATED_TOKEN"
REVOKED_TOKEN_ID="$CREATED_TOKEN_ID"

section "Auth enforcement"

status=$(curl -s -o /dev/null -w "%{http_code}" \
  "http://localhost:${GATEWAY_PORT}/openai/" 2>/dev/null)
[[ "$status" == "407" ]] && pass "Missing token → 407" || fail "Missing token → expected 407, got $status"

status=$(curl -s -o /dev/null -w "%{http_code}" \
  -H "Authorization: Bearer wrong-token" \
  "http://localhost:${GATEWAY_PORT}/openai/" 2>/dev/null)
[[ "$status" == "407" ]] && pass "Wrong token → 407" || fail "Wrong token → expected 407, got $status"

gw_request_token GET /openai/ "$HTTPBIN_SCOPED_TOKEN"
[[ "$GW_STATUS" == "403" ]] \
  && pass "Out-of-scope registry token → 403" \
  || fail "Out-of-scope registry token → expected 403, got $GW_STATUS"

GW_STATUS=$(curl -s -o "$GW_TMPFILE" -w "%{http_code}" \
  -X DELETE "http://localhost:${GATEWAY_PORT}/admin/tokens/${REVOKED_TOKEN_ID}" \
  -H "Authorization: Bearer ${COCO_ADMIN_TOKEN}" 2>/dev/null)
[[ "$GW_STATUS" == "200" ]] && pass "Revoked registry token" || fail "Revocation → expected 200, got $GW_STATUS"

gw_request_token GET /httpbin/headers "$REVOKED_TOKEN"
[[ "$GW_STATUS" == "407" ]] \
  && pass "Revoked token → 407" \
  || fail "Revoked token → expected 407, got $GW_STATUS"

section "Routing edge cases"

gw_request GET /unknown-route/
[[ "$GW_STATUS" == "404" ]] && pass "Unknown route → 404" || fail "Unknown route → expected 404, got $GW_STATUS"

section "CLI activation"

CLI_HOME=$(mktemp -d)
mkdir -p "$CLI_HOME/.config/coco"
cat > "$CLI_HOME/.config/coco/config.toml" <<EOF
gateway_url = "http://localhost:${GATEWAY_PORT}"
admin_token = "${COCO_ADMIN_TOKEN}"

[tokens.laptop]
token = "${ALL_TOKEN}"
scope = []

[tokens.github_only]
token = "${GITHUB_SCOPED_TOKEN}"
scope = ["github"]
EOF

CLI_STDOUT="$CLI_HOME/stdout.txt"
CLI_STDERR="$CLI_HOME/stderr.txt"
CLI_ENV=(HOME="$CLI_HOME")
[[ -n "${CARGO_HOME:-}" ]] && CLI_ENV+=(CARGO_HOME="$CARGO_HOME")
[[ -z "${CARGO_HOME:-}" && -n "$REAL_HOME" ]] && CLI_ENV+=(CARGO_HOME="$REAL_HOME/.cargo")
[[ -n "${RUSTUP_HOME:-}" ]] && CLI_ENV+=(RUSTUP_HOME="$RUSTUP_HOME")
[[ -z "${RUSTUP_HOME:-}" && -n "$REAL_HOME" ]] && CLI_ENV+=(RUSTUP_HOME="$REAL_HOME/.rustup")

if env "${CLI_ENV[@]}" cargo run -q -p coco-cli -- env github_only --codex >"$CLI_STDOUT" 2>"$CLI_STDERR"; then
  pass "coco env --codex succeeds for non-OpenAI token"
else
  fail "coco env --codex should not fail for non-OpenAI token"
fi

grep -q "export GH_HOST=localhost:8080" "$CLI_STDOUT" \
  && pass "github-only env exports are printed" \
  || fail "github-only env exports missing"

[[ ! -s "$CLI_STDERR" ]] \
  && pass "coco env --codex compatibility path is quiet" \
  || fail "coco env --codex wrote unexpected stderr: $(head -1 "$CLI_STDERR")"

[[ ! -f "$CLI_HOME/.codex/config.toml" ]] \
  && pass "non-OpenAI --codex does not write Codex config" \
  || fail "non-OpenAI --codex wrote Codex config"

if env "${CLI_ENV[@]}" cargo run -q -p coco-cli -- tool install codex github_only >"$CLI_STDOUT" 2>"$CLI_STDERR"; then
  fail "coco tool install codex should reject non-OpenAI token"
else
  pass "coco tool install codex rejects non-OpenAI token"
fi

if env "${CLI_ENV[@]}" cargo run -q -p coco-cli -- env laptop --codex >"$CLI_STDOUT" 2>"$CLI_STDERR"; then
  pass "coco env --codex writes Codex config for all-route token"
else
  fail "coco env --codex should write Codex config for all-route token"
fi

grep -q 'openai_base_url = "http://localhost:8080/openai"' "$CLI_HOME/.codex/config.toml" \
  && pass "Codex config points at gateway OpenAI route" \
  || fail "Codex config missing gateway OpenAI route"

section "Route: httpbin"

if [[ -z "${HTTPBIN_TOKEN:-}" ]]; then
  skip "HTTPBIN_TOKEN not set — skipping httpbin injection tests"
else
  gw_request GET /httpbin/headers

  [[ "$GW_STATUS" == "200" ]] \
    && pass "Request reached httpbin (200)" \
    || fail "httpbin request failed — status $GW_STATUS"

  injected=$(echo "$GW_BODY" | jq -r '.headers.Authorization // empty' 2>/dev/null)
  [[ "$injected" == "Bearer ${HTTPBIN_TOKEN}" ]] \
    && pass "Authorization header injected correctly" \
    || fail "Authorization header wrong — got: '$injected', expected: 'Bearer ${HTTPBIN_TOKEN}'"
fi

section "Route: anthropic"

if [[ -z "${ANTHROPIC_API_KEY:-}" ]]; then
  skip "ANTHROPIC_API_KEY not set — skipping Anthropic test"
elif [[ "${COCO_TEST_ANTHROPIC_MODE:-apikey}" == "oauth" ]]; then
  info "COCO_TEST_ANTHROPIC_MODE=oauth — testing Claude Code OAuth path"
  GW_STATUS=$(curl -s -o "$GW_TMPFILE" -w "%{http_code}" \
    -X POST "http://localhost:${GATEWAY_PORT}/anthropic/v1/messages" \
    -H "Authorization: Bearer ${ALL_TOKEN}" \
    -H "Content-Type: application/json" \
    -H "anthropic-version: 2023-06-01" \
    -H "anthropic-beta: oauth-2025-04-20" \
    -d '{
      "model": "claude-haiku-4-5-20251001",
      "max_tokens": 8,
      "system": [{"type": "text", "text": "You are Claude Code, Anthropics official CLI for Claude."}],
      "messages": [{"role": "user", "content": "Reply: OK"}]
    }' 2>/dev/null)
  GW_BODY=$(cat "$GW_TMPFILE")

  [[ "$GW_STATUS" == "200" ]] \
    && pass "Claude Code OAuth path reached Anthropic (200)" \
    || { fail "Anthropic OAuth request failed — status $GW_STATUS"; echo "    Body: $(echo "$GW_BODY" | head -3)"; }

  content_len=$(echo "$GW_BODY" | jq -r '.content | length' 2>/dev/null || echo 0)
  [[ "$content_len" -ge 1 ]] \
    && pass "Response has content ($content_len block(s))" \
    || fail "Response missing content field"
else
  info "Testing regular API key path — set COCO_TEST_ANTHROPIC_MODE=oauth for Claude Code OAuth"
  GW_STATUS=$(curl -s -o "$GW_TMPFILE" -w "%{http_code}" \
    -X POST "http://localhost:${GATEWAY_PORT}/anthropic/v1/messages" \
    -H "x-api-key: ${ALL_TOKEN}" \
    -H "Content-Type: application/json" \
    -H "anthropic-version: 2023-06-01" \
    -d '{
      "model": "claude-haiku-4-5-20251001",
      "max_tokens": 8,
      "messages": [{"role": "user", "content": "Reply: OK"}]
    }' 2>/dev/null)
  GW_BODY=$(cat "$GW_TMPFILE")

  [[ "$GW_STATUS" == "200" ]] \
    && pass "API key path reached Anthropic (200)" \
    || { fail "Anthropic API key request failed — status $GW_STATUS"; echo "    Body: $(echo "$GW_BODY" | head -3)"; }

  content_len=$(echo "$GW_BODY" | jq -r '.content | length' 2>/dev/null || echo 0)
  [[ "$content_len" -ge 1 ]] \
    && pass "Response has content ($content_len block(s))" \
    || fail "Response missing content field"
fi

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

echo
echo "════════════════════════════════════"
echo -e "  ${GREEN}PASS${NC}: $PASS   ${RED}FAIL${NC}: $FAIL   ${YELLOW}SKIP${NC}: $SKIP"
echo "════════════════════════════════════"

[[ $FAIL -gt 0 ]] && exit 1
exit 0
