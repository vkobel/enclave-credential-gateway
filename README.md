# CoCo Credential Gateway

A credential proxy for AI agents: agents authenticate with a phantom token, the gateway validates it and injects the real upstream API key, so secrets never touch the agent's host.

Remotely deployable with automatic TLS (Caddy). Hardware-attested deployment on Phala TDX CVM coming in Phase 1b.

---

## Quickstart — Local dev

**1. Set your credentials and start the gateway:**

```bash
export COCO_ADMIN_TOKEN=$(openssl rand -hex 32)   # admin API secret — save this
export ANTHROPIC_API_KEY=sk-ant-...
export OPENAI_API_KEY=sk-...
export GITHUB_TOKEN=ghp_...
export HTTPBIN_TOKEN=any-value                     # smoke test only

docker compose up -d --build
```

`COCO_ADMIN_TOKEN` is required. The gateway refuses to start without it.

**2. Mint a phantom token for your agent:**

```bash
curl -s -X POST http://localhost:8080/admin/tokens \
  -H "Authorization: Bearer $COCO_ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"laptop","scope":["anthropic","openai","api"]}' | jq .
# { "id": "...", "name": "laptop", "token": "ccgw_...", "scope": [...], ... }
```

The `token` field is shown **once**. Save it. `scope` is optional — omit it to allow all routes.

**3. Write the coco CLI config:**

```toml
# ~/.config/coco/config.toml
gateway_url = "http://localhost:8080"
admin_token = "<COCO_ADMIN_TOKEN>"

[tokens]
laptop = "ccgw_..."
```

**4. Activate your shell:**

```bash
eval $(coco env laptop)            # sets ANTHROPIC_BASE_URL, OPENAI_BASE_URL, GH_HOST, OLLAMA_HOST
eval $(coco env laptop --codex)    # also writes ~/.codex/config.toml for Codex CLI
```

**5. Run agents — no real keys on the local machine:**

```bash
claude                      # Claude Code → gateway → Anthropic
codex                       # Codex CLI   → gateway → OpenAI
gh repo list                # gh CLI      → gateway → GitHub
ollama run llama3.2         # Ollama      → gateway → your Ollama server
```

---

## Quickstart — Remote with TLS

**1. Point a domain at your server** (A record to the host IP).

**2. Set env vars and start:**

```bash
export COCO_DOMAIN=gw.example.com
export COCO_ADMIN_TOKEN=$(openssl rand -hex 32)
export ANTHROPIC_API_KEY=sk-ant-...
# ... other real credentials ...

docker compose up -d --build
# Caddy auto-provisions a Let's Encrypt certificate for COCO_DOMAIN.
```

Caddy listens on 443 and reverse-proxies to the gateway on 8080. The gateway stays HTTP-only behind it.

**3. Update your coco config:**

```toml
# ~/.config/coco/config.toml
gateway_url = "https://gw.example.com"
admin_token = "<COCO_ADMIN_TOKEN>"

[tokens]
laptop    = "ccgw_3a9f..."
ci-runner = "ccgw_ab12..."
```

**4. Health check:**

```bash
curl https://gw.example.com/health
# {"status":"ok"}
```

---

## Admin API

All `/admin/*` routes require `Authorization: Bearer <COCO_ADMIN_TOKEN>`.

**Create a token:**
```bash
curl -s -X POST https://gw.example.com/admin/tokens \
  -H "Authorization: Bearer $COCO_ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"ci-runner","scope":["openai","anthropic"]}'
```

**List tokens:**
```bash
curl -s https://gw.example.com/admin/tokens \
  -H "Authorization: Bearer $COCO_ADMIN_TOKEN" | jq .
```

**Revoke a token:**
```bash
curl -s -X DELETE https://gw.example.com/admin/tokens/<id> \
  -H "Authorization: Bearer $COCO_ADMIN_TOKEN"
```

Token values are hashed (blake3) in `/data/tokens.json` and never returned after creation.

---

## coco CLI

### Building coco

```bash
cargo build --release -p coco-cli
cp target/release/coco /usr/local/bin/coco
```

### Token management

```bash
# Create a token
coco token create --name laptop --scope anthropic,openai,api

# List tokens
coco token ls

# Revoke by name
coco token revoke laptop
```

### Shell activation

`coco env <name>` emits `export` statements for every tool the gateway supports:

