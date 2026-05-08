# Enclave Credential Gateway

**Status: alpha / early implementation.** Enclave Credential Gateway is early work toward a TEE-backed credential gateway for AI agents. The working gateway already lets clients use scoped phantom tokens (`gate_...`) instead of real vendor keys, but the TEE trust boundary, attestation flow, sealed credential storage, and reproducible release verification are not implemented yet.

**Core idea:** credentials should be infrastructure, not agent state.

---

## Vision

AI tools routinely hold real API keys in `.env` files, shell exports, CI secrets, and local config. That makes credentials easy to leak, hard to rotate, and difficult to audit.

Enclave Credential Gateway's target architecture is a network gateway running inside a hardware trust boundary. Agents receive revocable phantom tokens. The gateway validates the phantom, checks route scope, injects the real upstream credential server-side, and forwards the request. In the TEE-backed version, the infrastructure operator should be able to verify the binary without being able to read the real credentials.

| Capability | Local proxy | Enclave Credential Gateway target |
|---|---|---|
| Agent cannot read the real key | yes | yes |
| Central revocation and rotation | limited | yes |
| Works from other devices or CI | no | yes |
| Operator cannot read the real key | no | target: TDX boundary |
| Verifiable running binary | no | target: attestation + MRTD |

Verification is the key idea behind the TEE path. The `gate` CLI will have a normal verification mode that checks a gateway's attestation evidence before trusting it, and a heavier reproducibility mode that builds the same release locally, compares the resulting image and measurement material, and checks it against the live enclave registers reported by the server. The goal is for a technical user to verify both "this server is running inside the expected enclave" and "that enclave corresponds to source and release artifacts I can inspect."

---

## Working Today

This repo is an early work in progress. The proxy, token registry, and CLI are usable locally and through Docker Compose:

- **Gateway/proxy** - Axum HTTP gateway with route matching, auth middleware, credential stripping, and upstream credential injection.
- **Phantom token registry** - named tokens, Blake3-hashed at rest, persisted to `tokens.json`, with per-token route scope enforcement.
- **Admin API** - `POST /admin/tokens`, `GET /admin/tokens`, `DELETE /admin/tokens/:id`, protected by `GATE_ADMIN_TOKEN`.
- **CLI** - `gate admin token ...` for gateway administration, plus `gate activate` and `gate git-credential` for local tool setup.
- **Tool activation** - generated config/env for `gh`, Codex, and Claude Code.
- **Deployment scaffold** - Docker Compose with Caddy TLS and optional `GATE_DOMAIN`.

**Not implemented yet:** TDX attestation (`GET /attest`), reproducible build/MRTD verification, sealed credential storage, audit log, token expiry, and additional route profiles beyond OpenAI, Anthropic, and GitHub.

See [spec/roadmap.md](./spec/roadmap.md) for the implementation plan.

---

## Quick Start

Prerequisites: Docker + Compose, Rust/Cargo for the CLI, `gh`, `git`, `curl`, and `jq`.

### 1. Start the Gateway

```bash
export GATE_ADMIN_TOKEN="$(openssl rand -hex 32)"
export GITHUB_TOKEN=ghp_...       # real upstream token, stays server-side
export ANTHROPIC_API_KEY=sk-ant-...
export OPENAI_API_KEY=sk-...

docker compose up -d --build
curl -k https://localhost/health
```

`GATE_ADMIN_TOKEN` protects the admin API. Vendor keys are read by the gateway and injected server-side; clients receive only `gate_...` phantom tokens.

### 2. Trust Caddy's Local Certificate

```bash
docker compose cp caddy:/data/caddy/pki/authorities/local/root.crt /tmp/gate-caddy-root.crt
```

macOS:

```bash
sudo security add-trusted-cert -d -r trustRoot \
  -k /Library/Keychains/System.keychain /tmp/gate-caddy-root.crt
```

Debian/Ubuntu:

```bash
sudo cp /tmp/gate-caddy-root.crt /usr/local/share/ca-certificates/gate-caddy-root.crt
sudo update-ca-certificates
```

