# CoCo Credential Gateway

CoCo is a credential proxy for AI agents and developer tools. Clients use scoped CoCo tokens (`ccgw_...`) instead of real vendor keys. The gateway validates the CoCo token, checks route scope, removes the client credential, and injects the real upstream credential server-side.

Use it when tools like `gh`, Git, Codex, Claude Code, OpenAI-compatible SDKs, OpenCode, Ollama, or curl scripts need vendor APIs without placing real API keys on the machine running the tool.

## Local Gateway Deployment

This example runs the gateway locally with Docker Compose and Caddy TLS. It uses `https://localhost` because `gh` treats custom GitHub hosts as HTTPS GitHub Enterprise hosts.

Prerequisites:

- Docker with Compose
- Rust/Cargo, for the local `coco` CLI
- `gh`, `git`, `curl`, and `jq`
- A real GitHub token in `GITHUB_TOKEN` for the gateway to inject upstream

### 1. Start the Gateway

Run this from the repository root:

```bash
export COCO_ADMIN_TOKEN="$(openssl rand -hex 32)"
export GITHUB_TOKEN=ghp_...          # real upstream GitHub token, kept server-side
export HTTPBIN_TOKEN=anything        # optional, useful for route smoke tests

docker compose up -d --build
```

What these values do:

- `COCO_ADMIN_TOKEN` protects the gateway admin API. Save it; the CLI needs it to create and revoke CoCo tokens.
- `GITHUB_TOKEN` is the real GitHub credential. Clients never receive it; the gateway injects it only when forwarding `github` traffic upstream.
- `HTTPBIN_TOKEN` enables the `httpbin` route for simple testing.
- Compose stores the token registry in the `coco-data` volume at `/data/tokens.json`.
- Caddy listens on local ports `80` and `443` and reverse-proxies to `coco-gateway:8080`.

Check that the services are up:

```bash
docker compose ps
curl -k https://localhost/health
```

`curl -k` is only for the first check, before trusting Caddy's local certificate.

### 2. Trust Caddy's Local Certificate

Caddy serves `https://localhost` with its local CA by default. Trust that CA once so `curl`, `gh`, and Git accept the gateway certificate.

```bash
docker compose cp caddy:/data/caddy/pki/authorities/local/root.crt /tmp/coco-caddy-root.crt
```

On macOS:

```bash
sudo security add-trusted-cert -d -r trustRoot \
  -k /Library/Keychains/System.keychain /tmp/coco-caddy-root.crt
```

On Debian/Ubuntu:

```bash
sudo cp /tmp/coco-caddy-root.crt /usr/local/share/ca-certificates/coco-caddy-root.crt
sudo update-ca-certificates
```

Now verify without `-k`:

```bash
curl https://localhost/health
```

For a real hostname, set `COCO_DOMAIN=gw.example.com` before `docker compose up`. Caddy will then request a public certificate for that name.

### 3. Install and Configure the CLI

Build the CLI and put it on your `PATH`. The Git credential helper path used by `coco activate --tool gh` relies on `coco` being executable from the shell where Git runs.

```bash
cargo build --release -p coco-cli
export PATH="$PWD/target/release:$PATH"
```

Create `~/.config/coco/config.toml`:

```bash
mkdir -p ~/.config/coco
cat > ~/.config/coco/config.toml <<EOF
gateway_url = "https://localhost"
admin_token = "$COCO_ADMIN_TOKEN"
EOF
```

`gateway_url` is the public URL clients call. `admin_token` is only for token administration; it is not sent to upstream vendors.

## GitHub CLI and Git

This is the full local `gh` flow: create a GitHub-scoped CoCo token, activate the shell environment, and use both `gh` and plain Git through the gateway.

### 1. Create a CoCo Token for GitHub

```bash
coco token create --name gh-local --scope github
```

