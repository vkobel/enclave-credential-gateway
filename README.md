# CoCo Credential Gateway

CoCo is a credential proxy for AI agents and developer tools. Clients send a scoped phantom token to the gateway; the gateway validates it, checks route scope, strips the phantom, and injects the real upstream credential server-side.

Use it when you want Claude Code, Codex, OpenAI-compatible SDKs, `gh`, or other tools to call vendor APIs without putting real API keys on the agent host.

## Quickstart

```bash
# Build the CLI.
cargo build --release -p coco-cli
cp target/release/coco /usr/local/bin/coco

# Start a local gateway.
export COCO_ADMIN_TOKEN=$(openssl rand -hex 32)
export HTTPBIN_TOKEN=anything
export GITHUB_TOKEN=ghp_...        # optional
export OPENAI_API_KEY=sk-...       # optional
export ANTHROPIC_API_KEY=sk-ant-... # optional
docker compose up -d --build
```

Create `~/.config/coco/config.toml`:

```toml
gateway_url = "http://localhost:8080"
admin_token = "<COCO_ADMIN_TOKEN>"
```

Create and use a scoped phantom token:

```bash
coco token create --name laptop --scope github,httpbin
eval $(coco env laptop)

curl http://localhost:8080/httpbin/bearer \
  -H "Authorization: Bearer $GH_TOKEN"
```

For remote TLS, set `COCO_DOMAIN=gw.example.com` before `docker compose up -d --build`; Caddy terminates HTTPS and proxies to the gateway.

## Route Definitions

The built-in routes are defined in [profiles/routes.json](./profiles/routes.json). That file is embedded into the gateway binary at build time. The compose example also mounts [examples/profile.json](./examples/profile.json) as a runtime profile; set `COCO_PROFILE=/path/to/profile.json` to use another profile.

Token scopes use the canonical route key. The `/api/v3/...` GitHub compatibility route is named `api` in paths, but scopes as `github`.

| Path prefix | Scope | Upstream | Credential env | Injection |
|---|---|---|---|---|
| `/openai/...` | `openai` | `https://api.openai.com` | `OPENAI_API_KEY` | `Authorization: Bearer ...` |
| `/anthropic/...` | `anthropic` | `https://api.anthropic.com` | `ANTHROPIC_API_KEY` | OAuth tokens as `Authorization: Bearer ...`, API keys as `x-api-key` |
| `/github/...` | `github` | `https://api.github.com` | `GITHUB_TOKEN` | `Authorization: Bearer ...` |
| `/api/v3/...` | `github` | `https://api.github.com` | `GITHUB_TOKEN` | `gh` compatibility alias; strips `/v3` |
| `/httpbin/...` | `httpbin` | `https://httpbin.org` | `HTTPBIN_TOKEN` | `Authorization: Bearer ...` |
| `/ollama/...` | `ollama` | `https://127.0.0.1:11434` | `OLLAMA_API_KEY` | `Authorization: Bearer ...` |
| `/telegram/...` | `telegram` | `https://api.telegram.org` | `TELEGRAM_BOT_TOKEN` | URL path token injection |
| `/groq/...` | `groq` | `https://api.groq.com` | `GROQ_API_KEY` | `Authorization: Bearer ...` |
| `/together/...` | `together` | `https://api.together.xyz` | `TOGETHER_API_KEY` | `Authorization: Bearer ...` |
| `/elevenlabs/...` | `elevenlabs` | `https://api.elevenlabs.io` | `ELEVENLABS_API_KEY` | `xi-api-key` |

Profile fields are intentionally small: `upstream`, `credential_sources` or `credential_env`, `inject_mode`, optional `canonical`, optional `strip_prefix`, optional `url_path_prefix`, and optional `inject_param`.

## CLI

