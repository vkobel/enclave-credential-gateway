#!/usr/bin/env bash
# test-e2e.sh — coco-gateway end-to-end tests
#
# Starts the gateway with docker compose, creates scoped registry tokens through
# the admin API, validates routing/auth behavior, checks CLI activation flows,
# and tears the compose project down on exit unless a gateway was already up.
#
# Usage:
#   export COCO_ADMIN_TOKEN=test-admin       # optional; defaults to test-admin
#   export OPENAI_API_KEY=sk-...            # optional, enables live OpenAI test
#   export ANTHROPIC_API_KEY=sk-ant-api-... # optional, enables live Anthropic test
#   export GITHUB_TOKEN=ghp_...            # optional, enables live GitHub REST + git tests
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
GH_E2E_WORKDIR=""
cleanup() {
  echo
  [[ -n "$GW_TMPFILE" && -f "$GW_TMPFILE" ]] && rm -f "$GW_TMPFILE"
  [[ -n "$CLI_HOME" && -d "$CLI_HOME" ]] && rm -rf "$CLI_HOME"
  [[ -n "$GH_E2E_WORKDIR" && -d "$GH_E2E_WORKDIR" ]] && rm -rf "$GH_E2E_WORKDIR"

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
  true
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
  local all_routes="${3:-false}"
  GW_STATUS=$(curl -s -o "$GW_TMPFILE" -w "%{http_code}" \
    -X POST "http://localhost:${GATEWAY_PORT}/admin/tokens" \
    -H "Authorization: Bearer ${COCO_ADMIN_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "{\"name\":\"${name}\",\"scope\":${scope_json},\"all_routes\":${all_routes}}" 2>/dev/null)
  GW_BODY=$(cat "$GW_TMPFILE")

  if [[ "$GW_STATUS" == "200" ]]; then
    CREATED_TOKEN=$(echo "$GW_BODY" | jq -r '.token')
    CREATED_TOKEN_ID=$(echo "$GW_BODY" | jq -r '.id')
    pass "Created token '$name' with scope $scope_json all_routes=$all_routes"
  else
    fail "Create token '$name' — expected 200, got $GW_STATUS"
    echo "    Body: $(echo "$GW_BODY" | head -3)"
  fi
}

section "Registry tokens"

create_token "e2e-all" '[]' true
ALL_TOKEN="$CREATED_TOKEN"
GW_STATUS=$(curl -s -o "$GW_TMPFILE" -w "%{http_code}" \
  -X POST "http://localhost:${GATEWAY_PORT}/admin/tokens" \
  -H "Authorization: Bearer ${COCO_ADMIN_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{"name":"e2e-all","scope":["openai"],"all_routes":false}' 2>/dev/null)
GW_BODY=$(cat "$GW_TMPFILE")
if [[ "$GW_STATUS" == "409" && "$GW_BODY" == *"already exists"* ]]; then
  pass "Duplicate token name rejected"
else
  fail "Duplicate token name — expected 409, got $GW_STATUS"
  echo "    Body: $(echo "$GW_BODY" | head -3)"
fi
create_token "e2e-openai" '["openai"]'
OPENAI_SCOPED_TOKEN="$CREATED_TOKEN"
create_token "e2e-github" '["github"]'
GITHUB_SCOPED_TOKEN="$CREATED_TOKEN"
create_token "e2e-revoked" '["openai"]'
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

gw_request_token GET /anthropic/ "$OPENAI_SCOPED_TOKEN"
[[ "$GW_STATUS" == "403" ]] \
  && pass "Out-of-scope registry token → 403" \
  || fail "Out-of-scope registry token → expected 403, got $GW_STATUS"

GW_STATUS=$(curl -s -o "$GW_TMPFILE" -w "%{http_code}" \
  -X DELETE "http://localhost:${GATEWAY_PORT}/admin/tokens/${REVOKED_TOKEN_ID}" \
  -H "Authorization: Bearer ${COCO_ADMIN_TOKEN}" 2>/dev/null)
[[ "$GW_STATUS" == "200" ]] && pass "Revoked registry token" || fail "Revocation → expected 200, got $GW_STATUS"

gw_request_token GET /openai/ "$REVOKED_TOKEN"
[[ "$GW_STATUS" == "407" ]] \
  && pass "Revoked token → 407" \
  || fail "Revoked token → expected 407, got $GW_STATUS"

section "Routing edge cases"

gw_request GET /unknown-route/
[[ "$GW_STATUS" == "404" ]] && pass "Unknown route → 404" || fail "Unknown route → expected 404, got $GW_STATUS"

section "Auth: HTTP Basic scheme"

# `git` over HTTPS authenticates with `Authorization: Basic base64(user:token)`,
# not Bearer. The gateway should decode Basic and validate either half against
# the registry. We assert auth passes (status != 407/401) — the actual upstream
# response depends on whether ANTHROPIC_API_KEY is set.
basic_value=$(printf 'x-access-token:%s' "$ALL_TOKEN" | base64)
status=$(curl -s -o /dev/null -w "%{http_code}" \
  -H "Authorization: Basic ${basic_value}" \
  "http://localhost:${GATEWAY_PORT}/anthropic/v1/messages" 2>/dev/null)
case "$status" in
  407|401) fail "Basic auth (token in password slot) → got $status, expected auth to pass" ;;
  *)       pass "Basic auth (token in password slot) accepted (status $status)" ;;
