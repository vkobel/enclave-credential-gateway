# CoCo Credential Gateway — Roadmap

## Phase Status

| Phase | Description | Status |
|---|---|---|
| **1a** | Phantom token auth, profile routing, multi-source credential injection | ✅ done |
| **1c** | Remote deploy, built-in route manifest, token registry, CLI | ✅ done |
| **1b** | TDX attestation — `GET /attest`, Phala CVM deploy, GHCR image | 🔜 next |
| **2** | Per-token policy, encrypted registry, audit log | not started |
| **3** | Sealed credential store, full CLI, deploy tooling, v1 release | not started |

---

## What's Done (1a + 1c)

**Gateway (`crates/coco-gateway`):**
- Constant-time phantom token validation (Blake3 hash compare)
- Named token registry persisted to `/data/tokens.json`
- Admin API: `POST/GET/DELETE /admin/tokens`
- Scope enforcement: per-token route allowlist, 403 before credential resolution
- Route profiles: OpenAI, Anthropic, GitHub, Groq, ElevenLabs, Telegram, Together, Ollama
- Injection modes: `header` (Bearer / x-api-key) and `url_path` (Telegram)
- GitHub aliases: `/api/v3/...` compatibility, Git smart-HTTP
- Caddy TLS termination; `GET /health` endpoint

**CLI (`crates/coco-cli`):**
- Config at `~/.config/coco/config.toml`
- `coco token create/revoke/ls` — calls admin API
- `coco activate <name> --tool <gh|codex|claude-code>` — writes env vars and tool-specific config files
- `coco git-credential <name>` — Git credential helper for gateway host

**Profiles (`profiles/`):**
- One YAML file per route (`profiles/routes/*.yaml`) and per tool adapter (`profiles/tools/*.yaml`)
- Embedded at build time via `include_str!`

---

## Phase 1b — TDX Attestation (next)

**Goal:** prove to a verifier that the running binary matches the published source. An independent party can call `GET /attest`, verify the TDX QuoteV4, and confirm the MRTD matches the released image digest.

**Tasks:**

- [ ] **1b.1** — Add `GET /attest` handler in `crates/coco-gateway/src/`. Calls the tappd sidecar (`POST /prpc/Tappd.TdxQuote` on `http://localhost:1080`), accepts optional `?nonce=<hex>` query param, hashes it into `reportData`, returns `{ "quote": "<hex>", "debug": bool }`.
- [ ] **1b.2** — Parse `td_attributes` bit 0: if set, log `ERROR: TDX debug mode` and include `"debug": true` in the response.
- [ ] **1b.3** — Return `503` when tappd is unreachable; gateway continues serving proxy routes normally.
- [ ] **1b.4** — Wire `GET /attest` as an unauthenticated route in `main.rs` router (alongside `/health`).
- [ ] **1b.5** — Add GitHub Actions workflow (`.github/workflows/publish.yml`): on push to `main`, build the gateway binary with `cargo build --locked --release`, build Docker image, push to GHCR (`ghcr.io/vkobel/coco-gateway:latest` + SHA tag). Requires `GHCR_TOKEN` secret.
- [ ] **1b.6** — Deploy to Phala Cloud TDX CVM using the GHCR image. Provision real credentials via `phala cvms secrets set ANTHROPIC_API_KEY=...`. Document the steps in `docs/DEPLOY.md`.
- [ ] **1b.7** — Validate end-to-end: `GET /attest` returns a valid non-debug TDX quote; a phantom-token request reaches OpenAI through the CVM.

**Acceptance:** `curl https://gw.example.com/attest | jq '.debug'` prints `false`. MRTD in the response matches the image digest published in the GitHub release.

---

## Phase 2 — Policy, Audit Log, and Non-LLM Profiles

- [ ] **2a** — Per-token hard expiry: reject requests from tokens where `expires_at` is in the past (`401`). Admin API accepts `expires_in_days` on token creation.
- [ ] **2b** — Append-only audit log: write one JSON line per request to `/data/audit.log`. Fields: `timestamp`, `token_name`, `route`, `method`, `upstream_status`, `request_bytes`, `response_bytes`, `policy_action`.
- [ ] **2c** — `GET /admin/audit` admin endpoint: returns last N log entries, filterable by `token_name`. Default `?limit=100`.
- [ ] **2d** — `coco audit tail [--token <name>]` CLI subcommand: polls `GET /admin/audit` and pretty-prints new entries.
- [ ] **2e** — Response body credential redaction: scan upstream response bodies for injected credential values; replace with `[REDACTED_BY_COCO]` before forwarding (closes the credential-echo exfiltration path).

---

## Phase 3 — Sealed Credential Store, CLI Polish, and v1 Release

- [ ] **3a** — Encrypted credential store: AES-256-GCM, key derived inside the TEE (Phala secret injection for v1). Persisted to `/data/credentials.enc`. Admin API: `POST/GET/DELETE /admin/credentials`.
- [ ] **3b** — `coco creds add/rotate/rm/ls` CLI subcommands. `add` transmits the value once over the admin API; it is never echoed back.
- [ ] **3c** — `coco verify <gateway-url>`: fetches `GET /attest?nonce=<random>`, verifies TDX QuoteV4 against Intel PCS, asserts no debug bit, checks nonce is in `reportData`, compares MRTD to pinned value, prints pass/fail summary.
- [ ] **3d** — Write `docs/DEPLOY.md`: end-to-end walkthrough, "I have a Phala account" → working gateway in under 15 minutes.
- [ ] **3e** — Extend `scripts/test-e2e.sh`: token expiry rejection, audit log entries, credential-echo redaction.
- [ ] **3f** — Tag `v1.0.0`; publish release notes with GHCR image digest and MRTD.

---

## Dependency Order

```
1a (done)
  └── 1c (done)
        └── 1b (next — attestation + CI/CD)
              └── 2a-2e (policy + audit log)
                    └── 3a-3f (credential store + release)
```

---

## Post-v1 Direction

See [spec/vision.md](./vision.md) for the full roadmap beyond v1. Key priorities:

- **`HTTPS_PROXY` / CONNECT mode** — zero-config integration for tools without a configurable base URL
- **Host-based routing** — per-service subdomains (`github.localhost`, `openai.localhost`) to eliminate path-prefix conflicts
- **Mobile credential injection** — owner-direct attested injection (credential encrypted to the enclave's ephemeral key, never visible to the operator)
- **Multi-operator support (v2)** — per-operator API tokens, audit log with attribution
