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

Scope values are route prefixes from your profile. Omit `--scope` to allow all routes.

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

> Route key reference: `github`, `anthropic`, `openai`, `httpbin`, `ollama`, `telegram`. The `github` route also accepts `gh` CLI requests — `gh` appends `/api/v3/` to `GH_HOST`, and the route has a built-in alias that maps that prefix transparently.

---

## One-command activation

```bash
eval $(coco env laptop)
```

This sets all env vars for every supported tool in one shot. Add `--codex` to also configure Codex CLI:

```bash
eval $(coco env laptop --codex)
```

After this, proceed directly to any tool section below — no per-tool configuration needed.

---

## Claude Code

### With coco (recommended)

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
eval $(coco env laptop --codex)   # --codex writes ~/.codex/config.toml
codex
```

### Manual env vars

```bash
export OPENAI_BASE_URL=https://gw.example.com/openai
export OPENAI_API_KEY=ccgw_...
codex
```

Codex CLI requires its own config file (`~/.codex/config.toml`) in addition to env vars. The `--codex` flag handles this automatically.

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
eval $(coco env laptop)
gh repo list
```

### Manual env vars

```bash
export GH_HOST=gw.example.com
export GH_TOKEN=ccgw_...
gh repo list
```

`GH_HOST` tells `gh` to route all API requests through the gateway instead of directly to `api.github.com`. `gh` appends `/api/v3/` to any custom host; the `github` route has a built-in alias for that prefix and strips it before forwarding to `api.github.com` with the real `GITHUB_TOKEN`.

> **Note:** `GH_HOST` is a hostname, not a full URL. `coco env` sets it correctly.

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
