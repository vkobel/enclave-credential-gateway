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

The built-in routes are defined in [profiles/routes.json](./profiles/routes.json). That file is embedded into the gateway binary at build time; runtime route overrides are intentionally not supported yet.

Token scopes use the top-level route key. Empty scope is unrestricted and allows all current and future routes. GitHub also owns an `/api/v3/...` compatibility alias for `gh`; it scopes as `github` and strips `/v3` before forwarding.

| Path prefix | Scope | Upstream | Credential env | Injection |
|---|---|---|---|---|
| `/openai/...` | `openai` | `https://api.openai.com` | `OPENAI_API_KEY` | `Authorization: Bearer ...` |
| `/anthropic/...` | `anthropic` | `https://api.anthropic.com` | `ANTHROPIC_API_KEY` | OAuth tokens as `Authorization: Bearer ...`, API keys as `x-api-key` |
| `/github/...` | `github` | `https://api.github.com` | `GITHUB_TOKEN` | `Authorization: Bearer ...` |
| `/api/v3/...` | `github` | `https://api.github.com` | `GITHUB_TOKEN` | `gh` compatibility route; strips route-relative `/v3` |
| `/httpbin/...` | `httpbin` | `https://httpbin.org` | `HTTPBIN_TOKEN` | `Authorization: Bearer ...` |
| `/ollama/...` | `ollama` | `https://127.0.0.1:11434` | `OLLAMA_API_KEY` | `Authorization: Bearer ...` |
| `/telegram/...` | `telegram` | `https://api.telegram.org` | `TELEGRAM_BOT_TOKEN` | URL path token injection |
| `/groq/...` | `groq` | `https://api.groq.com` | `GROQ_API_KEY` | `Authorization: Bearer ...` |
| `/together/...` | `together` | `https://api.together.xyz` | `TOGETHER_API_KEY` | `Authorization: Bearer ...` |
| `/elevenlabs/...` | `elevenlabs` | `https://api.elevenlabs.io` | `ELEVENLABS_API_KEY` | `xi-api-key` |

Route manifest syntax is route-first:

```json
{
  "routes": {
    "<route-key>": {
      "upstream": "https://api.example.com",
      "aliases": [
        { "prefix": "<compat-path-prefix>", "strip_prefix": "/optional/path/to/remove" }
      ],
      "credential_sources": [
        {
          "env": "REAL_VENDOR_TOKEN_ENV",
          "inject_header": "Authorization",
          "format": "Bearer {}",
          "prefix": "optional-token-prefix"
        }
      ],
      "inject_mode": "header"
    }
  }
}
```

The top-level route key is both the normal URL prefix and the token scope. For example, `github` handles `/github/...` and a token scoped to `github` can call it. Aliases are compatibility path prefixes owned by a top-level route; GitHub's `api` alias means `/api/v3/user` scopes as `github`, strips `/v3`, and forwards `/user` to `https://api.github.com`.

`credential_sources` are tried in order. The first env var that exists and matches its optional `prefix` is selected. In normal `header` mode, the gateway removes the client's phantom token header and injects the real credential into `inject_header` after applying `format`; `{}` is replaced with the env value. Most routes omit `inject_mode` because `header` is the default. `url_path` mode is only for APIs such as Telegram where the token is part of the path: `/telegram/sendMessage` becomes `https://api.telegram.org/bot<TOKEN>/sendMessage`; set `url_path_prefix` only with that mode.

Supported fields:

| Field | Purpose |
|---|---|
| `upstream` | Base upstream URL |
| `credential_sources` | Ordered env-backed credentials with `env`, `inject_header`, optional `format`, and optional `prefix` |
| `aliases` | Optional compatibility prefixes owned by this route, such as GitHub's `api` alias |
| `strip_prefix` | Alias path prefix to remove before forwarding |
| `inject_mode` | `header` or `url_path`; defaults to `header` |
| `url_path_prefix` | Prefix used by `url_path` credential injection |

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

Run the full local test suite before opening a PR:

```bash
cargo fmt --check
cargo test --workspace
./scripts/test-e2e.sh
```

What these do:

- `cargo fmt --check` verifies Rust formatting without changing files.
- `cargo test --workspace` runs CLI tests plus gateway auth, proxy, registry, admin API, profile parsing, inject-mode, and token-scope tests. Live gateway feature tests are ignored unless explicitly enabled.
- `./scripts/test-e2e.sh` runs the Docker-backed end-to-end flow: gateway startup, admin token creation, missing/wrong token handling, route-scope denial, revocation, CLI Codex compatibility, and optional upstream credential injection checks.

Run focused package tests while iterating:

```bash
cargo test -p coco-gateway
cargo test -p coco-cli
```

Compose-backed e2e:

```bash
# Minimal run. COCO_ADMIN_TOKEN defaults to test-admin if omitted.
./scripts/test-e2e.sh
```

The e2e script tears down the compose project on exit. If port `8080` is occupied by an incompatible local gateway, it falls back to `18080`; override with `COCO_E2E_PORT=<port>`.

Optional live upstream checks:

```bash
export HTTPBIN_TOKEN=anything
./scripts/test-e2e.sh

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