```bash
# Token registry.
coco token create --name laptop --scope openai,anthropic,github
coco token ls
coco token revoke laptop

# Generic shell activation.
eval $(coco env laptop)

# Tool-specific adapters.
eval $(coco tool env gh laptop)
coco tool install codex laptop
eval $(coco tool env opencode laptop)
```

`coco env <name> --codex` remains a quiet compatibility path: it writes `~/.codex/config.toml` only when the token can access `openai`; otherwise it leaves Codex config untouched. Prefer `coco tool install codex <name>` in new scripts.

For copy-paste setup per client, use [docs/USING.md](./docs/USING.md).

## How It Works

```text
client sends phantom token -> gateway validates token and route scope
                           -> gateway selects /<route>/ profile
                           -> gateway removes phantom credential
                           -> gateway injects real server-side credential
                           -> upstream API receives only the real credential
```

Status codes:

| Code | Meaning |
|---|---|
| `200` | Upstream request succeeded |
| `407` | Missing, revoked, or invalid phantom token |
| `403` | Token is valid but not scoped for this route |
| `404` | Unknown route prefix |
| `503` | Route exists but required real credential env var is missing |

`COCO_PHANTOM_TOKEN` is still supported as a legacy single-token fallback, but the registry/admin API path is preferred.

## Configuration

Gateway environment:

| Env var | Required | Default | Purpose |
|---|---:|---|---|
| `COCO_ADMIN_TOKEN` | yes | none | Admin API bearer token |
| `COCO_LISTEN_PORT` | no | `8080` | Gateway listen port |
| `COCO_PROFILE` | no | embedded routes or `/etc/coco/profile.json` | Route profile path |
| `COCO_TOKENS_FILE` | no | `/data/tokens.json` in compose | Registry storage |
| `COCO_PHANTOM_TOKEN` | no | none | Legacy single-token fallback |
| `COCO_DOMAIN` | no | none | Domain for Caddy TLS |

CLI config:

```toml
gateway_url = "https://gw.example.com"
admin_token = "<COCO_ADMIN_TOKEN>"

[tokens.laptop]
token = "ccgw_..."
scope = ["openai", "github"]
```

## Testing

Fast local checks:

```bash
cargo fmt --check
cargo test --workspace
```

`cargo test --workspace` runs CLI tests plus gateway auth, proxy, registry, admin API, inject-mode, and scope tests. Live gateway feature tests are ignored unless explicitly enabled.

Compose-backed e2e:

```bash
# Minimal run. COCO_ADMIN_TOKEN defaults to test-admin if omitted.
export COCO_ADMIN_TOKEN=test-admin
export HTTPBIN_TOKEN=anything
./scripts/test-e2e.sh
```

The e2e script starts docker compose, creates registry tokens through the admin API, validates missing/wrong token handling, scope denial, revocation, CLI Codex compatibility, and httpbin credential injection. It tears down the compose project on exit. If port `8080` is occupied by an incompatible local gateway, it falls back to `18080`; override with `COCO_E2E_PORT=<port>`.

Optional live upstream checks:

```bash
export OPENAI_API_KEY=sk-...
./scripts/test-e2e.sh

export ANTHROPIC_API_KEY=sk-ant-api-...
./scripts/test-e2e.sh

export ANTHROPIC_API_KEY=sk-ant-oat01-...
COCO_TEST_ANTHROPIC_MODE=oauth ./scripts/test-e2e.sh
```

Missing real upstream credentials are skipped, not treated as failures.

Run the ignored live-gateway Rust tests against an already running gateway:

```bash
export TEST_PHANTOM_TOKEN=ccgw_...
cargo test -p coco-gateway --features integration
```

## Docs

- [docs/USING.md](./docs/USING.md): per-tool setup for Claude Code, Codex, `gh`, OpenCode, Ollama, Telegram, and SDKs.
- [docs/TEE-SECURITY.md](./docs/TEE-SECURITY.md): TEE and deployment security notes.
- [docs/product.md](./docs/product.md): product framing.
- [docs/task.md](./docs/task.md): project task history.