Then verify without `-k`:

```bash
curl https://localhost/health
```

For a real hostname, set `GATE_DOMAIN=gw.example.com` before `docker compose up`. Caddy will request a public certificate for that name.

### 3. Install the CLI

```bash
cargo build --release -p gate-cli
export PATH="$PWD/target/release:$PATH"

mkdir -p ~/.config/gate
cat > ~/.config/gate/config.toml <<EOF
gateway_url = "https://localhost"
admin_token = "$GATE_ADMIN_TOKEN"
EOF
```

---

## Usage Examples

### GitHub CLI and Git

```bash
gate admin token create --name gh-local --scope github
gate activate gh-local --tool gh

gh api user
gh repo list
gh repo clone OWNER/REPO
```

`gate admin token create` talks to the gateway admin API and requires `admin_token` in config or `GATE_ADMIN_TOKEN`. `gate activate` is local: it reads the saved phantom token and configures the selected tool.

Activation sets `GH_HOST`, `GH_ENTERPRISE_TOKEN`, and a generated Git credential helper. Git remotes stay token-free, for example `https://localhost/OWNER/REPO.git`.

### Claude Code

```bash
gate admin token create --name claude-local --scope anthropic
gate activate claude-local --tool claude-code
claude
```

### Codex

```bash
gate admin token create --name codex-local --scope openai
gate activate codex-local --tool codex
codex
```

### Existing Gateway

If someone else operates the gateway, you only need its URL and a phantom token:

```toml
# ~/.config/gate/config.toml
gateway_url = "https://gw.example.com"

[tokens.laptop]
token = "gate_..."
scope = ["github", "openai", "anthropic"]
```

```bash
gate activate laptop --tool gh
```

---

## Routes

Built-in routes are defined in [`profiles/routes/`](./profiles/routes) and embedded at build time. Tool adapters are defined in [`profiles/tools/`](./profiles/tools).

| Path prefix | Scope | Upstream | Credential env |
|---|---|---|---|
| `/openai/...` | `openai` | `https://api.openai.com` | `OPENAI_API_KEY` |
| `/anthropic/...` | `anthropic` | `https://api.anthropic.com` | `ANTHROPIC_API_KEY` |
| `/github/...` | `github` | `https://api.github.com` | `GITHUB_TOKEN` |
| `/api/v3/...` | `github` | `https://api.github.com` | `GITHUB_TOKEN` (`gh` compatibility, strips `/v3`) |
| `/<owner>/<repo>.git/...` | `github` | `https://github.com` | `GITHUB_TOKEN` (Git smart-HTTP) |

Planned future profiles include Groq, ElevenLabs, Telegram, Together, and Ollama.

Status codes: `407` = missing/invalid token, `403` = valid token with wrong scope, `404` = unknown route, `503` = required real credential env var is not set.

---

## How It Works Today

```text
client sends gate_... phantom
    -> gateway validates token with a constant-time Blake3 hash comparison
    -> gateway checks route scope before credential lookup
    -> gateway removes the phantom credential from the request
    -> gateway injects the real server-side credential
    -> upstream receives only the real credential
```

The TEE version will keep the same request model, but move the credential boundary into Intel TDX and expose public attestation. See [spec/tee-security.md](./spec/tee-security.md) for the target security model.

---

## Testing

```bash
cargo fmt --check
cargo test --workspace
./scripts/test-e2e.sh
```

`test-e2e.sh` exercises the Docker flow: gateway startup, admin API, token validation, scope enforcement, revocation, and CLI activation. Live upstream checks are skipped when credentials are not set.

---

## Docs

- [spec/vision.md](./spec/vision.md) - product vision and long-term direction
- [spec/roadmap.md](./spec/roadmap.md) - current status and next implementation milestones
- [docs/USING.md](./docs/USING.md) - detailed per-tool setup for Claude Code, Codex, and `gh`
- [spec/tee-security.md](./spec/tee-security.md) - target TEE security requirements and threat model
- [profiles/README.md](./profiles/README.md) - route and tool profile format