```bash
eval $(coco env laptop)
```

This sets:

| Variable | Value |
|---|---|
| `ANTHROPIC_BASE_URL` | `https://gw.example.com/anthropic` |
| `ANTHROPIC_API_KEY` | phantom token |
| `OPENAI_BASE_URL` | `https://gw.example.com/openai` |
| `OPENAI_API_KEY` | phantom token |
| `GH_HOST` | `gw.example.com` |
| `GH_TOKEN` | phantom token |
| `OLLAMA_HOST` | `https://gw.example.com/ollama` |

`--codex` additionally writes `~/.codex/config.toml` (Codex reads its own config, not `OPENAI_BASE_URL`):

```bash
eval $(coco env laptop --codex)
```

---

## Profile Library

Eight named service profiles ship in `profiles/`. The deploy-time `profile.json` composes whichever you need.

| File | Upstream | inject_mode | Notes |
|---|---|---|---|
| `anthropic.json` | api.anthropic.com | header | `x-api-key` or `Authorization: Bearer` (OAuth token detection) |
| `openai.json` | api.openai.com | header | `Authorization: Bearer` |
| `github.json` | api.github.com | header | `Authorization: Bearer`; `api` route strips `/v3` prefix for `GH_HOST` |
| `groq.json` | api.groq.com | header | OpenAI-compatible |
| `elevenlabs.json` | api.elevenlabs.io | header | `xi-api-key` header |
| `ollama.json` | configurable | header | `OLLAMA_HOST=https://gw.example.com/ollama` |
| `telegram.json` | api.telegram.org | url_path | Token injected into URL: `/bot{credential}/...` |
| `together.json` | api.together.xyz | header | OpenAI-compatible |

`examples/profile.json` ships with anthropic + openai + github + httpbin as the starter set.

---

## Inject Modes

The `inject_mode` field on a route controls where the credential is placed:

| Mode | Behavior | Use case |
|---|---|---|
| `header` (default) | Inject into a request header | LLM APIs, GitHub, most REST APIs |
| `url_path` | Replace `{credential}` placeholder in upstream URL path | Telegram Bot API |
| `query_param` | Append as a query parameter | APIs requiring key in URL |

The gateway logs a warning at startup if a `url_path` route has no `{credential}` placeholder in its upstream URL.

---

## How It Works

```
Agent (Claude Code / SDK / gh / curl)
  phantom token in auth header      ──▶  coco-gateway
                                            │
                                    validate token (constant-time hash)
                                    check scope (if token has routes list)
                                            │
                                    match /<prefix>/ → upstream config
                                            │
                                    strip phantom, inject real credential
                                      inject_mode: header   → set header
                                      inject_mode: url_path → rewrite URL
                                            │
                                            ▼
                                    api.anthropic.com / api.openai.com / ... (TLS)
```

Accepted phantom token locations (checked in order):
1. `Proxy-Authorization: Bearer <token>` — explicit proxy credential
2. Route's own auth header (`x-api-key`, `Authorization: Bearer`, etc.) — used by SDK clients

Response codes:
- `200` — success
- `407` — missing or wrong phantom token
- `403` — token valid but route not in its scope
- `404` — unknown route prefix
- `503` — upstream credential env var absent

**Legacy fallback:** `COCO_PHANTOM_TOKEN` works as a fallback when registry lookup fails — backwards compatible with existing deployments.

---

## Custom Profiles

Routes are defined in `examples/profile.json` (or a custom path via `COCO_PROFILE`).

**Single-source route:**

```json
{
  "routes": {
    "openai": {
      "upstream": "https://api.openai.com",
      "credential_env": "OPENAI_API_KEY"
    }
  }
}
```

**Multi-source route** (ordered fallback — first matching source wins):

```json
{
  "routes": {
    "anthropic": {
      "upstream": "https://api.anthropic.com",
      "credential_sources": [
        {"env": "ANTHROPIC_API_KEY", "inject_header": "Authorization", "format": "Bearer {}", "prefix": "sk-ant-oat"},
        {"env": "ANTHROPIC_API_KEY", "inject_header": "x-api-key",     "format": "{}"}
      ]
    }
  }
}
```

**Route fields:**