esac

basic_value=$(printf '%s:x-oauth-basic' "$ALL_TOKEN" | base64)
status=$(curl -s -o /dev/null -w "%{http_code}" \
  -H "Authorization: Basic ${basic_value}" \
  "http://localhost:${GATEWAY_PORT}/anthropic/v1/messages" 2>/dev/null)
case "$status" in
  407|401) fail "Basic auth (token in username slot) → got $status, expected auth to pass" ;;
  *)       pass "Basic auth (token in username slot) accepted (status $status)" ;;
esac

section "Auth: route credential headers"

status=$(curl -s -o /dev/null -w "%{http_code}" \
  -H "x-api-key: ${ALL_TOKEN}" \
  "http://localhost:${GATEWAY_PORT}/anthropic/v1/messages" 2>/dev/null)
case "$status" in
  407|401) fail "Anthropic x-api-key registry token → got $status, expected auth to pass" ;;
  *)       pass "Anthropic x-api-key registry token accepted (status $status)" ;;
esac

status=$(curl -s -o /dev/null -w "%{http_code}" \
  -H "Authorization: Bearer claude-ai-session-token" \
  -H "x-api-key: ${ALL_TOKEN}" \
  "http://localhost:${GATEWAY_PORT}/anthropic/v1/messages" 2>/dev/null)
case "$status" in
  407|401) fail "Anthropic x-api-key with conflicting Authorization → got $status, expected auth to pass" ;;
  *)       pass "Anthropic x-api-key wins over conflicting Authorization (status $status)" ;;
esac

section "Route: github (git smart-HTTP)"

# Resolves via the GitSmartHttp matcher, not a path prefix. The token is
# scoped to "github", which must cover both API and git endpoints because they
# share the same canonical_route.
git_path="/octocat/Spoon-Knife.git/info/refs?service=git-upload-pack"
status=$(curl -s -o /dev/null -w "%{http_code}" \
  -H "Authorization: Bearer ${GITHUB_SCOPED_TOKEN}" \
  "http://localhost:${GATEWAY_PORT}${git_path}" 2>/dev/null)
case "$status" in
  407|404) fail "git smart-HTTP routing → got $status, expected the gateway to recognise the path" ;;
  *)       pass "git smart-HTTP path resolved through gateway (status $status)" ;;
esac

# Same path, no auth — must 401 (git paths return 401 + WWW-Authenticate so git
# retries with credentials; 407 would cause git to bail without retrying).
status=$(curl -s -o /dev/null -w "%{http_code}" \
  "http://localhost:${GATEWAY_PORT}${git_path}" 2>/dev/null)
[[ "$status" == "401" ]] \
  && pass "git smart-HTTP without auth → 401" \
  || fail "git smart-HTTP without auth → expected 401, got $status"

# Same path, out-of-scope token — must 403.
status=$(curl -s -o /dev/null -w "%{http_code}" \
  -H "Authorization: Bearer ${OPENAI_SCOPED_TOKEN}" \
  "http://localhost:${GATEWAY_PORT}${git_path}" 2>/dev/null)
[[ "$status" == "403" ]] \
  && pass "git smart-HTTP with out-of-scope token → 403" \
  || fail "git smart-HTTP with out-of-scope token → expected 403, got $status"

section "CLI activation"

CLI_HOME=$(mktemp -d)
mkdir -p "$CLI_HOME/.config/coco"
cat > "$CLI_HOME/.config/coco/config.toml" <<EOF
gateway_url = "http://localhost:${GATEWAY_PORT}"
admin_token = "${COCO_ADMIN_TOKEN}"

