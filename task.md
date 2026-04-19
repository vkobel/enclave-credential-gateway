# CoCo Credential Gateway — v1 Task List

> Generated from product vision review (April 2026).
> Current state: Phase 1a complete (plain proxy). Phase 1b (CVM attestation) is in-flight.
> Goal: achieve the v1 promise — deploy once, mint phantoms, audit, revoke.

---

## Phase 1b — CVM Attestation (prerequisite for everything below)

Status: tasks defined in `openspec/changes/poc-v1b-cvm-attestation/tasks.md`. Complete these first.

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

- [ ] 2a.1 — Define `TokenRecord` struct: `id` (UUID), `name`, `routes_allowlist` (`Vec<String>`), `budget_daily_requests: Option<u32>`, `budget_daily_tokens: Option<u64>`, `expires_at: Option<DateTime<Utc>>`, `status: Active|Revoked`, `created_at`
- [ ] 2a.2 — Implement `TokenRegistry`: in-memory store backed by an encrypted-at-rest JSON file (`/data/tokens.enc`). Use AES-256-GCM with a key derived inside the enclave (from Phala's injected entropy or a sealed random key generated at first boot)
- [ ] 2a.3 — Persist registry on every mutating operation (add, revoke); load at startup; fail-safe to empty registry on corrupt/missing file
- [ ] 2a.4 — Add `--legacy-token` startup flag: when set, single `COCO_PHANTOM_TOKEN` env var works as before (one-release migration path for existing deployments)
- [ ] 2a.5 — Unit tests: registry round-trip through encryption/decryption, revoked tokens rejected, expired tokens rejected

### 2b. Admin token + admin API

- [ ] 2b.1 — Generate a cryptographically random admin token at first boot; print it once to stdout (`ADMIN TOKEN: <hex>`); never print again. Store its hash in `/data/admin.hash`
- [ ] 2b.2 — Add `AdminAuth` extractor that validates `Authorization: Bearer <admin-token>` on all `/admin/*` routes using constant-time comparison
- [ ] 2b.3 — Implement `POST /admin/tokens` — create token: body `{ "name": str, "routes": [str], "budget_requests": int?, "budget_tokens": int?, "expires_in_days": int? }`, returns `{ "id": uuid, "token": "<hex>", ...record }`
- [ ] 2b.4 — Implement `DELETE /admin/tokens/:id` — revoke token (immediate effect, status set to Revoked, persisted)
- [ ] 2b.5 — Implement `GET /admin/tokens` — list all tokens (omit token value, include all metadata)
- [ ] 2b.6 — Update auth middleware: look up incoming phantom in registry (constant-time); attach `TokenRecord` to request extensions for downstream use by policy and audit

### 2c. `coco` CLI — token subcommands

- [ ] 2c.1 — Add `coco` CLI binary (new crate `crates/coco-cli`) with subcommand structure
- [ ] 2c.2 — `coco token create --name <str> [--routes <csv>] [--budget-requests <n>] [--budget-tokens <n>] [--expires <Nd>]` — calls `POST /admin/tokens`, prints token value once
- [ ] 2c.3 — `coco token revoke <id|name>` — calls `DELETE /admin/tokens/:id`
- [ ] 2c.4 — `coco token ls` — calls `GET /admin/tokens`, pretty-prints table
- [ ] 2c.5 — CLI reads gateway URL + admin token from `~/.config/coco/config.toml` (or env vars `COCO_GATEWAY_URL`, `COCO_ADMIN_TOKEN`)

---

## Phase 3 — Per-Token Policy + Audit Log

### 3a. Per-request policy enforcement

- [ ] 3a.1 — Implement route allowlist check in proxy handler: if `TokenRecord.routes_allowlist` is non-empty and the request prefix is not in it, return `403 Forbidden`
- [ ] 3a.2 — Implement daily request counter per token: atomic in-memory counter, reset at UTC midnight, persisted to `/data/counters.enc` on update. Reject with `429 Too Many Requests` when `budget_daily_requests` is exceeded
- [ ] 3a.3 — Implement approximate daily token counter for LLM routes (anthropic, openai): parse `Content-Length` of request + response as proxy for token consumption (rough estimate; flag as approximate in audit log). Reject with `429` when `budget_daily_tokens` exceeded
- [ ] 3a.4 — Hard expiry check: reject any request from a token whose `expires_at` is in the past with `401 Unauthorized`
- [ ] 3a.5 — Unit tests: route allowlist blocks disallowed routes; daily request cap enforced; expiry enforced

### 3b. Append-only structured audit log

- [ ] 3b.1 — Define `AuditEntry`: `timestamp`, `token_id`, `token_name`, `route`, `method`, `upstream_status`, `request_bytes`, `response_bytes`, `approx_tokens: Option<u64>`, `policy_action: Allow|DenyRoute|DenyBudget|DenyExpiry`
- [ ] 3b.2 — Write one JSON line per request to `/data/audit.log` (newline-delimited JSON). File opened in append mode; no rotation in v1 (rotation is v1.x)
- [ ] 3b.3 — Implement `GET /admin/audit` — returns last N entries (default 100, `?limit=N`). Query param `?token_id=<uuid>` filters by token
- [ ] 3b.4 — Optional S3 sink: if `COCO_AUDIT_S3_BUCKET` and `COCO_AUDIT_S3_PREFIX` are set, flush completed log lines to S3 in background (best-effort, non-blocking)
- [ ] 3b.5 — `coco audit tail` — polls `GET /admin/audit?limit=20` in a loop, pretty-prints new entries (SSE or polling)
- [ ] 3b.6 — `coco audit grep --token <name>` — queries `GET /admin/audit?token_id=<id>` and prints

---

## Phase 4 — Credential Store + Polish

### 4a. Sealed credential store

- [ ] 4a.1 — Implement encrypted credential store: AES-256-GCM, key sealed inside TEE, persisted to `/data/credentials.enc`. Schema: `{ "<name>": { "value": str, "inject_header": str, "format": str } }`
- [ ] 4a.2 — Implement `POST /admin/credentials` — add/update credential: body `{ "name": str, "value": str, "inject_header": str?, "format": str? }`
- [ ] 4a.3 — Implement `DELETE /admin/credentials/:name` — remove credential
- [ ] 4a.4 — Implement `GET /admin/credentials` — list credential names (never values)
- [ ] 4a.5 — Update route resolution: prefer sealed credentials over env var fallback; env vars remain as a backward-compatible fallback
- [ ] 4a.6 — `coco creds add <name> <value> [--header <str>] [--format <str>]`
- [ ] 4a.7 — `coco creds rotate <name> <new-value>` — replaces value in-place, zero downtime
- [ ] 4a.8 — `coco creds rm <name>`
- [ ] 4a.9 — `coco creds ls`

### 4b. Deployment tooling

- [ ] 4b.1 — `coco deploy phala` — one-shot helper: checks `phala` CLI is authenticated, pushes image to GHCR, calls `phala cvms create` with the compose file, waits for CVM to be up, calls `GET /attest`, prints MRTD and admin token
- [ ] 4b.2 — `coco verify <gateway-url>` — fetches `GET /attest`, verifies TDX QuoteV4 against Intel PCS, asserts debug bit unset, prints MRTD for pinning
- [ ] 4b.3 — Write `DEPLOY.md`: Phala account prerequisites → `coco deploy phala` → `coco verify` → `coco creds add` → `coco token create`. Under 15 minutes end-to-end
- [ ] 4b.4 — Write `USING.md`: copy-paste recipes for Claude Code, OpenAI Python SDK, curl, GitHub Actions runner

### 4c. End-to-end test + release

- [ ] 4c.1 — Extend `scripts/test-e2e.sh` to cover: registry token creation, routing enforcement (allowlist), budget cap (mock), revocation, audit log entry verified
- [ ] 4c.2 — Tag `v1.0.0`; publish release notes against the v1 definition of done in `product.md`

---

## Ordering / dependencies

```
1b (CVM attestation, in-flight)
  └── 2a (encrypted registry)
        └── 2b (admin API)
              ├── 2c (CLI: token subcommands)
              └── 3a (per-request policy)   ←── 3b (audit log)
                                                   └── 3b.5-6 (CLI: audit subcommands)
4a (sealed credential store)  ←── can start in parallel with 3a once 2b is done
4b (deployment tooling)       ←── can start once 4a is mostly done
4c (e2e test + release)       ←── final gate
```

Phases 2a–2c are the critical path to a useful system. Start there immediately after 1b ships.