| Field | Required | Default | Description |
|---|---|---|---|
| `upstream` | yes | — | HTTPS upstream base URL |
| `inject_mode` | no | `"header"` | `"header"`, `"url_path"`, or `"query_param"` |
| `strip_prefix` | no | — | Strip this path prefix before forwarding (e.g. `/v3` for `GH_HOST`) |
| `credential_env` | if no `credential_sources` | — | Env var holding the real credential |
| `inject_header` | no | `Authorization` | Header to inject into (single-source) |
| `credential_format` | no | `Bearer {}` | Format string; `{}` is replaced with the credential |
| `credential_sources` | no | — | Ordered list of credential sources |

**`credential_sources` entry fields:**

| Field | Required | Default | Description |
|---|---|---|---|
| `env` | yes | — | Env var name |
| `inject_header` | yes | — | Header to inject into |
| `format` | no | `Bearer {}` | Format string |
| `prefix` | no | — | Only match when the credential value starts with this string |

---

## Configuration

| Env var | Required | Default | Description |
|---|---|---|---|
| `COCO_ADMIN_TOKEN` | **yes** | — | Admin API secret. Gateway refuses to start without it. |
| `COCO_LISTEN_PORT` | no | `8080` | Port to bind |
| `COCO_PROFILE` | no | `/etc/coco/profile.json` | Profile file path |
| `COCO_TOKENS_FILE` | no | `/data/tokens.json` | Token registry file path |
| `COCO_PHANTOM_TOKEN` | no | — | Legacy single-token fallback (backwards compat) |
| `COCO_DOMAIN` | no | — | Domain for Caddy TLS termination |
| `ANTHROPIC_API_KEY` | — | — | Real Anthropic API key or OAuth token |
| `OPENAI_API_KEY` | — | — | Real OpenAI key |
| `GITHUB_TOKEN` | — | — | Real GitHub token |
| `TELEGRAM_BOT_TOKEN` | — | — | Telegram bot token |
| `HTTPBIN_TOKEN` | — | — | Any string (smoke tests) |

---

## Testing

**Unit + integration tests** (no running gateway needed):

```bash
cargo test --workspace
# 33 tests: auth, proxy, registry, admin API, inject modes, scope enforcement
```

**Live e2e tests** (starts gateway via docker compose, tears down on exit):

```bash
export COCO_ADMIN_TOKEN=test-admin
export HTTPBIN_TOKEN=anything
./scripts/test-e2e.sh

# With real Anthropic API key:
export ANTHROPIC_API_KEY=sk-ant-api-...
./scripts/test-e2e.sh

# With Claude Code OAuth token:
export ANTHROPIC_API_KEY=sk-ant-oat01-...
COCO_TEST_ANTHROPIC_MODE=oauth ./scripts/test-e2e.sh

# With real OpenAI key:
export OPENAI_API_KEY=sk-...
./scripts/test-e2e.sh
```

Credentials not set are skipped (`SKIP`, not `FAIL`).

**Smoke test without real credentials:**

```bash
# Start the gateway, then:
TOKEN=$(curl -s -X POST http://localhost:8080/admin/tokens \
  -H "Authorization: Bearer $COCO_ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"smoke"}' | jq -r .token)

curl http://localhost:8080/httpbin/bearer \
  -H "Proxy-Authorization: Bearer $TOKEN"
```

---

## Milestones

**Phase 1a — done.** Plain proxy on any Docker host. Phantom token auth, profile-based routing, multi-source credential injection.

**Phase 1c — done.** Remote deploy with TLS (Caddy), named token registry with admin API, profile library (8 services), inject modes (header/url_path/query_param), local `coco` CLI with `env` and `token` subcommands.

**Phase 1b — next.** CVM attestation — promote to Phala Cloud TDX CVM, add `GET /attest` (TDX QuoteV4), GHCR image build.

**Phase 2+.** Encrypted token store, per-token policy, audit log, sealed credential store, full CLI polish.

See [`docs/task.md`](./docs/task.md) for the full task list and progress.

---

## References

- [`docs/USING.md`](./docs/USING.md) — copy-paste setup for Claude Code, Codex, gh CLI, Ollama, Telegram
- [`docs/product.md`](./docs/product.md) — product vision
- [`docs/task.md`](./docs/task.md) — task list and progress
- [`docs/TEE-SECURITY.md`](./docs/TEE-SECURITY.md) — TEE security model
- [Phala Cloud](https://phala.network) — TDX CVM deployment platform