[tokens.laptop]
token = "${ALL_TOKEN}"
scope = []
all_routes = true

[tokens.github_only]
token = "${GITHUB_SCOPED_TOKEN}"
scope = ["github"]
all_routes = false
EOF

CLI_STDOUT="$CLI_HOME/stdout.txt"
CLI_STDERR="$CLI_HOME/stderr.txt"
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
COCO_BIN="${TARGET_DIR%/}/debug/coco"
CLI_ENV=(HOME="$CLI_HOME")
[[ -n "${CARGO_HOME:-}" ]] && CLI_ENV+=(CARGO_HOME="$CARGO_HOME")
[[ -z "${CARGO_HOME:-}" && -n "$REAL_HOME" ]] && CLI_ENV+=(CARGO_HOME="$REAL_HOME/.cargo")
[[ -n "${RUSTUP_HOME:-}" ]] && CLI_ENV+=(RUSTUP_HOME="$RUSTUP_HOME")
[[ -z "${RUSTUP_HOME:-}" && -n "$REAL_HOME" ]] && CLI_ENV+=(RUSTUP_HOME="$REAL_HOME/.rustup")

if cargo build -q -p coco-cli; then
  pass "Built coco CLI"
else
  fail "cargo build -p coco-cli failed"
fi

if [[ ! -x "$COCO_BIN" ]]; then
  fail "Built coco CLI binary not found at $COCO_BIN"
fi

if env "${CLI_ENV[@]}" "$COCO_BIN" activate github_only --eval >"$CLI_STDOUT" 2>"$CLI_STDERR"; then
  pass "coco activate succeeds for non-OpenAI token"
else
  fail "coco activate should not fail for non-OpenAI token"
fi

grep -q "export GH_HOST=localhost:${GATEWAY_PORT}" "$CLI_STDOUT" \
  && pass "github-only activate exports are printed" \
  || fail "github-only env exports missing"

[[ ! -s "$CLI_STDERR" ]] \
  && pass "coco activate is quiet" \
  || fail "coco activate wrote unexpected stderr: $(head -1 "$CLI_STDERR")"

[[ ! -f "$CLI_HOME/.codex/config.toml" ]] \
  && pass "non-OpenAI activate does not write Codex config" \
  || fail "non-OpenAI activate wrote Codex config"

if env "${CLI_ENV[@]}" "$COCO_BIN" activate laptop --eval --tool codex >"$CLI_STDOUT" 2>"$CLI_STDERR"; then
  pass "coco activate --eval --tool codex writes generated Codex config"
else
  fail "coco activate --eval --tool codex should write generated Codex config"
fi

grep -q "export CODEX_HOME=.*\\.config/coco/generated/codex/laptop/home" "$CLI_STDOUT" \
  && pass "Codex eval exports generated CODEX_HOME" \
  || fail "Codex eval missing generated CODEX_HOME"

grep -q "openai_base_url = \"http://localhost:${GATEWAY_PORT}/openai/v1\"" "$CLI_HOME/.config/coco/generated/codex/laptop/home/config.toml" \
  && pass "Generated Codex config points at gateway OpenAI route" \
  || fail "Generated Codex config missing gateway OpenAI route"

section "Route: anthropic"

if [[ -z "${ANTHROPIC_API_KEY:-}" ]]; then
  skip "ANTHROPIC_API_KEY not set — skipping Anthropic test"
elif [[ "${ANTHROPIC_API_KEY}" == ccgw_* ]]; then
  skip "ANTHROPIC_API_KEY contains a CoCo phantom token — set it to the real Anthropic key for live gateway tests"