The CLI sends an authenticated `POST /admin/tokens` request to the local gateway. The gateway stores only a hash of the new CoCo token in its registry. The CLI saves the returned token under `[tokens.gh-local]` in `~/.config/coco/config.toml`.

You can inspect the saved client config:

```bash
sed -n '/tokens.gh-local/,$p' ~/.config/coco/config.toml
```

The saved token is the `ccgw_...` client credential. It is scoped to `github`, so it can use GitHub REST routes, the `gh` `/api/v3/...` compatibility route, and Git smart-HTTP clone/fetch/push routes. It cannot access routes like `openai` or `anthropic`.

### 2. Activate `gh` and Git in the Current Shell

```bash
eval "$(coco activate gh-local --tool gh)"
```

That generated shell config does four things:

- `GH_HOST=localhost` tells `gh` to send GitHub API calls to the local gateway host.
- `GH_ENTERPRISE_TOKEN=ccgw_...` is the token `gh` actually reads for custom hosts.
- `GH_TOKEN=ccgw_...` is also exported for scripts and manual curl examples.
- `GIT_CONFIG_GLOBAL=~/.config/coco/generated/gh/gh-local/gitconfig` points Git at a generated config file for the gateway host.

The generated Git config includes your normal `~/.gitconfig`, resets inherited credential helpers for the gateway URL, and then adds `!coco git-credential gh-local`. Re-running the `eval` is safe because the file is regenerated deterministically.

### 3. Use `gh` and Git

```bash
gh api user
gh repo list
gh repo clone OWNER/REPO

cd REPO
git fetch
git pull
git push
```

`gh` REST calls go to `https://localhost/api/v3/...`; the gateway strips `/v3` and forwards them to `https://api.github.com` with the real `GITHUB_TOKEN`.

`gh repo clone` shells out to Git. Git talks to paths like `https://localhost/OWNER/REPO.git/info/refs`; the gateway forwards those smart-HTTP requests to `https://github.com`. The generated Git credential helper supplies the CoCo token as HTTP Basic auth only for the gateway host.

Confirm that remotes stay token-free:

```bash
git remote -v
```

The remote should contain `https://localhost/OWNER/REPO.git`, not a `ccgw_...` token and not the real `GITHUB_TOKEN`.

## Other Tool Examples

For longer per-tool setup, see [docs/USING.md](./docs/USING.md).

### Codex

```bash
coco token create --name codex-local --scope openai
coco activate codex-local --write --tool codex
codex
```

Codex needs `~/.codex/config.toml` for the gateway URL and `~/.codex/auth.json` for API-key auth. The Codex `openai_base_url` must include `/v1`; `OPENAI_BASE_URL` is enough for SDKs and curl, but not for Codex.

### Generic SDKs and curl

```bash
coco token create --name sdk-local --scope openai
eval "$(coco activate sdk-local --tool shell)"

curl "$OPENAI_BASE_URL/v1/models" \
  -H "Authorization: Bearer $OPENAI_API_KEY"
```

The API key value is the CoCo token. The gateway swaps it for the real upstream key before forwarding.

## Use an Existing Gateway

If someone else operates the gateway, you only need:

- `gateway_url`, for example `https://gw.example.com`
- a CoCo token, for example `ccgw_...`

Install the CLI, then create `~/.config/coco/config.toml`:

```toml
gateway_url = "https://gw.example.com"

[tokens.laptop]
token = "ccgw_..."
scope = ["github", "openai", "anthropic"]
all_routes = false
```

The local `scope` list tells the CLI which environment variables and tool adapters to render. It should match the scopes granted by the gateway. Use `all_routes = true` only for unrestricted tokens.

Then activate the tool you need:

```bash
eval "$(coco activate laptop --tool gh)"
coco activate laptop --write --tool codex
eval "$(coco activate laptop --tool shell)"
```

## Routes

Built-in routes and tool adapters are defined together in [profiles/coco.yaml](./profiles/coco.yaml) and embedded into the binaries at build time.

