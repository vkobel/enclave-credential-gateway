# CoCo Gateway — TEE Security

> Platform: Phala Cloud TDX CVM (v1 target).
> Framework: [KRAB](https://github.com/vkobel/coco-krab-framework) for scoring attestation, reproducibility, session binding, and key release.

---

## KRAB Target Vector

```
A2[Phala TDX] | R[f0/o1/l4/a4] | B2 | K3
```

| Dimension | Score | Meaning |
|---|---|---|
| **A — Attestation** | `A2[Phala TDX]` | Silicon-rooted TDX quote. Phala's dstack paravisor sits in the launch TCB — a conscious trust delegation to Phala. |
| **R — Reproducibility** | `R[f0/o1/l4/a4]` | Firmware opaque (CSP-controlled `f0`); dstack OS source-available but not yet independently reproducible (`o1`); base Docker image pinned by SHA256 digest (`l4`); gateway binary reproducible from locked toolchain (`a4`). |
| **B — Session Binding** | `B2` | `GET /attest?nonce=<hex>` hashes the caller nonce into `reportData`. `coco verify` checks the nonce and enforces a 5-minute quote TTL. Replay of a prior quote to a different session is structurally prevented. |
| **K — Key Release** | `K3` | Phala KMS releases the sealing key only to a CVM whose MRTD matches the registered value. Debug-mode quotes are rejected. |

**Status:** `GET /attest` and the attestation verification flow are Phase 1b (not yet implemented). The proxy and token registry work today. See [spec/roadmap.md](../spec/roadmap.md).

This is a **conscious trust delegation** to Phala at the A and K layers — not a structural weakness. Each gap is documented here, not hidden.

**Post-v1 target:** `A2[Phala TDX] | R[f0/o2/l4/a4] | B2 | K4` — upgrade `o` when dstack OS becomes maintainer-signed; upgrade `K` to K4 when secret injection is owner-direct (see below).

---

## Threat Model

**What the TEE boundary defends against:**

- Phala operator or infrastructure admin reading credentials stored in the CVM. ✅ Plaintext keys never exist outside enclave memory.
- User accidentally exposing credentials via `cat`, logs, or shell history. ✅ Keys are sealed inside the TEE; the CLI transmits them over the admin API and they are encrypted immediately.
- Compromised agent process reading credentials from the host environment. ✅ Agents hold only phantom tokens; the real credential is never in agent process space.
- Prompt injection causing an agent to exfiltrate credential values. ✅ Structural prevention — credentials are never in agent memory.
- Supply chain attack substituting a backdoored binary. ✅ Mitigated by reproducible builds: an independent verifier can rebuild and compare MRTD.

**What this does NOT defend against:**

- A malicious agent making calls that are within its phantom's scope (by design).
- Phala serving a modified dstack paravisor (A2 trust delegation — accepted consciously).
- Side-channel attacks (TDX mitigations exist but are not a CoCo concern).
- Post-boot env var injection (see Measurement Gap section).

---

## Security Requirements

### R1 — TDX Attestation Endpoint

`GET /attest` must return a valid Intel TDX QuoteV4 from the tappd sidecar:
- Caller supplies `?nonce=<hex>`; gateway hashes it into `reportData` (first 32 bytes = `SHA256(nonce)`).
- If `td_attributes` bit 0 is set (debug mode): log `ERROR`, include `"debug": true` in response.
- Return `503` when tappd is unreachable; other routes continue working.
- Endpoint is unauthenticated — it is the public proof surface.

### R2 — Reproducible Binary

Any independent party must be able to rebuild the `coco-gateway` binary and arrive at the same MRTD:
- Rust toolchain pinned in `rust-toolchain.toml` (exact version, not `stable`).
- Docker base image pinned by SHA256 digest — no floating tags.
- `Cargo.lock` committed; CI uses `cargo build --locked`.
- GitHub Actions workflow builds and pushes to GHCR on every push to `main`.
- Each release publishes the Docker image digest and the derived MRTD.
- `scripts/reproduce.sh`: given a git commit SHA, builds the image locally and prints the MRTD for comparison.

### R3 — Sealed Credential Storage (Phase 3)

Credentials will be encrypted at rest with AES-256-GCM. The encryption key is derived inside the TEE via Phala's KMS (key released only to a CVM with the correct MRTD). Stored at `/data/credentials.enc`; credential values never appear in API responses, logs, or audit entries.

### R4 — Admin Token

The admin token is set via `COCO_ADMIN_TOKEN` at deploy time. Its Blake3 hash is what the gateway stores and compares (constant-time) on every `/admin/*` request. The plaintext never persists beyond the shell environment.

### R5 — Client Attestation Verification (`coco verify`)

`coco verify <gateway-url>` (Phase 3) performs:
1. Generate a random nonce locally.
2. Call `GET /attest?nonce=<hex>`.
3. Verify the TDX QuoteV4 signature against Intel PCS.
4. Assert `td_attributes` debug bit is unset.
5. Verify `reportData` contains `SHA256(nonce)` — confirms the quote is fresh for this session.
6. Check quote timestamp is within TTL (5 minutes).
7. Compare MRTD to pinned value from `~/.config/coco/config.toml` or `--mrtd` flag.
8. Print: MRTD, GHCR image digest, timestamp, pass/fail.

### R6 — Measurement Gap: Post-Boot Inputs

The binary is measured correctly, but runtime inputs (env vars, injected secrets) are not part of the MRTD. Specific risks:

- **Phala injects secrets as env vars.** If an attacker substitutes a malicious `COCO_ADMIN_TOKEN` at boot, they compromise the admin surface without touching the binary. Mitigation (v1): validate all env vars at startup; log the names (not values) of all vars consumed at boot.
- **`docker-compose.yml` is not measured.** Only the image digest is part of the MRTD. A modified compose file could inject unexpected env vars. Mitigation (v1): pin the compose file SHA in `coco verify` output; document that users should compare the deployed compose file against the published version.

**Post-v1 mitigation:** owner-direct attested credential injection — the credential owner's device verifies the enclave attestation, then ECDH-encrypts the credential to the enclave's freshly generated ephemeral public key (embedded in `reportData`). The credential transits directly into enclave memory, encrypted end-to-end. Phala's infrastructure is a dumb carrier; it never holds the plaintext. This upgrades K3 → K4 and eliminates the env-var measurement gap.

---

## Reproducible Build Pipeline

```
Source (vkobel/coco-credential-gateway @ git SHA)
    │
    ▼
rust-toolchain.toml  ← pinned exact version
Cargo.lock           ← committed, all deps locked
    │
    ▼
GitHub Actions (ubuntu-24.04, pinned runner image)
    │  cargo build --locked --release --target x86_64-unknown-linux-musl
    ▼
static musl binary: coco-gateway
    │
    ▼
Dockerfile  ← FROM distroless/static@sha256:<pinned>
    │  COPY coco-gateway /app/coco-gateway
    ▼
Docker image  → pushed to GHCR with SHA256 digest
    │
    ▼
Phala CVM launch  ← image digest registered with Phala KMS
    │  dstack measures the image into MRTD at boot
    ▼
MRTD  → published in GitHub release notes
    │
    ▼
coco verify  ← any independent party runs this
```

**Why musl?** No dynamic library dependency on the host OS. The binary's behavior is fully determined by its source and Rust toolchain version. glibc introduces an uncontrolled variable; musl removes it.

---

## KRAB Scorecard

| Dimension | Score | Justification |
|---|---|---|
| **A** | `A2[Phala TDX]` | TDX silicon root of trust. Phala's dstack paravisor in launch TCB — conscious delegation. |
| **R** | `R[f0/o1/l4/a4]` | Firmware opaque (`f0`). dstack OS source-available, not reproducible yet (`o1`). Base image pinned by digest (`l4`). Gateway binary reproducible from locked toolchain (`a4`). |
| **B** | `B2` | Nonce hashed into `reportData`; `coco verify` enforces TTL + nonce match. |
| **K** | `K3` | Phala KMS gates key release on MRTD match. Debug quotes rejected. No session-level binding at KMS (that's K4, post-v1). |

---

## References

- [KRAB Framework](https://github.com/vkobel/coco-krab-framework)
- [Intel TDX Documentation](https://www.intel.com/content/www/us/en/developer/tools/trust-domain-extensions/documentation.html) — MRTD, RTMR register semantics
- [Phala dstack](https://github.com/Phala-Network/dstack) — CVM runtime, tappd sidecar, secret injection
- [Edgeless Systems — Reproducible Builds for Confidential Computing](https://www.edgeless.systems/blog/reproducible-builds-for-confidential-computing)
- [Trail of Bits — WhatsApp Private Processing Audit](https://blog.trailofbits.com/2026/04/07/what-we-learned-about-tee-security-from-auditing-whatsapps-private-inference/) — post-boot env var injection as measurement gap
