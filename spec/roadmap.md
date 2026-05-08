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

## Release Posture

The current codebase is publishable as a `0.x` preview of the proxy, phantom token, route, and CLI workflow. It should not be described as production TEE-secure or as a v1 security release until the attestation, reproducibility, sealed storage, and verification pieces below are implemented.

Preview releases should be explicit about the trust boundary: today the gateway reduces credential exposure to clients and agents, but it does not yet prevent the host or infrastructure operator from reading upstream credentials. The first public release can be useful for feedback and integration testing, while v1 should be reserved for a gateway that can publish verifiable release artifacts and be checked by `gate verify`.

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

Reproducibility groundwork for the server gateway must come before making strong Phala deployment claims. The priority is a pinned, reproducible gateway binary and packaged gateway image that produce published golden digests and measurement material. The `gate` CLI then consumes those materials as the verifier:

- `gate verify <gateway-url>` checks live attestation evidence against the published golden measurements.
- `gate verify --reproduce <gateway-url>` locally rebuilds the server gateway from source with pinned inputs, including a reproducible `cargo build`, and verifies that the reproduced binary/image material matches the published release exactly.

CLI artifact reproducibility is useful release hygiene, but it is secondary to server gateway reproducibility because the server gateway is the code running inside the TEE and handling upstream credentials.

The release-hardening items below are tentative and should be refined based on scope, CI complexity, and what `gate verify` needs to consume. Prefer publishing simple, inspectable artifacts first; add heavier supply-chain frameworks when they materially improve verifier trust without obscuring the core TEE measurement story.

- [ ] Pin the Rust toolchain used for release builds.
- [ ] Pin the gateway base image by digest.
- [ ] Add CI that builds the server gateway binary with locked inputs and records the binary digest as a golden release artifact.
- [ ] Add CI that packages the gateway binary into a GHCR image and records the image digest and expected measurement material as golden release artifacts.
- [ ] Evaluate build provenance for release artifacts, such as SLSA provenance or GitHub artifact attestations, if it fits the verification model.
- [ ] Add a reproduction script that locally rebuilds the server gateway release and verifies that the reproduced binary/image proof material matches the published golden artifacts.
- [ ] Add CI/release coverage for the `gate` CLI artifact with locked inputs and a published artifact digest after the gateway verification path is defined.
- [ ] Add unauthenticated `GET /attest`.
- [ ] Fetch a TDX QuoteV4 from tappd and return the quote as hex JSON.
- [ ] Accept `?nonce=<hex>` and bind it into `reportData`.
- [ ] Surface TDX debug mode in the response and logs.
- [ ] Return `503` from `/attest` when tappd is unavailable while keeping proxy routes available.
- [ ] Document a first Phala Cloud TDX deployment path.

Acceptance: a deployed gateway exposes `/attest`, a non-debug quote can be retrieved, and release artifacts publish the golden gateway binary digest, image digest, and measurement material that future `gate verify` work will check.

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
- [ ] Add a heavier `gate verify --reproduce` path that locally rebuilds the server gateway with a reproducible `cargo build`, reproduces the packaged release material, and verifies a 100% match against the published golden artifacts before comparing expected measurements against live enclave evidence.
- [ ] Write `docs/DEPLOY.md` for a complete Phala deployment.
- [ ] Extend e2e coverage for expiry, audit entries, redaction, and sealed credential behavior.
- [ ] Publish v1 release notes with server gateway binary digest, gateway image digest, expected measurement material/MRTD, and CLI artifact digest.

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