elif [[ "${COCO_TEST_ANTHROPIC_MODE:-apikey}" == "oauth" ]]; then
  info "COCO_TEST_ANTHROPIC_MODE=oauth — testing Claude Code OAuth path"
  GW_STATUS=$(curl -s -o "$GW_TMPFILE" -w "%{http_code}" \
    -X POST "http://localhost:${GATEWAY_PORT}/anthropic/v1/messages" \
    -H "Authorization: Bearer ${ALL_TOKEN}" \
    -H "Content-Type: application/json" \
    -H "anthropic-version: 2023-06-01" \
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

section "Route: github (live REST API + git smart-HTTP)"

if [[ -z "${GITHUB_TOKEN:-}" ]]; then
  skip "GITHUB_TOKEN not set — skipping live GitHub tests"
else

GH_E2E_WORKDIR=$(mktemp -d)

export HOME="$CLI_HOME"
export PATH="${COCO_BIN%/*}:$PATH"
eval "$("$COCO_BIN" activate github_only --eval --tool gh)"
export GIT_TERMINAL_PROMPT=0

# Resolve authenticated username
gh_user=$(curl -s \
  -H "Authorization: Bearer ${GITHUB_SCOPED_TOKEN}" \
  "http://localhost:${GATEWAY_PORT}/github/user" 2>/dev/null | jq -r '.login // empty')

if [[ -n "$gh_user" ]]; then
  pass "GitHub REST: GET /user → authenticated as ${gh_user}"
else
  fail "GitHub REST: GET /user → no login returned; skipping remaining GitHub tests"
  gh_user=""
fi

if [[ -n "$gh_user" ]]; then

GH_E2E_REPO="${gh_user}/coco-gateway-e2e"

# Pre-cleanup from any previous run (best effort; 403 = no delete_repo scope)
curl -s -o /dev/null -X DELETE \
  -H "Authorization: Bearer ${GITHUB_SCOPED_TOKEN}" \
  "http://localhost:${GATEWAY_PORT}/github/repos/${GH_E2E_REPO}" 2>/dev/null || true

# Create private repo
GW_STATUS=$(curl -s -o "$GW_TMPFILE" -w "%{http_code}" \
  -X POST \
  -H "Authorization: Bearer ${GITHUB_SCOPED_TOKEN}" \
  -H "Content-Type: application/json" \
  -d "{\"name\":\"coco-gateway-e2e\",\"private\":true,\"auto_init\":false,\"description\":\"coco-gateway e2e — safe to delete\"}" \
  "http://localhost:${GATEWAY_PORT}/github/user/repos" 2>/dev/null)
[[ "$GW_STATUS" == "201" ]] \
  && pass "GitHub REST: created private repo ${GH_E2E_REPO}" \
  || fail "GitHub REST: create repo → expected 201, got $GW_STATUS"

# View repo
GW_STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
  -H "Authorization: Bearer ${GITHUB_SCOPED_TOKEN}" \
  "http://localhost:${GATEWAY_PORT}/github/repos/${GH_E2E_REPO}" 2>/dev/null)
[[ "$GW_STATUS" == "200" ]] \
  && pass "GitHub REST: view repo → 200" \
  || fail "GitHub REST: view repo → expected 200, got $GW_STATUS"

# Clone via git smart-HTTP. Remote URL has no embedded token; the Git
# credential helper emitted by `coco activate --eval --tool gh` supplies Basic auth.
git_remote="http://localhost:${GATEWAY_PORT}/${GH_E2E_REPO}.git"
if git clone -q "$git_remote" "${GH_E2E_WORKDIR}/repo" 2>/dev/null; then
  pass "git clone via gateway (smart-HTTP, credential helper)"
else
  fail "git clone via gateway failed"
fi

remote_url=$(git -C "${GH_E2E_WORKDIR}/repo" remote get-url origin 2>/dev/null || true)
case "$remote_url" in
  *"${GITHUB_SCOPED_TOKEN}"*|*ccgw_*) fail "git remote URL contains a token" ;;
  *)                                  pass "git remote URL remains token-free" ;;
esac

# Push initial commit (tests git-receive-pack path)
cd "${GH_E2E_WORKDIR}/repo"
git config user.email "test@coco.local"
git config user.name "CoCo E2E"
echo "# coco-gateway-e2e" > README.md
git add README.md
git commit -q -m "init"
git branch -M main
if git push -q origin main 2>/dev/null; then
  pass "git push (initial commit) via gateway (smart-HTTP)"
else
  fail "git push via gateway failed"
fi

