# CoCo Credential Gateway — Tasks

## Progress

| Phase | Description | Status |
|---|---|---|
| 1a | Plain proxy — phantom token auth, profile routing, multi-source credential injection | **done** |
| 1c | Remote deploy, built-in route manifest, lightweight token registry, local `coco` CLI | **done** |
| 1b | CVM attestation — `GET /attest`, TDX QuoteV4, Phala deploy | **next** |
| 2 | Full token registry — encrypted store, admin API, policy | not started |
| 3 | Per-token policy + audit log | not started |
| 4 | Sealed credential store + full CLI + polish | not started |

---

## Standards anchor

CoCo implements a TEE-backed RFC 8693 Security Token Service.
- The phantom token is the `subject_token` (who is acting).
- The TEE is the STS (the authority that decides what rights to grant).
- The injected upstream credential — or in post-v1, a short-lived derived JWT — is the output `access_token`.

---

## Mental model

OneCLI and AgentSecrets protect credentials from the agent.
CoCo protects credentials from everyone — including the infrastructure operator —
while producing a cryptographically verifiable audit trail.

A classical KMS answers: "here is your secret, go use it."
CoCo answers: "give me the request, I'll authenticate and execute it inside a hardware boundary, here's the response."
The credential participates in the live request but is never transmitted to the requester.

---

## CLI design — user stories

Principles: noun-first verb-second (`coco cred add` not `coco add-cred`). `--json` flag on every command for scripting. Errors to stderr. One-line happy paths. Output is a table by default, machine-readable with `--json`.

### Story 1 — First deploy (4 commands)
```
coco deploy phala
# → builds image, pushes to GHCR, creates Phala CVM, waits for liveness
# → prints: MRTD: a3f9…  ADMIN TOKEN: ccgw_admin_… (once, save it)

coco verify https://gw.example
# → TDX Quote: valid | Debug: false | MRTD: a3f9… | Binary: sha256:…

coco cred add openai sk-proj-…  [--scope api.openai.com/v1/*]
# → Sealed. Never echoed.

coco token create --name claude-code [--scope api.openai.com] [--expires 30d]
# → ccgw_3a9f…  (printed once)
```

### Story 2 — Give CI a narrowly scoped token
```
coco token create --name github-actions \
  --scope api.openai.com/v1/embeddings \
  --expires 30d
# → ccgw_ab12…

coco token ls
# NAME             SCOPE                            EXPIRES    CALLS
# claude-code      api.openai.com/*                 never      142
# github-actions   api.openai.com/v1/embeddings     29d        0
```

### Story 3 — Live audit
```
coco audit tail                       # streams all tokens
coco audit tail --token claude-code   # filter by name
# 20:14:03  claude-code  POST  api.openai.com/v1/chat         200  1.2s
# 20:14:07  claude-code  POST  api.anthropic.com/v1/messages  403  DENIED:scope
# 20:14:09  claude-code  POST  api.openai.com/v1/chat         200  0.8s
```

### Story 4 — Incident: revoke immediately
```
coco token revoke claude-code
# Revoked. 0 further requests will be served for this token.
```

### Story 5 — Credential rotation (zero downtime)
```
coco cred rotate openai sk-proj-newkey…
# Rotated. In-flight requests complete with previous value.
# New value active for all subsequent requests.

coco cred ls
# NAME    SCOPE                  ROTATED
# openai  api.openai.com/*       2m ago
# github  api.github.com/*       never
```

### Story 6 — Auditor / new team member verifies gateway
```
coco verify https://gw.example
# TDX Quote:   valid (Intel PCS)
# Debug mode:  false
# MRTD:        a3f9…  ← pin in client config
# Binary:      ghcr.io/vkobel/coco-gateway:v1.0.0@sha256:…
# Issued:      2026-04-19T18:00Z
```

### Command surface summary
```
coco deploy phala
coco verify <url>

coco token create --name <str> [--scope <host/path>] [--expires <Nd>]
coco token ls [--json]
coco token revoke <name|id>

coco cred add <name> <value> [--scope <host/path>] [--header <str>]
coco cred rotate <name> <new-value>
coco cred ls [--json]
coco cred rm <name>

coco audit tail [--token <name>] [--limit <n>]
coco audit grep --token <name> [--since <duration>] [--json]

coco status    # gateway liveness + attestation summary
```