Token scopes use the top-level route key. Unrestricted tokens are explicit: create them with `coco token create --name laptop --all-routes`, which stores `all_routes = true` in the local CLI config. GitHub also owns an `/api/v3/...` compatibility alias for `gh`; it scopes as `github` and strips `/v3` before forwarding. Git smart-HTTP paths also scope as `github` and proxy to `github.com`.

| Path prefix | Scope | Upstream | Gateway credential env | Injection |
|---|---|---|---|---|
| `/openai/...` | `openai` | `https://api.openai.com` | `OPENAI_API_KEY` | `Authorization: Bearer ...` |
| `/anthropic/...` | `anthropic` | `https://api.anthropic.com` | `ANTHROPIC_API_KEY` | OAuth tokens as `Authorization: Bearer ...` plus `anthropic-beta: oauth-2025-04-20`; API keys as `x-api-key`; rejects `ccgw_...` phantom values upstream |
| `/github/...` | `github` | `https://api.github.com` | `GITHUB_TOKEN` | `Authorization: Bearer ...` |
| `/api/v3/...` | `github` | `https://api.github.com` | `GITHUB_TOKEN` | `gh` compatibility route; strips route-relative `/v3` |
| `/<owner>/<repo>.git/{info/refs,git-upload-pack,git-receive-pack}` | `github` | `https://github.com` | `GITHUB_TOKEN` | Git smart-HTTP; accepts HTTP Basic auth |
| `/httpbin/...` | `httpbin` | `https://httpbin.org` | `HTTPBIN_TOKEN` | `Authorization: Bearer ...` |
| `/ollama/...` | `ollama` | `https://ollama.com` | `OLLAMA_API_KEY` | Ollama Cloud API; `Authorization: Bearer ...` |
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

Supported fields:

| Field | Purpose |
|---|---|
| `upstream` | Base upstream URL |
| `credential_sources` | Ordered env-backed credentials with `env`, `inject_header`, optional `format`, optional `prefix`, optional `reject_prefixes`, and optional `extra_headers` |
| `aliases` | Optional compatibility prefixes owned by this route, such as GitHub's `api` alias |
| `strip_prefix` | Alias path prefix to remove before forwarding |
| `inject_mode` | `header` or `url_path`; defaults to `header` |
| `url_path_prefix` | Prefix used by `url_path` credential injection |
| `git_protocol` | Optional companion route that proxies git smart-HTTP requests to a separate upstream. Currently used only by `github`; shares scope and credentials with the parent route. |

## How It Works

```text
client sends CoCo token -> gateway validates token and route scope
                         -> gateway selects the route profile
                         -> gateway removes the client credential
                         -> gateway injects the real server-side credential
                         -> upstream API receives only the real credential
```

Status codes:

| Code | Meaning |
|---|---|
| `200` | Upstream request succeeded |
| `407` | Missing, revoked, or invalid CoCo token |
| `403` | Token is valid but not scoped for this route |
| `404` | Unknown route prefix |
| `503` | Route exists but required real credential env var is missing |

`COCO_PHANTOM_TOKEN` is still supported as a legacy single-token fallback, but the registry/admin API path is preferred.

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
- `./scripts/test-e2e.sh` runs the Docker-backed end-to-end flow: gateway startup, admin token creation, missing/wrong token handling, route-scope denial, revocation, CLI activation, and optional upstream credential injection checks.

Run focused package tests while iterating:

```bash
cargo test -p coco-gateway
cargo test -p coco-cli
```

Optional live upstream checks:

```bash
export HTTPBIN_TOKEN=anything
export OPENAI_API_KEY=sk-...
export ANTHROPIC_API_KEY=sk-ant-api-...
export OLLAMA_API_KEY=ollama_...
export GITHUB_TOKEN=ghp_...
./scripts/test-e2e.sh
```

Missing real upstream credentials are skipped, not treated as failures. Live GitHub checks exercise REST calls plus Git clone, push, and pull through the gateway.

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
