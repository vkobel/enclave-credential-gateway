# Using Enclave Credential Gateway — Copy-Paste Setup Guide

This guide assumes you have a running gateway (local or remote). If not, see the [README quick start](../README.md#quick-start).

---

## Prerequisites

**1. Build the gate CLI:**

```bash
cargo build --release -p gate-cli
cp target/release/gate /usr/local/bin/
```

**2. Write the config file:**

Copy [`docs/config.example.toml`](config.example.toml) to `~/.config/gate/config.toml` and fill in your values.

```toml
# ~/.config/gate/config.toml (minimal)
gateway_url = "https://gw.example.com"
admin_token = "..."   # only needed for `gate admin ...` commands
e2e = false           # set to true and add [attestation] for production
```

**3. Create or add a phantom token:**

Scope values are route prefixes from your profile. Use `--all-routes` to create an unrestricted token that allows all current and future routes.

```bash
# Gateway admin operation: requires admin_token in config or GATE_ADMIN_TOKEN.
gate admin token create --name laptop --scope github,anthropic,openai
# The CLI saves the returned token to ~/.config/gate/config.toml.

# Or directly via curl
curl -s -X POST https://gw.example.com/admin/tokens \
  -H "Authorization: Bearer $GATE_ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"laptop","scope":["github","anthropic","openai"]}' | jq -r .token
```

**4. Add the token to config:**

```toml
# ~/.config/gate/config.toml
[tokens.laptop]
token = "gate_..."
scope = ["github", "anthropic", "openai"]
all_routes = false
```

`gate admin token ...` manages tokens on the gateway. `gate activate ...` uses a locally saved phantom token to configure your shell and tool-specific files; it does not require the admin token.

> Built-in routes: `github`, `anthropic`, `openai`. Tool adapters: `gh`, `codex`, `claude-code`. Routes and adapters live in the embedded `profiles/routes` and `profiles/tools` directories. GitHub owns an `api` compatibility alias for `gh`; `/api/v3/...` scopes as `github`.

---

## One-command activation

```bash
gate activate laptop
```

This opens an activated subshell. Enclave Credential Gateway prints a short banner with exported env vars and generated config files; type `exit` to leave.

For scripts that need to mutate the current shell, use `--eval`:

```bash
eval "$(gate activate laptop --eval --tool gh)"
```

Use `--describe` to inspect the activation without applying it.

---

## Claude Code (Experimental)

### With gate (recommended)

```bash
gate activate laptop --tool claude-code
claude
```

### Generic shell env

```bash
eval "$(gate activate laptop --eval --tool claude-code)"
claude
```

### Manual env vars

```bash
export ANTHROPIC_BASE_URL=https://gw.example.com/anthropic
export ANTHROPIC_API_KEY=gate_...    # phantom token — gateway swaps in the real credential
claude
```

The gateway process must have the real Anthropic credential in `ANTHROPIC_API_KEY`.
The Claude Code client shell also uses `ANTHROPIC_API_KEY`, but there it must be
the `gate_...` phantom token. Keep those environments separate: do not start or
restart the gateway from a shell where `ANTHROPIC_API_KEY` has already been
changed to the client phantom token.

### What happens

Claude Code sends `x-api-key: <phantom>` (or `Authorization: Bearer <phantom>` for OAuth sessions). The gateway validates the phantom, strips it, and injects the real upstream Anthropic credential in the correct header before forwarding to `api.anthropic.com`.

For Anthropic OAuth tokens (`sk-ant-oat...`) the gateway injects `Authorization: Bearer <token>` and adds `anthropic-beta: oauth-2025-04-20`. For regular API keys it injects `x-api-key: <key>`. The detection is automatic based on the real upstream credential prefix, and `gate_...` phantom tokens are rejected as upstream credentials.

---

## OpenAI Python SDK / Codex CLI

### With gate (recommended)

```bash
gate activate laptop --tool codex
codex
```

### Manual Codex setup

```bash
mkdir -p ~/.codex
cat > ~/.codex/config.toml <<'EOF'
model_provider = "openai"
openai_base_url = "https://gw.example.com/openai/v1"
EOF
cat > ~/.codex/auth.json <<'EOF'
{
  "auth_mode": "apikey",
  "OPENAI_API_KEY": "gate_..."
}
EOF
codex
```

Codex CLI requires its own config file for the gateway base URL and API-key login state. Its `openai_base_url` must include `/v1` because Codex appends endpoint paths like `/responses`. It does not use `OPENAI_BASE_URL` as the runtime endpoint, and `codex login --with-api-key` only writes API-key auth state; it does not configure the gateway URL. `gate activate <token> --tool codex` writes generated Codex config under `~/.config/gate/generated/codex/<token>/home` and exports `CODEX_HOME` inside the activated shell.

### Python SDK

```python
from openai import OpenAI

client = OpenAI(
    base_url="https://gw.example.com/openai/v1",
    api_key="gate_...",   # phantom token
)
response = client.chat.completions.create(
    model="gpt-4o-mini",
    messages=[{"role": "user", "content": "ping"}],
)
```

---

## GitHub CLI (`gh`)

### With gate (recommended)

```bash
gate activate laptop --tool gh
gh repo list
```

### Manual env vars

```bash
export GH_HOST=gw.example.com
export GH_ENTERPRISE_TOKEN=gate_...
export GH_TOKEN=$GH_ENTERPRISE_TOKEN   # optional alias for curl/examples
gh repo list
```

`GH_HOST` tells `gh` to route all API requests through the gateway instead of directly to `api.github.com`. `gh` appends `/api/v3/` to any custom host; GitHub's built-in `api` compatibility alias strips `/v3` before forwarding to `api.github.com` with the real `GITHUB_TOKEN`.

`gh repo clone` shells out to `git`, which authenticates the smart-HTTP transport with HTTP Basic auth. The gateway recognises requests of the form `/<owner>/<repo>.git/{info/refs,git-upload-pack,git-receive-pack}` and proxies them to `github.com` (the git host, not the API host). Tokens scoped to `github` cover both endpoints — no extra scope is needed.

`gate activate --tool gh` also exports `GIT_CONFIG_GLOBAL` to a generated Git config under `~/.config/gate/generated/gh/<token>/gitconfig`. That file includes your normal `~/.gitconfig`, resets inherited credential helpers for the gateway URL, and adds the `gate git-credential` helper. In the activated shell, plain `git fetch`, `git pull`, and `git push` against gateway remotes authenticate automatically without embedding the token in `.git/config` or the remote URL. For current-shell activation, run `eval "$(gate activate <token> --eval --tool gh)"`.

> **Note:** `GH_HOST` is a hostname, not a full URL. `gh` treats any `GH_HOST` other than `github.com` as a GitHub Enterprise host and reads `GH_ENTERPRISE_TOKEN` (not `GH_TOKEN`). `gate activate --tool gh` exports both so `gh` works for the gateway host and `GH_TOKEN` stays available for curl/manual examples.

---

## Service credentials

Registered credentials are RAM-only — after a gateway restart they must be re-registered, and phantom tokens with explicit `--cred` bindings hard-fail with 503 until the named credential is re-registered.

```bash
# Register a credential (value read from stdin — avoids shell history)
echo "sk-..." | gate admin creds register openai
# Or pass inline (stored in shell history)
gate admin creds register openai sk-...
# With an explicit name (defaults to <service> if omitted)
gate admin creds register openai --name openai-prod
echo "sk-..." | gate admin creds register openai --name openai-prod

# List registered credentials
gate admin creds ls

# Remove a credential
gate admin creds rm openai-prod
```

When creating a token, bind a route to a named credential so it uses that credential instead of the gateway-wide env var:

```bash
gate admin token create --name agent --scope openai --cred openai=openai-prod
```

Direct curl equivalents:

```bash
# Register
curl -s -X POST https://gw.example.com/admin/creds \
  -H "Authorization: Bearer $GATE_ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"openai-prod","service":"openai","value":"sk-..."}'

# List
curl -s https://gw.example.com/admin/creds \
  -H "Authorization: Bearer $GATE_ADMIN_TOKEN" | jq

# Remove
curl -s -X DELETE https://gw.example.com/admin/creds/openai-prod \
  -H "Authorization: Bearer $GATE_ADMIN_TOKEN"
```

---

## Admin channel (steve E2E encryption)

Admin commands (`gate admin creds ...`, `gate admin token ...`) are sent over the steve-encrypted channel by default (`e2e = true`).

**Local dev / Docker Compose** — disable encryption:

```toml
# ~/.config/gate/config.toml
e2e = false
```

**Debug against a remote gateway** — use the encrypted channel with all-zero PCRs. The steve SDK does not enforce PCR values yet, so zeros pass validation and let you test the E2E path without a real attestation report:

```toml
# ~/.config/gate/config.toml
[attestation]
pcr0 = "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
pcr1 = "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
pcr2 = "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
```

**Production Caution enclave** — fill PCR values from `caution verify`:

```toml
# ~/.config/gate/config.toml
[attestation]
pcr0 = "..."   # 96 hex chars (SHA-384 / 48 bytes)
pcr1 = "..."
pcr2 = "..."
# base_url = "..."   # steve URL; defaults to gateway_url
```

---

## Verifying the gateway

```bash
# Health check (unauthenticated)
curl https://gw.example.com/health
# {"status":"ok"}

# Test scope enforcement: request a route not in the token's scope
curl https://gw.example.com/openai/v1/models \
  -H "Authorization: Bearer gate_..."
# 403 Forbidden — token scope doesn't include "openai"
```

---

## Revoking access

```bash
# By name (looks up ID from GET /admin/tokens)
gate admin token revoke laptop

# Direct curl
curl -X DELETE https://gw.example.com/admin/tokens/<id> \
  -H "Authorization: Bearer $GATE_ADMIN_TOKEN"
```

Revocation takes effect immediately. In-flight requests complete; all subsequent requests from that token return `407`.

---

## Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `407 Proxy Authentication Required` | Wrong or missing phantom token | Check `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` match your token value |
| `403 Forbidden` | Token doesn't have this route in its scope | Recreate token with correct `--scope`, or use `--all-routes` for all routes |
| `404 Not Found` | Unknown route prefix | Check the prefix matches a built-in route key in `profiles/routes` |
| `503 Service Unavailable` | Real credential env var missing on the gateway | Set the credential env var and restart |
| `e2e is enabled but no [attestation] section found` | `e2e` defaults to true, but no PCR values set | Add `e2e = false` for local dev; or add `[attestation]` with real PCRs for production; or use all-zero PCRs to test the encrypted channel without attestation |
| `gate activate` fails | Token not in config file | Add `[tokens.<name>]` with `token = "gate_..."` to `~/.config/gate/config.toml` |
| `GH_HOST` is wrong | Set to full URL instead of hostname | `GH_HOST` must be just the hostname (`gw.example.com`), not a URL |
| `gh` returns 407 despite `GH_TOKEN` being set | `gh` treats custom `GH_HOST` as Enterprise and ignores `GH_TOKEN` | Export `GH_ENTERPRISE_TOKEN` (or run `eval "$(gate activate <name> --eval --tool gh)"` which sets both) |