Config: `~/.config/coco/config.toml` (or `COCO_GATEWAY_URL` + `COCO_ADMIN_TOKEN` env vars).
The `coco` binary never touches credential values after `cred add` — it sends them over mTLS to the admin API and they are sealed inside the TEE immediately.

---

## Phase 1c — Remote Deploy + Profile Library + Token Registry + Local CLI ← next

Goal: a remotely deployable gateway that any agent or tool can connect to, with a curated set of named service profiles, per-client named tokens, and a local `coco` helper that sets up your shell in one command.

### 1c-A. Remote deploy infrastructure

- [x] 1c.1 — Add `GET /health` unauthenticated endpoint returning `200 OK {"status":"ok"}`.
- [x] 1c.2 — Add alias `strip_prefix` support. Needed for `GH_HOST`: GitHub Enterprise clients route to `/api/v3/...`; the `github` route owns an `api` compatibility alias that strips route-relative `/v3` before forwarding to `api.github.com`.
  - ⚠️ Known limitation: the path-prefix compatibility alias (`"api"` scoped as `github`) conflicts with any other registered route that also uses `/api/` as its base path. CLI tools like `gh` cannot include a path in `GH_HOST`, so they always hit the root of a hostname — path-prefix routing is inherently fragile for them. See post-v1: **Host-based routing**.
