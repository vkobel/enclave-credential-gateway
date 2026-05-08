# Enclave Credential Gateway - Roadmap

## Phase Status

| Phase | Description | Status |
|---|---|---|
| **1a** | Phantom token auth, profile routing, multi-source credential injection | done |
| **1b** | Docker/Caddy deployment scaffold, token registry, CLI activation | done |
| **1c** | TDX attestation, CI image publishing, reproducibility groundwork | next |
| **2** | Audit log, token expiry, response redaction, additional route profiles | not started |
| **3** | Sealed credential store, `gate verify`, deploy polish, v1 release | not started |

---

## Done: Working Proxy and CLI

**Gateway (`crates/enclave-credential-gateway`):**

- Constant-time phantom token validation.
- Named token registry persisted to `tokens.json` with Blake3 token hashes.
- Admin API: `POST /admin/tokens`, `GET /admin/tokens`, `DELETE /admin/tokens/:id`.
- Scope enforcement before credential resolution.
- Embedded route profiles for OpenAI, Anthropic, and GitHub.
- Header credential injection, plus GitHub `/api/v3/...` and Git smart-HTTP compatibility.
- Caddy TLS termination through Docker Compose.
- `GET /health` endpoint.

**CLI (`crates/gate-cli`):**

- Config at `~/.config/gate/config.toml`.
- `gate admin token create/revoke/ls` for gateway token administration.
- `gate activate <name> --tool <gh|codex|claude-code>` for local tool setup.
- `gate git-credential <name>` for GitHub smart-HTTP through the gateway.

**Profiles (`profiles/`):**

- One YAML file per shipped route under `profiles/routes/`.
- One YAML file per tool adapter under `profiles/tools/`.
- Profiles are embedded at build time via `include_str!`.

---

## Phase 1c - TDX Attestation and Release Groundwork

**Goal:** make it possible for a verifier to ask the running gateway what code it is running, then compare that answer to published release artifacts.

- [ ] Add unauthenticated `GET /attest`.
- [ ] Fetch a TDX QuoteV4 from tappd and return the quote as hex JSON.
- [ ] Accept `?nonce=<hex>` and bind it into `reportData`.
- [ ] Surface TDX debug mode in the response and logs.
- [ ] Return `503` from `/attest` when tappd is unavailable while keeping proxy routes available.
- [ ] Add CI that builds the gateway with `cargo build --locked --release` and publishes a GHCR image.
- [ ] Start reproducibility hardening: pinned Rust toolchain, pinned base image, release digest publication, and a reproduction script.
- [ ] Document a first Phala Cloud TDX deployment path.

Acceptance: a deployed gateway exposes `/attest`, a non-debug quote can be retrieved, and release artifacts identify the image digest that future `gate verify` work will pin.

---

## Phase 2 - Policy, Audit Log, and More Profiles

- [ ] Add per-token expiry and reject expired tokens.
- [ ] Add append-only request audit log with token name, route, method, upstream status, byte counts, and policy action.
- [ ] Add `GET /admin/audit` with simple limit and token filters.
- [ ] Add `gate audit tail`.
- [ ] Add response body credential redaction for upstream credential echoes.
- [ ] Add additional route profiles after tests are in place: Groq, ElevenLabs, Telegram, Together, and Ollama.

---

## Phase 3 - Sealed Credentials and v1 Release

- [ ] Add sealed/encrypted credential storage at `/data/credentials.enc`.
- [ ] Add admin credential management endpoints that never return credential values.
- [ ] Add `gate creds add/rotate/rm/ls`.
- [ ] Add `gate verify <gateway-url>` for TDX quote verification, nonce binding, debug-bit rejection, and MRTD comparison.
- [ ] Add a heavier `gate verify --reproduce` path that locally rebuilds the release and compares expected measurements against live enclave evidence.
- [ ] Write `docs/DEPLOY.md` for a complete Phala deployment.
- [ ] Extend e2e coverage for expiry, audit entries, redaction, and sealed credential behavior.
- [ ] Publish v1 release notes with image digest and MRTD.

---

## Dependency Order

```text
1a (working proxy foundation)
  -> 1b (registry, CLI activation, Docker/Caddy scaffold)
      -> 1c (attestation and release artifacts)
          -> 2 (policy, audit, more routes)
              -> 3 (sealed credentials, verify, v1 release)
```

---

## Post-v1 Direction

See [spec/vision.md](./vision.md) for the longer-term product direction. Key themes:

- `HTTPS_PROXY` / CONNECT mode for tools without configurable base URLs.
- Host-based routing with per-service subdomains.
- Owner-direct attested credential injection.
- Multi-operator support with attributed audit logs.