# Create issue
GW_STATUS=$(curl -s -o "$GW_TMPFILE" -w "%{http_code}" \
  -X POST \
  -H "Authorization: Bearer ${GITHUB_SCOPED_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{"title":"e2e test issue","body":"created by coco-gateway e2e"}' \
  "http://localhost:${GATEWAY_PORT}/github/repos/${GH_E2E_REPO}/issues" 2>/dev/null)
GH_ISSUE_NUM=$(cat "$GW_TMPFILE" | jq -r '.number // empty')
[[ "$GW_STATUS" == "201" && -n "$GH_ISSUE_NUM" ]] \
  && pass "GitHub REST: created issue #${GH_ISSUE_NUM}" \
  || fail "GitHub REST: create issue → expected 201, got $GW_STATUS"

# Push PR branch (tests another git-receive-pack round-trip)
git checkout -q -b feat/e2e-pr
echo "e2e change" >> README.md
git add README.md
git commit -q -m "e2e change"
if git push -q origin feat/e2e-pr 2>/dev/null; then
  pass "git push (PR branch) via gateway"
else
  fail "git push (PR branch) via gateway failed"
fi
cd - >/dev/null

# Create PR
GW_STATUS=$(curl -s -o "$GW_TMPFILE" -w "%{http_code}" \
  -X POST \
  -H "Authorization: Bearer ${GITHUB_SCOPED_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{"title":"e2e test PR","body":"created by coco-gateway e2e","head":"feat/e2e-pr","base":"main"}' \
  "http://localhost:${GATEWAY_PORT}/github/repos/${GH_E2E_REPO}/pulls" 2>/dev/null)
GH_PR_NUM=$(cat "$GW_TMPFILE" | jq -r '.number // empty')
[[ "$GW_STATUS" == "201" && -n "$GH_PR_NUM" ]] \
  && pass "GitHub REST: created PR #${GH_PR_NUM}" \
  || fail "GitHub REST: create PR → expected 201, got $GW_STATUS"

# Merge PR
if [[ -n "${GH_PR_NUM:-}" ]]; then
  GW_STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
    -X PUT \
    -H "Authorization: Bearer ${GITHUB_SCOPED_TOKEN}" \
    -H "Content-Type: application/json" \
    -d '{"merge_method":"squash"}' \
    "http://localhost:${GATEWAY_PORT}/github/repos/${GH_E2E_REPO}/pulls/${GH_PR_NUM}/merge" 2>/dev/null)
  [[ "$GW_STATUS" == "200" ]] \
    && pass "GitHub REST: merged PR #${GH_PR_NUM}" \
    || fail "GitHub REST: merge PR → expected 200, got $GW_STATUS"
fi

# git pull after merge (tests git-upload-pack with the squash commit)
cd "${GH_E2E_WORKDIR}/repo"
git checkout -q main
if git pull -q 2>/dev/null; then
  pass "git pull (post-merge) via gateway"
else
  fail "git pull via gateway failed"
fi
cd - >/dev/null

# Close issue
if [[ -n "${GH_ISSUE_NUM:-}" ]]; then
  GW_STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
    -X PATCH \
    -H "Authorization: Bearer ${GITHUB_SCOPED_TOKEN}" \
    -H "Content-Type: application/json" \
    -d '{"state":"closed"}' \
    "http://localhost:${GATEWAY_PORT}/github/repos/${GH_E2E_REPO}/issues/${GH_ISSUE_NUM}" 2>/dev/null)
  [[ "$GW_STATUS" == "200" ]] \
    && pass "GitHub REST: closed issue #${GH_ISSUE_NUM}" \
    || fail "GitHub REST: close issue → expected 200, got $GW_STATUS"
fi

# Create release
GW_STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
  -X POST \
  -H "Authorization: Bearer ${GITHUB_SCOPED_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{"tag_name":"v0.0.1-e2e","name":"E2E release","body":"test","draft":false,"prerelease":true}' \
  "http://localhost:${GATEWAY_PORT}/github/repos/${GH_E2E_REPO}/releases" 2>/dev/null)
[[ "$GW_STATUS" == "201" ]] \
  && pass "GitHub REST: created release" \
  || fail "GitHub REST: create release → expected 201, got $GW_STATUS"

# Cleanup: delete test repo. Requires delete_repo scope on GITHUB_TOKEN.
GW_STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
  -X DELETE \
  -H "Authorization: Bearer ${GITHUB_SCOPED_TOKEN}" \
  "http://localhost:${GATEWAY_PORT}/github/repos/${GH_E2E_REPO}" 2>/dev/null)
if [[ "$GW_STATUS" == "204" ]]; then
  pass "Cleaned up test repo ${GH_E2E_REPO}"
else
  skip "Repo cleanup skipped (GITHUB_TOKEN may lack delete_repo scope) — delete https://github.com/${GH_E2E_REPO} manually"
fi

fi # gh_user

unset GIT_TERMINAL_PROMPT

fi # GITHUB_TOKEN

echo
echo "════════════════════════════════════"
echo -e "  ${GREEN}PASS${NC}: $PASS   ${RED}FAIL${NC}: $FAIL   ${YELLOW}SKIP${NC}: $SKIP"
echo "════════════════════════════════════"

[[ $FAIL -gt 0 ]] && exit 1
exit 0
