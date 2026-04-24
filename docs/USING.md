# Using CoCo — Copy-Paste Setup Guide

This guide assumes you have a running gateway (local or remote). If not, see the [README quickstart](../README.md).

---

## Prerequisites

**1. Build the coco CLI:**

```bash
cargo build --release -p coco-cli
cp target/release/coco /usr/local/bin/
```

**2. Write the config file:**

```toml
# ~/.config/coco/config.toml
gateway_url = "https://gw.example.com"
admin_token = "..."   # only needed for token management commands
```

**3. Create a token:**

Scope values are route prefixes from your profile. Omit `--scope` to create an unrestricted token that allows all current and future routes.

```bash
# Via coco CLI (requires admin_token in config)
coco token create --name laptop --scope github,httpbin,anthropic,openai,ollama
# token: ccgw_... ← shown once; add it to config

# Or directly via curl
curl -s -X POST https://gw.example.com/admin/tokens \
  -H "Authorization: Bearer $COCO_ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"laptop","scope":["github","httpbin","anthropic","openai","ollama"]}' | jq -r .token
```

**4. Add the token to config:**

```toml
# ~/.config/coco/config.toml
[tokens]
laptop = "ccgw_..."
```

> Built-in route reference: `github`, `api`, `anthropic`, `openai`, `httpbin`, `ollama`, `telegram`, `groq`, `together`, `elevenlabs`. The built-in routes live in the embedded `profiles/routes.json` manifest. The `api` route is the GitHub CLI compatibility route and scopes as `github`.

---

## One-command activation

```bash
eval $(coco env laptop)
```

This sets the generic shell env vars in one shot:

```bash
eval $(coco env laptop)
```

For file-backed tools, use the dedicated tool adapters:

```bash
coco tool install codex laptop
eval $(coco tool env gh laptop)
eval $(coco tool env opencode laptop)
```

---

## Claude Code (Experimental)

### Render an experimental shell fragment

```bash
coco tool render claude-code laptop
```

### Generic shell env

```bash
eval $(coco env laptop)
claude
```

### Manual env vars

```bash
export ANTHROPIC_BASE_URL=https://gw.example.com/anthropic
export ANTHROPIC_API_KEY=ccgw_...    # phantom token — gateway swaps in the real credential
claude
```

### What happens

Claude Code sends `x-api-key: <phantom>` (or `Authorization: Bearer <phantom>` for OAuth sessions). The gateway validates the phantom, strips it, and injects the real `ANTHROPIC_API_KEY` in the correct header before forwarding to `api.anthropic.com`.

For Anthropic OAuth tokens (`sk-ant-oat...`) the gateway injects `Authorization: Bearer <token>`. For regular API keys it injects `x-api-key: <key>`. The detection is automatic based on the `sk-ant-oat` prefix.

---

## OpenAI Python SDK / Codex CLI

### With coco (recommended)

```bash
coco tool install codex laptop
codex
```

### Manual env vars

```bash
export OPENAI_BASE_URL=https://gw.example.com/openai
export OPENAI_API_KEY=ccgw_...
codex
```

Codex CLI requires its own config file (`~/.codex/config.toml`) in addition to env vars. `coco tool install codex <token>` writes it directly. `coco env --codex` is still accepted as a quiet compatibility alias: it writes the Codex config only when the token can access the `openai` route, and otherwise leaves the config untouched.

### Python SDK

```python
from openai import OpenAI

client = OpenAI(
    base_url="https://gw.example.com/openai",
    api_key="ccgw_...",   # phantom token
)
response = client.chat.completions.create(
    model="gpt-4o-mini",
    messages=[{"role": "user", "content": "ping"}],
)
```

---

## GitHub CLI (`gh`)

### With coco (recommended)

```bash
eval $(coco tool env gh laptop)
gh repo list
```

### Manual env vars

```bash
export GH_HOST=gw.example.com
export GH_ENTERPRISE_TOKEN=ccgw_...
export GH_TOKEN=$GH_ENTERPRISE_TOKEN   # optional alias for curl/examples
gh repo list
```

