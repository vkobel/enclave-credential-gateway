# CoCo Credential Gateway

CoCo is a TEE-backed credential gateway for AI agents. Instead of giving agents real API keys, you give them **phantom tokens** (`ccgw_...`). The gateway validates the phantom inside a hardware trust boundary, injects the real credential into the live HTTP request, and forwards it upstream. The real key never leaves the enclave.

**The core insight: credentials are infrastructure, not agent state.**

---

## Why CoCo

Every AI tool you run today holds your real API keys: in `.env` files, shell exports, CI secrets, config files scattered across machines. You have no audit trail, no revocation, and rotating one leaked key means finding it in seven places.

|  | Local proxy | CoCo (TEE) |
|---|---|---|
| Agent can't read the key | ✅ | ✅ |
| Operator can't read the key | ❌ host access = full access | ✅ hardware enclave boundary |
| Works from any device or CI | ❌ local only | ✅ network-accessible |
| One credential change updates all agents | ❌ restart every proxy | ✅ gateway is the source of truth |
| Cryptographically verifiable binary | ❌ | ✅ TDX attestation + MRTD |

A local proxy protects credentials from the agent process. CoCo protects them from everyone — including the infrastructure operator — because the real key exists only inside a hardware enclave (Intel TDX). The enclave is measured at boot; any independent party can verify that the published source code is what's actually running.

CoCo is to AI agents what a hardware password manager is to browsers — except the credentials never leave the device even to fill a form, because CoCo fills the form itself.

---

## Current State

Phases **1a** and **1c** are complete. The remote proxy and CLI work today:

- **Phantom token registry** — named tokens, Blake3-hashed at rest, admin API (`POST/GET/DELETE /admin/tokens`), scope enforcement per token
- **Route profiles** — OpenAI, Anthropic, GitHub, Groq, ElevenLabs, Telegram, Together, Ollama; credential injection in headers or URL path
- **CLI** — `coco token create/revoke/ls`, shell activation for Claude Code, Codex, `gh`
- **Deployment** — Docker Compose + Caddy TLS; `COCO_DOMAIN` for a real hostname + Let's Encrypt

**Not yet implemented:** TDX attestation (`GET /attest`), encrypted credential store, audit log. See [spec/roadmap.md](./spec/roadmap.md) for what's next and why.

---

## Quick Start

Prerequisites: Docker + Compose, Rust/Cargo (for the CLI), `gh`, `git`, `curl`, `jq`.

### 1. Start the Gateway

```bash
export COCO_ADMIN_TOKEN="$(openssl rand -hex 32)"
export GITHUB_TOKEN=ghp_...       # real upstream token, stays server-side
export ANTHROPIC_API_KEY=sk-ant-...
export OPENAI_API_KEY=sk-...

docker compose up -d --build
curl -k https://localhost/health
```

`COCO_ADMIN_TOKEN` protects the admin API. `GITHUB_TOKEN` and other real keys are injected server-side; clients never receive them.

### 2. Trust Caddy's Local Certificate

```bash
docker compose cp caddy:/data/caddy/pki/authorities/local/root.crt /tmp/coco-caddy-root.crt
```

**macOS:**
```bash
sudo security add-trusted-cert -d -r trustRoot \
  -k /Library/Keychains/System.keychain /tmp/coco-caddy-root.crt
```

**Debian/Ubuntu:**
```bash
sudo cp /tmp/coco-caddy-root.crt /usr/local/share/ca-certificates/coco-caddy-root.crt
sudo update-ca-certificates
```

Then verify without `-k`:
```bash
curl https://localhost/health
```

For a real hostname: set `COCO_DOMAIN=gw.example.com` before `docker compose up`. Caddy requests a public certificate automatically.

### 3. Install the CLI

```bash
cargo build --release -p coco-cli
export PATH="$PWD/target/release:$PATH"

mkdir -p ~/.config/coco
cat > ~/.config/coco/config.toml <<EOF
gateway_url = "https://localhost"
admin_token = "$COCO_ADMIN_TOKEN"
EOF
```

---

## Usage Examples

### GitHub CLI and Git

```bash
coco token create --name gh-local --scope github
coco activate gh-local --tool gh

gh api user
gh repo list
gh repo clone OWNER/REPO
```

Activation sets `GH_HOST`, `GH_ENTERPRISE_TOKEN`, and a generated Git credential helper. Git remotes stay token-free (`https://localhost/OWNER/REPO.git`).

### Claude Code

```bash
coco token create --name claude-local --scope anthropic
coco activate claude-local --tool claude-code
claude
```

### Codex

```bash
coco token create --name codex-local --scope openai
coco activate codex-local --tool codex
codex
```

### Using an Existing Gateway

If someone else runs the gateway, you only need the URL and a token:

```toml
# ~/.config/coco/config.toml
gateway_url = "https://gw.example.com"

[tokens.laptop]
token = "ccgw_..."
scope = ["github", "openai", "anthropic"]
```

```bash
coco activate laptop --tool gh
```

---

## Routes

Built-in routes are in [`profiles/routes/`](./profiles/routes), tool adapters in [`profiles/tools/`](./profiles/tools), embedded at build time.

| Path prefix | Scope | Upstream | Credential env |
|---|---|---|---|
| `/openai/...` | `openai` | `https://api.openai.com` | `OPENAI_API_KEY` |
| `/anthropic/...` | `anthropic` | `https://api.anthropic.com` | `ANTHROPIC_API_KEY` |
| `/github/...` | `github` | `https://api.github.com` | `GITHUB_TOKEN` |
| `/api/v3/...` | `github` | `https://api.github.com` | `GITHUB_TOKEN` (`gh` compatibility, strips `/v3`) |
| `/<owner>/<repo>.git/...` | `github` | `https://github.com` | `GITHUB_TOKEN` (Git smart-HTTP) |
| `/groq/...` | `groq` | `https://api.groq.com` | `GROQ_API_KEY` |
| `/elevenlabs/...` | `elevenlabs` | `https://api.elevenlabs.io` | `ELEVENLABS_API_KEY` |
| `/telegram/...` | `telegram` | `https://api.telegram.org` | `TELEGRAM_BOT_TOKEN` (URL-path injection) |

Status codes: `407` = missing/invalid token · `403` = valid token, wrong scope · `404` = unknown route · `503` = real credential env var not set.

---

## How It Works

```
client sends ccgw_... phantom
    → gateway validates token (constant-time Blake3 hash compare)
    → gateway checks scope (403 before credential is even touched)
    → gateway removes the phantom from the request
    → gateway injects the real server-side credential
    → upstream receives only the real credential
```

The credential injection happens inside a hardware boundary (Intel TDX, on Phala Cloud). The binary is reproducible: anyone can rebuild from source and verify the MRTD matches the running enclave. See [docs/TEE-SECURITY.md](./docs/TEE-SECURITY.md).

---

## Testing

```bash
cargo fmt --check
cargo test --workspace
./scripts/test-e2e.sh
```

`test-e2e.sh` exercises the full Docker flow: gateway startup, admin API, token validation, scope enforcement, revocation, CLI activation. Live upstream checks are skipped when credentials are not set.

---

## Docs

- [spec/vision.md](./spec/vision.md) — product vision and roadmap
- [spec/roadmap.md](./spec/roadmap.md) — implementation phases and current status
- [docs/USING.md](./docs/USING.md) — detailed per-tool setup (Claude Code, Codex, `gh`)
- [docs/TEE-SECURITY.md](./docs/TEE-SECURITY.md) — TEE threat model and attestation design
- [profiles/README.md](./profiles/README.md) — route and tool profile format
