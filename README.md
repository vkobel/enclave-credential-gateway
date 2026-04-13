# CoCo Credential Gateway

A credential proxy for AI agents: agents authenticate with a phantom token, the gateway validates it and injects the real upstream API key, so secrets never touch the agent's host.

Built on [`nono-proxy`](./nono) — promoted into a remotely deployable, hardware-attested service.

---

## Quickstart

**1. Generate a phantom token** — the shared secret your agents use as their API key:

```bash
export COCO_PHANTOM_TOKEN=$(openssl rand -hex 32)
echo $COCO_PHANTOM_TOKEN   # save this, you'll pass it to agents
```

**2. Set your upstream credentials** and start the gateway:

```bash
export OPENAI_API_KEY=sk-...
export ANTHROPIC_API_KEY=sk-ant-...    # API key (sk-ant-api...) or Claude Code OAuth token (sk-ant-oat...)
export GITHUB_TOKEN=ghp_...
export HTTPBIN_TOKEN=any-value          # any string — used for smoke tests

docker compose up -d --build
```

Routes are loaded from [`examples/profile.json`](./examples/profile.json). Edit it to add or remove upstreams; restart with `docker compose up -d --build`.

**3. Call any upstream through the gateway** — real keys never leave the gateway:

```bash
# Smoke test — no real credential needed
curl http://localhost:8080/httpbin/bearer \
  -H "Proxy-Authorization: Bearer $COCO_PHANTOM_TOKEN"

# OpenAI
curl -X POST http://localhost:8080/openai/v1/chat/completions \
  -H "Proxy-Authorization: Bearer $COCO_PHANTOM_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-4o-mini","messages":[{"role":"user","content":"ping"}]}'

# Anthropic
curl -X POST http://localhost:8080/anthropic/v1/messages \
  -H "Proxy-Authorization: Bearer $COCO_PHANTOM_TOKEN" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -d '{"model":"claude-haiku-4-5-20251001","max_tokens":64,"messages":[{"role":"user","content":"ping"}]}'
```

The gateway returns `407` on a missing/wrong phantom token, `404` on an unknown prefix, `503` if the upstream credential env var is absent.

---

## Using Claude Code with no real credentials

Point Claude Code at the gateway and give it the phantom token as its "API key". The gateway injects the real Anthropic credential server-side — no key on the local machine.

The gateway detects the token shape at inject time:
- `sk-ant-oat...` (Claude Code OAuth token) → injected as `Authorization: Bearer <token>`
- anything else (regular API key) → injected as `x-api-key: <key>`

```bash
# 1. Start the gateway (accepts both API keys and Claude Code OAuth tokens)
export COCO_PHANTOM_TOKEN=my-secret
export ANTHROPIC_API_KEY=sk-ant-oat01-...   # or sk-ant-api... — gateway handles both
docker compose up -d --build

# 2. Point Claude Code at the gateway (no real credential here)
export ANTHROPIC_BASE_URL=http://localhost:8080/anthropic
export ANTHROPIC_API_KEY=my-secret     # phantom token — gateway swaps in the real credential

claude chat
```

Claude Code sends `x-api-key: my-secret` (or `Authorization: Bearer my-secret` when using an OAuth session). The gateway validates the phantom, strips it, and injects the real credential in the correct header before forwarding to `api.anthropic.com`.

---

## How It Works

The gateway accepts the phantom token from the **same header the SDK uses for a real credential** (following nono's pattern). This means SDKs work without modification.

```
Claude Code / SDK
  x-api-key: <phantom>           ──▶  coco-gateway
  (or Authorization: Bearer ...)          │
                                  validate token (constant-time)
                                          │
                                  match /<prefix>/ → upstream
                                          │
                                  strip phantom, inject real credential
                                    x-api-key: <real-key>
                                    (or Authorization: Bearer <key>)
                                          │
                                          ▼
                                  api.anthropic.com (TLS)
```

Accepted phantom token locations (checked in order):
1. `Proxy-Authorization: Bearer <token>` — generic, works with `curl` and test scripts
2. Route's own auth header (`x-api-key`, `Authorization: Bearer`, etc.) — used by SDK clients

**Known gap (POC):** Agents route through the gateway voluntarily via `BASE_URL`. A compromised agent can bypass by connecting directly upstream. Mitigate with egress firewall rules. Path C (nono fork + Landlock enforcement) closes this properly.

---

## Custom Profiles

Routes are defined in [`examples/profile.json`](./examples/profile.json) and mounted into the container. Edit that file to add any upstream.

**Single-source route** (one credential):

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

The `prefix` field lets a single env var route to different headers based on token shape: OAuth tokens (`sk-ant-oat...`) inject as `Authorization: Bearer`, regular API keys fall through to `x-api-key`.

**Single-source fields:**

| Field | Required | Default | Description |
|---|---|---|---|
| `upstream` | yes | — | HTTPS upstream base URL |
| `credential_env` | yes | — | Env var holding the real credential |
| `inject_header` | no | `Authorization` | Header to inject the credential into |
| `credential_format` | no | `Bearer {}` | Format string; `{}` replaced with the credential |

**`credential_sources` fields** (each entry):

| Field | Required | Default | Description |
|---|---|---|---|
| `env` | yes | — | Env var name |
| `inject_header` | yes | — | Header to inject into |
| `format` | no | `Bearer {}` | Format string |
| `prefix` | no | — | If set, this source only matches when the credential value starts with this string |

---

## Configuration

| Env var | Required | Default | Description |
|---|---|---|---|
| `COCO_PHANTOM_TOKEN` | yes | — | Shared secret agents use as their API key |
| `COCO_LISTEN_PORT` | no | `8080` | Port to bind |
| `COCO_PROFILE` | no | `/etc/coco/profile.json` | Profile file path |
| `OPENAI_API_KEY` | — | — | Real OpenAI key |
| `ANTHROPIC_API_KEY` | — | — | Real Anthropic API key |
| `GITHUB_TOKEN` | — | — | Real GitHub token |
| `HTTPBIN_TOKEN` | — | — | Any string (smoke tests) |

---

## Testing

**Unit + integration tests** (no running gateway needed):

```bash
cargo test
```

**Live e2e tests** (starts gateway via docker compose, tears down on exit):

```bash
export COCO_PHANTOM_TOKEN=test-phantom
export HTTPBIN_TOKEN=anything           # any value
./scripts/test-e2e.sh

# With real Anthropic API key (x-api-key path):
export ANTHROPIC_API_KEY=sk-ant-api-...
./scripts/test-e2e.sh

# With Claude Code OAuth token (Authorization: Bearer path):
export ANTHROPIC_API_KEY=sk-ant-oat01-...
COCO_TEST_ANTHROPIC_MODE=oauth ./scripts/test-e2e.sh

# With real OpenAI credential:
export OPENAI_API_KEY=sk-...
./scripts/test-e2e.sh
```

Credentials not set are skipped (shown as `SKIP` in the output, not `FAIL`).

---

## Milestones

**Phase 1a (current) — Plain infrastructure POC**
`coco-gateway` binary on any Docker host. Proves the proxy data plane end-to-end.

**Phase 1b — CVM attestation**
Promote to Phala Cloud TDX CVM. Add `GET /attest` (raw TDX QuoteV4 via `tappd` sidecar).

**Phase 2+ — Policy, identity, encrypted vaults, audit**
See [`openspec/`](./openspec) for specs and task lists.

---

## References

- [`nono/`](./nono) — nono-proxy (phantom token pattern, route store, credential injection)
- [`openspec/`](./openspec) — specs, design docs, task lists
- [Phala Cloud](https://phala.network) — TDX CVM deployment platform