`GH_HOST` tells `gh` to route all API requests through the gateway instead of directly to `api.github.com`. `gh` appends `/api/v3/` to any custom host; the built-in `api` compatibility route strips `/v3` before forwarding to `api.github.com` with the real `GITHUB_TOKEN`.

> **Note:** `GH_HOST` is a hostname, not a full URL. `gh` treats any `GH_HOST` other than `github.com` as a GitHub Enterprise host and reads `GH_ENTERPRISE_TOKEN` (not `GH_TOKEN`). `coco env` exports both so `gh` works for the gateway host and `GH_TOKEN` stays available for curl/manual examples.

---

## Ollama

### With coco (recommended)

```bash
eval $(coco env laptop)
ollama run llama3.2
```

### Manual env var

```bash
export OLLAMA_HOST=https://gw.example.com/ollama
ollama run llama3.2
```

Requires `OLLAMA_HOST` to be set to the gateway's `/ollama` prefix. The `ollama.json` profile forwards requests to the upstream Ollama server configured via `OLLAMA_HOST` on the gateway side.

---

## OpenCode

### With coco (recommended)

```bash
eval $(coco tool env opencode laptop)
opencode
```

`coco tool env opencode <token>` materializes an OpenCode config under `~/.config/coco/generated/` and exports `OPENCODE_CONFIG` plus the in-scope API key env vars needed by that generated config.

---

## Telegram Bot

Telegram's Bot API embeds the token in the URL path (`/bot<TOKEN>/<method>`), so it can't use header injection. The gateway handles this with `inject_mode: url_path`.

### Shell / curl

```bash
curl "https://gw.example.com/telegram/getMe" \
  -H "Authorization: Bearer ccgw_..."
# gateway rewrites path to /bot<TELEGRAM_BOT_TOKEN>/getMe before forwarding
```

### Python (python-telegram-bot)

```python
from telegram.ext import ApplicationBuilder

app = (
    ApplicationBuilder()
    .token("ccgw_...")              # phantom token
    .base_url("https://gw.example.com/telegram/")
    .build()
)
```

---

## Verifying the gateway

```bash
# Health check (unauthenticated)
curl https://gw.example.com/health
# {"status":"ok"}

# Smoke test — no real upstream credential needed; httpbin echoes the token back
curl https://gw.example.com/httpbin/bearer \
  -H "Authorization: Bearer ccgw_..."
# {"authenticated": true, "token": "any-value"}

# Test scope enforcement: request a route not in the token's scope
curl https://gw.example.com/openai/v1/models \
  -H "Authorization: Bearer ccgw_..."
# 403 Forbidden — token scope doesn't include "openai"
```

---

## Revoking access

```bash
# By name (looks up ID from GET /admin/tokens)
coco token revoke laptop

# Direct curl
curl -X DELETE https://gw.example.com/admin/tokens/<id> \
  -H "Authorization: Bearer $COCO_ADMIN_TOKEN"
```

Revocation takes effect immediately. In-flight requests complete; all subsequent requests from that token return `407`.

---

## Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `407 Proxy Authentication Required` | Wrong or missing phantom token | Check `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` match your token value |
| `403 Forbidden` | Token doesn't have this route in its scope | Recreate token with correct `--scope`, or omit scope for all routes |
| `404 Not Found` | Unknown route prefix | Check the prefix matches a route key in `profile.json` |
| `503 Service Unavailable` | Real credential env var missing on the gateway | Set the credential env var and restart |
| `coco env` fails | Token not in config file | Add `[tokens] laptop = "ccgw_..."` to `~/.config/coco/config.toml` |
| `GH_HOST` is wrong | Set to full URL instead of hostname | `GH_HOST` must be just the hostname (`gw.example.com`), not a URL |
| `gh` returns 407 despite `GH_TOKEN` being set | `gh` treats custom `GH_HOST` as Enterprise and ignores `GH_TOKEN` | Export `GH_ENTERPRISE_TOKEN` (or run `eval $(coco env <name>)` which sets both) |