- [x] 1c.3 — Add Caddy service to `docker-compose.yml` for automatic TLS termination (Let's Encrypt). Caddy proxies `443 → 8080`. Gateway itself stays HTTP-only behind it.

### 1c-B. Built-in route manifest

- [x] 1c.4 — Define the extended route schema fields needed by new profiles:
  - `aliases: [{ prefix, strip_prefix? }]` — compatibility prefixes owned by the top-level route
  - `inject_mode: "header" | "url_path"` — where to inject the credential (default: `"header"`)
    - `url_path`: inserts the credential into the upstream path (needed for Telegram: `/bot{credential}/...`)
  - `url_path_prefix: Option<String>` — path prefix placed before the credential when `inject_mode = "url_path"`
- [x] 1c.5 — Ship the following routes in the single embedded `profiles/routes.json` manifest:

  | Route | Upstream | inject_mode | Notes |
  |---|---|---|---|
  | `anthropic` | api.anthropic.com | header | x-api-key or Bearer (existing multi-source) |
  | `openai` | api.openai.com | header | Authorization: Bearer |
  | `github` | api.github.com | header | Authorization: Bearer + `api` alias with route-relative strip_prefix /v3 |
  | `groq` | api.groq.com | header | OpenAI-compatible, Authorization: Bearer |
  | `elevenlabs` | api.elevenlabs.io | header | xi-api-key header |
  | `ollama` | configurable upstream | header | Authorization: Bearer; OLLAMA_HOST=https://gw.example.com/ollama |
  | `telegram` | api.telegram.org | url_path | /bot{credential}/... |
  | `together` | api.together.xyz | header | Authorization: Bearer (OpenAI-compat) |

- [x] 1c.6 — Keep built-in routes embedded only. Do not support runtime profile overrides until a real route extension model exists.
- [x] 1c.7 — Add profile validation at startup: log a warning (not a fatal error) for any route with `inject_mode = url_path` that has no `{credential}` placeholder in its upstream URL.

### 1c-C. Lightweight token registry

Replaces the single `COCO_PHANTOM_TOKEN` env var with a named multi-token registry. No encryption yet (that comes with TEE in 1b/2). Tokens are stored in a JSON file, hashed at rest.

- [x] 1c.8 — Define `TokenRecord`: `id` (UUID), `name` (string), `scope` (optional list of route prefixes this token is allowed to call), `created_at`, `status: Active|Revoked`.
- [x] 1c.9 — Implement `TokenRegistry`: load from `/data/tokens.json` at startup; persist on every mutation. Hash tokens with `blake3` (fast, not a password hash — the token itself is 256-bit random so brute force is not a concern). Fail-safe to empty registry on missing/corrupt file.
- [x] 1c.10 — Admin API, protected by `COCO_ADMIN_TOKEN` env var (set at deploy time, validated constant-time):
  - `POST /admin/tokens` — body `{"name": str, "scope": [str]?}` → generates a 32-byte hex token, stores hashed, returns `{"id": uuid, "name": str, "token": "ccgw_<hex>"}` (token shown once)
  - `GET /admin/tokens` — list all records (name, id, scope, status, created_at — never the token value)
  - `DELETE /admin/tokens/:id` — revoke (sets status to Revoked, persists)
- [x] 1c.11 — Update auth middleware to validate against the token registry (constant-time hash comparison). Attach matched `TokenRecord` to request extensions so scope can be checked by the proxy handler.
- [x] 1c.12 — Enforce scope: if `TokenRecord.scope` is non-empty and the request's route prefix is not in it, return `403 Forbidden`. Evaluate before credential resolution.
- [x] 1c.13 — Keep `COCO_PHANTOM_TOKEN` as a legacy fallback: if set and the registry lookup fails, fall back to the single-token check (backwards compat for existing deployments).
- [x] 1c.14 — Unit tests: token creation round-trip, revoked token rejected, scope enforcement, legacy fallback.

### 1c-D. Local `coco` CLI

A single Rust binary (`crates/coco-cli`) with minimal subcommands. Goal: one command activates the right env vars (and tool-specific config files) for the current shell session.

- [x] 1c.15 — Scaffold `crates/coco-cli` with clap. Config file at `~/.config/coco/config.toml`:
  ```toml
  gateway_url = "https://gw.example.com"
  admin_token = "ccgw_admin_..."  # optional, only needed for token management

  [tokens]
  claude-code  = "ccgw_3a9f..."
  ci-runner    = "ccgw_ab12..."
  ```
- [x] 1c.16 — `coco env <token-name>` — prints `export` statements for every tool that the gateway supports, using the named token as the phantom. Output is eval'd in the shell: `eval $(coco env claude-code)`. Emits:
  ```bash
  export ANTHROPIC_BASE_URL=https://gw.example.com/anthropic
  export ANTHROPIC_API_KEY=ccgw_3a9f...
  export OPENAI_BASE_URL=https://gw.example.com/openai
  export OPENAI_API_KEY=ccgw_3a9f...
  export GH_HOST=gw.example.com
  export GH_TOKEN=ccgw_3a9f...
  export OLLAMA_HOST=https://gw.example.com/ollama
  ```
- [x] 1c.17 — `coco env <token-name> --codex` — additionally writes `~/.codex/config.toml` with `openai_base_url` set (Codex CLI does not read `OPENAI_BASE_URL` from env; requires its own config file).
- [x] 1c.18 — `coco token create --name <str> [--scope <csv>]` — calls `POST /admin/tokens`, prints the token value once.
- [x] 1c.19 — `coco token ls` — calls `GET /admin/tokens`, pretty-prints table.
- [x] 1c.20 — `coco token revoke <name|id>` — calls `DELETE /admin/tokens/:id`.
- [x] 1c.21 — Write `docs/USING.md`: copy-paste setup for Claude Code, Codex, gh CLI, and Ollama on a remote gateway. Shows both `eval $(coco env ...)` flow and manual env var approach.

---

**Usage after 1c:**
```bash
# Remote host:
# env: COCO_ADMIN_TOKEN=<secret>, ANTHROPIC_API_KEY=sk-ant-..., GITHUB_TOKEN=ghp_..., etc.
docker compose up -d   # Caddy handles TLS

# Create a named token for your laptop:
coco token create --name laptop --scope anthropic,openai,github,ollama
# → ccgw_3a9f…  (save to ~/.config/coco/config.toml)

# Activate in your shell (Claude Code + Codex + gh + Ollama all configured):
eval $(coco env laptop --codex)

# Everything works:
claude                    # Claude Code → gateway → Anthropic
codex                     # Codex CLI   → gateway → OpenAI
gh repo list              # gh CLI      → gateway → GitHub
ollama run llama3.2       # Ollama      → gateway → your Ollama server
```

---

## Phase 1b — CVM Attestation

Status: not started. Can be done in parallel with or after 1c.

- [ ] 1b.1 — Add `reqwest` dependency and implement `GET /attest` handler (tappd → TDX QuoteV4, hex-encode, JSON response)
- [ ] 1b.2 — Parse `td_attributes` bit 0; log `ERROR: TDX debug mode detected` and set `"debug": true` on mismatch
- [ ] 1b.3 — Return `503` when tappd is unreachable; gateway continues serving proxy routes
- [ ] 1b.4 — Wire `GET /attest` as an unauthenticated route in the Axum router
- [ ] 1b.5 — Add GitHub Actions workflow: build + push to GHCR on push to `main`
- [ ] 1b.6 — Deploy to Phala Cloud TDX CVM; provision secrets via `phala cvms secrets set`
- [ ] 1b.7 — Validate end-to-end: `GET /attest` returns valid non-debug TDX quote; OpenAI client succeeds through the CVM

---

## Phase 2 — Phantom Token Registry

Replaces the single `COCO_PHANTOM_TOKEN` env var with a named, per-client token registry stored encrypted inside the TEE.

### 2a. Encrypted registry storage

- [ ] 2a.1 — Define `TokenRecord` struct: `id` (UUID), `name` (human label for audit log and CLI), `routes_allowlist` (`Vec<String>`), `expires_at: Option<DateTime<Utc>>`, `status: Active|Revoked`, `created_at`
- [ ] 2a.2 — Implement `TokenRegistry`: in-memory store backed by an encrypted-at-rest JSON file (`/data/tokens.enc`). Use AES-256-GCM with a key derived inside the enclave
- [ ] 2a.3 — Persist registry on every mutating operation (add, revoke); load at startup; fail-safe to empty registry on corrupt/missing file
- [ ] 2a.4 — Add `--legacy-token` startup flag: when set, single `COCO_PHANTOM_TOKEN` env var works as before (one-release migration path)
- [ ] 2a.5 — Unit tests: registry round-trip through encryption/decryption, revoked tokens rejected, expired tokens rejected

### 2b. Admin token + admin API

- [ ] 2b.1 — Generate a cryptographically random admin token at first boot; print it once to stdout (`ADMIN TOKEN: <hex>`); store hash in `/data/admin.hash`
- [ ] 2b.2 — Add `AdminAuth` extractor that validates `Authorization: Bearer <admin-token>` on all `/admin/*` routes using constant-time comparison
- [ ] 2b.3 — Implement `POST /admin/tokens` — body `{ "name": str, "routes": [str], "expires_in_days": int? }`, returns `{ "id": uuid, "token": "<hex>", ...record }`
- [ ] 2b.4 — Implement `DELETE /admin/tokens/:id` — revoke token (immediate effect, persisted)
- [ ] 2b.5 — Implement `GET /admin/tokens` — list all tokens (omit token value, include all metadata)
- [ ] 2b.6 — Update auth middleware: look up incoming phantom in registry (constant-time); attach `TokenRecord` to request extensions for downstream use

### 2c. `coco` CLI — token subcommands

- [ ] 2c.1 — Add `coco` CLI binary (new crate `crates/coco-cli`) with subcommand structure
- [ ] 2c.2 — `coco token create --name <str> [--routes <csv>] [--expires <Nd>]` — calls `POST /admin/tokens`, prints token value once
- [ ] 2c.3 — `coco token revoke <id|name>` — calls `DELETE /admin/tokens/:id`
- [ ] 2c.4 — `coco token ls` — calls `GET /admin/tokens`, pretty-prints table
- [ ] 2c.5 — CLI reads gateway URL + admin token from `~/.config/coco/config.toml` (or env vars `COCO_GATEWAY_URL`, `COCO_ADMIN_TOKEN`)

---

## Phase 3 — Per-Token Policy + Audit Log

### 3a. Per-request policy enforcement

- [ ] 3a.0 — Evaluate allowlist *before* credential resolution. Return `403` without touching the credential store if the target host+path is not in the allowlist (avoids timing side-channel).
- [ ] 3a.1 — Implement route allowlist check: if `TokenRecord.routes_allowlist` is non-empty and request prefix is not in it, return `403 Forbidden`
- [ ] 3a.2 — Hard expiry check: reject requests from tokens whose `expires_at` is in the past with `401 Unauthorized`
- [ ] 3a.3 — Unit tests: route allowlist blocks disallowed routes; expiry enforced

### 3b. Append-only structured audit log

- [ ] 3b.0 — Implement response body credential redaction: scan upstream response body for injected credential values; replace with `[REDACTED_BY_COCO]` before forwarding. Closes the credential-echo exfiltration path.
- [ ] 3b.1 — Define `AuditEntry`: `timestamp`, `token_id`, `token_name`, `route`, `method`, `upstream_status`, `request_bytes`, `response_bytes`, `policy_action: Allow|DenyRoute|DenyExpiry`
- [ ] 3b.2 — Write one JSON line per request to `/data/audit.log` (newline-delimited). Append mode; no rotation in v1.
- [ ] 3b.3 — Implement `GET /admin/audit` — returns last N entries (default 100, `?limit=N`, `?token_id=<uuid>` filter)
- [ ] 3b.4 — Optional S3 sink: if `COCO_AUDIT_S3_BUCKET` is set, flush log lines to S3 in background (best-effort)
- [ ] 3b.5 — `coco audit tail` — polls `GET /admin/audit?limit=20`, pretty-prints new entries
- [ ] 3b.6 — `coco audit grep --token <name>` — queries `GET /admin/audit?token_id=<id>` and prints

---

## Phase 4 — Credential Store + Polish

### 4a. Sealed credential store

- [ ] 4a.1 — Implement encrypted credential store: AES-256-GCM, key sealed inside TEE, persisted to `/data/credentials.enc`
- [ ] 4a.2 — Implement `POST /admin/credentials` — add/update credential
- [ ] 4a.3 — Implement `DELETE /admin/credentials/:name`
- [ ] 4a.4 — Implement `GET /admin/credentials` — list names only (never values)
- [ ] 4a.5 — Update route resolution: prefer sealed credentials over env var fallback
- [ ] 4a.6 — `coco creds add <name> <value> [--header <str>] [--format <str>]`
- [ ] 4a.7 — `coco creds rotate <name> <new-value>` — zero-downtime replacement
- [ ] 4a.8 — `coco creds rm <name>`
- [ ] 4a.9 — `coco creds ls`

### 4b. Deployment tooling

- [ ] 4b.1 — `coco deploy phala` — checks auth, pushes image to GHCR, creates CVM, waits for liveness, calls `GET /attest`, prints MRTD and admin token
- [ ] 4b.2 — `coco verify <url>` — fetches `GET /attest`, verifies TDX QuoteV4, asserts no debug bit, prints MRTD
- [ ] 4b.3 — Write `docs/DEPLOY.md`: end-to-end walkthrough under 15 minutes

### 4c. End-to-end test + release

- [ ] 4c.1 — Extend `scripts/test-e2e.sh`: registry token creation, routing enforcement, revocation, credential-echo redaction, audit log with `token_name`
- [ ] 4c.2 — Tag `v1.0.0`; publish release notes

---

## Post-v1 ideas

- **Host-based routing**: each service gets its own subdomain (`github.localhost`, `openai.localhost`, …). Caddy routes by `Host` header and injects `X-Coco-Route: <service>`; the gateway checks that header before falling back to path prefix. Eliminates the `/api` route conflict and works naturally with CLI tools (`GH_HOST=github.localhost`) since they cannot include a path in the host setting. Path-prefix routing stays as a fallback for SDK clients. `coco env` would emit per-service hostnames instead of a single gateway URL.
- **`HTTPS_PROXY` / CONNECT mode**: inject credentials via SSL interception. Unlocks zero-config agent integration for tools without a configurable base URL. Requires CA cert distribution.
- **Local `coco` proxy mode**: a local process that sets env vars and proxies through the remote gateway, abstracting away per-tool configuration.
- **Derived credential injection**: derive a short-lived scoped token inside the TEE instead of injecting the raw key.
- **Workload identity**: agent proves TEE identity; CoCo derives credentials on the fly from a sealed root key.

---

## Ordering

```
1a (done)
  └── 1c (remote deploy — next)
        └── 1b (CVM attestation — parallel or after 1c)
              └── 2a (encrypted registry)
                    └── 2b (admin API)
                          ├── 2c (CLI: token subcommands)
                          └── 3a (per-request policy) ←── 3b (audit log)
4a (sealed cred store)  ←── parallel with 3a once 2b done
4b (deploy tooling)     ←── after 4a
4c (e2e + release)      ←── final gate
```
