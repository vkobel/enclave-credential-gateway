# Enclave Credential Gateway - Roadmap

## Phase Status

| Phase | Description | Status |
|---|---|---|
| **1a** | Phantom token auth, profile routing, multi-source credential injection | done |
| **1b** | Docker/Caddy deployment scaffold, token registry, CLI activation | done |
| **1c** | StageX reproducible OCI builds done; Caution/Nitro deployment and CI image publishing next | in progress |
| **2** | Audit log, token expiry, response redaction, additional route profiles | not started |
| **3** | Attested credential provisioning, `gate verify`, deploy polish, v1 release | not started |

---

## Release Posture

The current codebase is publishable as a `0.x` preview of the proxy, phantom token, route, CLI workflow, and StageX reproducible OCI artifact workflow. It should not be described as production TEE-secure or as a v1 security release until the Caution/Nitro deployment, attested credential provisioning, published PCRs, and verification pieces below are exercised end to end.

Preview releases should be explicit about the trust boundary: today the gateway reduces credential exposure to clients and agents. Protection from the host or infrastructure operator depends on a Caution/Nitro deployment with Locksmith-provisioned secrets. The first public release can be useful for feedback and integration testing, while v1 should be reserved for a gateway whose PCRs can be reproduced from source (`caution verify`, and later `gate verify`) and whose credentials are provisioned without plaintext exposure.

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

**Reproducible build groundwork:**

- `Containerfile.stagex` builds server and CLI OCI artifacts through the pinned [StageX](https://codeberg.org/stagex/stagex) Rust pallet.
- Runtime images are `scratch` images containing only the statically linked binary.
- The release compile runs after dependency fetch with `--network=none`, `--frozen`, and `--release`.
- `scripts/build-stagex-oci.sh` exports timestamp-normalized OCI tarballs for both artifacts.
- `docs/BUILDING.md` and the README publish expected linux/amd64 OCI tarball hashes and reproduction commands.
- Current linux/amd64 server and CLI OCI tarballs have been verified with byte-for-byte cached and no-cache rebuilds.

---

## Phase 1c - Caution Deployment and Release Groundwork

**Goal:** make it possible for a verifier to reproduce the running gateway's measurements from source and confirm they match the live enclave.

Attestation itself is provided by the Caution platform, not the gateway. On Caution's current AWS Nitro backing, `bootproofd` serves an NSM attestation document at `/attestation`, PCR0/PCR1/PCR2 measure the image/kernel/app, and `caution verify` reproduces the enclave from the manifest's pinned source commits and compares measurements. The gateway implements no attestation endpoint of its own. The remaining priority is to exercise the Caution deployment, publish golden measurements, and bring the verifier capability into `gate`.

StageX reproducible OCI artifacts are the deterministic foundation that makes the EIF PCRs reproducible. Server gateway measurement remains the security-critical path because the server is the code running inside the enclave and handling upstream credentials.

The release-hardening items below are tentative and should be refined based on scope and CI complexity. Prefer publishing simple, inspectable artifacts first.

- [x] Pin the release build environment with a StageX Rust pallet digest.
- [x] Package the server gateway and CLI as reproducible linux/amd64 OCI tarballs.
- [x] Record per-commit expected OCI tarball hashes in an annotated git tag (`build-stagex-oci.sh --tag <name>`), verified with `--check`.
- [x] Add no-cache reproduction commands that rebuild and compare the StageX OCI artifacts.
- [x] Provide a Caution-ready `Procfile` (server image, `http_port`, `app_sources`, `locksmith`).
- [ ] linux/amd64 is the supported reproducible target. arm64 is deferred: the pinned StageX rust pallet is amd64-only (single-arch image, not a multi-platform index), so arm64 would require building and pinning a self-built StageX arm64 pallet, not just adding a digest.
- [ ] Deploy to Caution/Nitro and record the resulting PCR0/PCR1/PCR2 as golden release values.
- [ ] Confirm `caution verify --attestation-url <url>` reproduces the deployed PCRs from source.
- [ ] Add CI that builds the server gateway binary with locked inputs and records the binary digest as a golden release artifact.
- [ ] Evaluate build provenance for release artifacts, such as SLSA provenance or GitHub artifact attestations, if it fits the verification model.
- [ ] `gate verify` reaches parity with `caution verify`: fetch `/attestation`, reuse `bootproof-sdk` to check the AWS Nitro root CA chain, COSE signature, nonce, and PCR match, and reject debug-mode (all-zero PCR) evidence; perform the reproduce step via Caution's `enclave-builder` or by shelling out to `caution verify`.
- [ ] Document a first Caution/Nitro deployment path.

Acceptance: a deployed gateway is reachable through Caution with PCRs that `caution verify` reproduces from source, and the release publishes the golden gateway binary digest and PCR values that `gate verify` will check.

---

## Phase 2 - Policy, Audit Log, and More Profiles

- [ ] Add per-token expiry and reject expired tokens.
- [ ] Add append-only request audit log with token name, route, method, upstream status, byte counts, and policy action.
- [ ] Add `GET /admin/audit` with simple limit and token filters.
- [ ] Add `gate audit tail`.
- [ ] Add response body credential redaction for upstream credential echoes.
- [ ] Add additional route profiles after tests are in place: Groq, ElevenLabs, Telegram, Together, and Ollama.

---

## Phase 3 - Attested Credentials and v1 Release

- [ ] Add owner-direct attested credential injection over steve (`e2e: true`): `gate creds push` runs the verifier (PCR match), then sends credentials over steve's attestation-bound channel into enclave RAM. Replaces Locksmith for the solo operator; Locksmith remains the quorum/multi-operator path.
- [ ] Add admin credential management endpoints that never return credential values.
- [ ] Add `gate creds add/rotate/rm/ls`.
- [ ] Add a heavier `gate verify --reproduce <gateway-url>` path that locally rebuilds the EIF (via Caution's `enclave-builder` or by shelling out to `caution verify`) and confirms the reproduced PCR0/PCR1/PCR2 match the live enclave before trusting it.
- [ ] Write `docs/DEPLOY.md` for a complete Caution/Nitro deployment.
- [ ] Extend e2e coverage for expiry, audit entries, redaction, and attested credential push.
- [ ] Publish v1 release notes with server gateway binary digest and golden PCR0/PCR1/PCR2 values, plus the CLI artifact digest.

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
- Multi-operator quorum provisioning via Locksmith, with attributed audit logs.
