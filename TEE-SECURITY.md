# CoCo Gateway — TEE Security Requirements

> Evaluated against the [KRAB framework](https://github.com/vkobel/coco-krab-framework).
> Platform: Phala Cloud TDX CVM (v1). Reproducibility model: inspired by Turnkey's StageX whitepaper.

---

## KRAB Target Vector

```
A2[Phala TDX] | R[f0/o1/l4/a4] | B2 | K3
```

| Dimension | v1 Score | Meaning |
|---|---|---|
| **A — Attestation** | `A2[Phala TDX]` | Silicon-rooted TDX quote, but Phala's dstack paravisor sits in the launch TCB — conscious trust delegation to Phala |
| **R — Reproducibility** | `R[f0/o1/l4/a4]` | Firmware opaque (CSP-controlled); OS `o1` (dstack OS is source-available, not fully reproducible yet); base Docker image `l4` pinned by SHA256 digest; gateway binary `a4` reproducible via locked toolchain |
| **B — Session Binding** | `B2` | Fresh nonce hashed into attestation `reportData`; `coco verify` enforces quote TTL and compares MRTD to pinned value |
| **K — Key Release** | `K3` | Phala KMS gates secret release on exact MRTD match; debug-mode quotes rejected; no dynamic session binding in the KMS path (B2 is enforced at the client verifier, not the KMS) |

This is a **conscious trust delegation** to Phala at the A and K layers. The attestation chain is not structurally broken — it is deliberately scoped. The weakness is explicit, not hidden.

**Post-v1 target vector:** `A2[Phala TDX] | R[f0/o2/l4/a4] | B2 | K4` — upgrade `o` to R2 when dstack OS becomes maintainer-signed, upgrade K to K4 when session binding is enforced at the KMS policy level.

---

## Threat Model

What CoCo's TEE boundary defends against:

- **Phala operator / infrastructure admin** reading credentials stored in the CVM. ✅ Hardware boundary: plaintext keys never exist outside the enclave memory.
- **User accidentally exposing credentials** via `cat`, logs, or shell history. ✅ Keys are sealed inside the TEE; the CLI transmits them over the admin API and they are immediately encrypted at rest.
- **Compromised agent process** reading credentials from the host environment. ✅ The agent holds only a phantom token; the real credential is never in the agent's process space.
- **Prompt injection** causing an agent to exfiltrate credential values. ✅ Structural prevention — credentials are never in agent memory to begin with.
- **Supply chain attack** substituting a backdoored gateway binary. ✅ Mitigated by reproducible builds: an independent verifier can rebuild the binary and compare MRTD. Attacker must also compromise the TDX hardware or the Intel attestation chain.

What this does **not** defend against (explicitly out of scope):

- A malicious agent making authorized calls (it has a phantom and can call within its scoped permissions — this is by design).
- Phala themselves serving a modified dstack paravisor (A2 trust delegation — accepted consciously).
- Side-channel attacks (TDX mitigations exist but are not a CoCo concern).
- Post-boot environment injection into the guest kernel via hypervisor ACPI tables or injected env vars (see measurement gap section below).

---

## Requirements

### R1 — TDX Attestation Endpoint

- The gateway **MUST** expose `GET /attest` returning a valid Intel TDX QuoteV4 signed by the platform's attestation key.
- The quote **MUST** be fetched from the tappd sidecar (Phala's TEE attestation proxy daemon) via `POST /prpc/Tappd.TdxQuote`.
- The quote **MUST** include a caller-supplied `nonce` hashed into the `reportData` field (the first 32 bytes of `reportData` = `SHA256(nonce)`). This is the B2 session binding hook: the client generates the nonce, so the quote cannot be replayed against a different session.
- The response **MUST** include `"debug": true` and a logged `ERROR` if `td_attributes` bit 0 is set (debug mode). The gateway **SHOULD** continue serving other routes but the presence of a debug quote **MUST** be surfaced to the client.
- `coco verify` **MUST** reject any quote where `"debug": true`.
- `coco verify` **MUST** enforce a quote TTL (default: 5 minutes) — a quote older than TTL is rejected even if the signature is valid.
- `coco verify` **MUST** compare the MRTD in the quote to a locally pinned MRTD value. MRTD mismatch is a hard failure.
- `GET /attest` is unauthenticated; it is the public proof surface.

### R2 — Reproducible Application Binary

The goal: any independent party must be able to rebuild the `coco-gateway` binary, derive the same MRTD, and confirm the running code matches the published source.

Per the Turnkey whitepaper: _"This is the difference between proving 'this enclave runs this binary digest' and 'this enclave runs this source code.'"_ A non-reproducible build is an inherent single point of failure — the party who compiled must be trusted unconditionally.

Requirements:

- The gateway binary **MUST** be built with a pinned, version-locked Rust toolchain (`rust-toolchain.toml` committed to the repo at exact channel+version, not `stable`).
- The Docker image **MUST** use a distroless or Alpine base image pinned by SHA256 digest (e.g. `FROM gcr.io/distroless/static@sha256:…`). No floating tags (`latest`, `alpine`).
- All Rust dependencies **MUST** be locked in `Cargo.lock` (committed, not gitignored). `cargo build --locked` is mandatory in CI.
- The GitHub Actions build workflow **MUST** reproduce the binary in a clean environment and push to GHCR. The workflow itself is part of the verifiable surface — its SHA is recorded.
- The resulting Docker image digest **MUST** be published alongside each release. The MRTD computed from that image is the pinned value that `coco verify` checks.
- A `scripts/reproduce.sh` **MUST** exist: given a git commit SHA, it builds the image locally and prints the derived MRTD for comparison against the published value.
- Reproducibility is tracked on every release. A build that produces a different digest from a clean rebuild on a second machine is a release blocker.

**v1 achievable grade: `a4`** — the application layer is fully reproducible. The OS and firmware layers remain opaque (Phala-controlled), which is the accepted trust delegation.

### R3 — Sealed Credential Storage

- Credentials **MUST** be encrypted at rest using AES-256-GCM.
- The encryption key **MUST** be derived inside the TEE, never transmitted outside it. Two approaches for v1:
  - **Phala secret injection** (simpler): Phala's KMS derives a CVM-specific key from the MRTD and injects it as an environment variable at boot via the sealed secrets channel. The key is only available to a CVM with the correct MRTD. This is the v1 path.
  - **Enclave-sealed random key** (alternative): Generate a random 256-bit key at first boot, seal it using Phala's KMS (which gates retrieval on MRTD match), persist the sealed blob. Retrieve and unseal on restart. This is the v1.x upgrade path for portability.
- Credentials **MUST** be stored at `/data/credentials.enc` on a persistent volume. The volume is Phala-managed and encrypted at the hypervisor layer.
- Credential names **MUST** be listable via the admin API; credential values **MUST NEVER** appear in API responses, logs, or audit entries.
- The admin API endpoint that accepts credential values (`POST /admin/credentials`) **MUST** operate over TLS only. Plaintext credential submission is not permitted.
- Response body redaction: if an upstream API echoes a credential value in its response body, the gateway **MUST** replace the value with `[REDACTED_BY_COCO]` before forwarding to the caller.

### R4 — Secret Bootstrap and Admin Token

- The admin token **MUST** be generated with a CSPRNG inside the enclave at first boot.
- It **MUST** be printed once to stdout at first boot only (`ADMIN TOKEN: <hex>`). Never stored in plaintext; only its BLAKE3 or SHA-256 hash persisted at `/data/admin.hash`.
- All `/admin/*` routes **MUST** require `Authorization: Bearer <admin-token>` validated with constant-time comparison.
- Phala secret injection (`phala cvms secrets set`) is used to pass the initial admin token or sealing key to the CVM over an attested channel. This means the value is only readable inside a CVM whose MRTD matches the registered value — operator's laptop never holds it in plaintext after injection.

### R5 — Attestation Verification by Clients (`coco verify`)

Following the Turnkey Boot Proof model — "App Proofs link back to Boot Proofs which link back to source code":

- `coco verify <gateway-url>` **MUST** perform the following chain:
  1. Generate a random nonce locally.
  2. Call `GET /attest?nonce=<hex>`.
  3. Verify the TDX QuoteV4 signature against Intel PCS (via `dcap` or Intel's verification service).
  4. Assert `td_attributes` debug bit is unset.
  5. Verify `reportData` contains `SHA256(nonce)` — confirms the quote was produced for this specific session, not replayed.
  6. Check quote timestamp is within TTL (5 minutes).
  7. Compare MRTD in quote to pinned value (from `~/.config/coco/config.toml` or `--mrtd` flag).
  8. Print result: MRTD, binary reference (GHCR digest), timestamp, pass/fail.
- On first use (`coco verify --trust-on-first-use`), the MRTD is stored locally as the pinned value for future checks. TOFU is documented as a risk: an attacker who intercepts the first connection can substitute a malicious MRTD. Mitigation: cross-check the MRTD against the published release in the GitHub repo before trusting.
- `coco token create` **SHOULD** embed the verified MRTD in the token record at creation time. A client verifying later can confirm the same binary signed the token.

### R6 — Measurement Gap: Post-Boot Inputs

The Turnkey whitepaper and the Trail of Bits WhatsApp TEE audit both document the same fracture pattern: the binary is correctly measured, but runtime inputs (env vars, injected config) are not. A host-controlled input consumed after the measurement point can alter behavior undetectably.

For CoCo, the specific risks and mitigations:

- **Risk:** Phala injects secrets via env vars at boot. If an attacker can inject a malicious `COCO_ADMIN_HASH` or `COCO_SEAL_KEY` env var, they compromise the admin surface without touching the binary.
  - **Mitigation (v1):** Validate all injected env vars at startup against expected types and ranges. Log the names (not values) of all env vars consumed at boot. Include the hash of the startup env var manifest in the audit log entry written at first boot.
  - **Mitigation (v1.x):** Move away from env var injection for secret material; use Phala's attested KMS channel directly from within the binary (the binary calls the KMS API over vsock, presents its own quote, and receives the unsealing key). No env var surface at all.
- **Risk:** The `docker-compose.yml` / container config is not part of the measured MRTD on Phala TDX CVMs — only the image digest is measured. A modified compose file could inject env vars or mount points not present in the reference deployment.
  - **Mitigation (v1):** Pin the compose file SHA in the `coco verify` output. Document that users should compare the compose file at the deployed CVM against the published version. This is a manual step in v1.
  - **Mitigation (v1.x):** Move compose file content into the image itself (baked-in entrypoint, no external config consumed at runtime). This brings the config surface inside the measured image boundary.

---

## Reproducible Build Pipeline (Proposed)

Inspired by Turnkey's StageX approach — the goal is `a4` on the application layer for v1.

```
Source (vkobel/coco-credential-gateway @ git SHA)
    │
    ▼
rust-toolchain.toml  ←  pinned channel + version (e.g. 1.87.0)
Cargo.lock           ←  committed, all transitive deps locked
    │
    ▼
GitHub Actions (ubuntu-24.04, pinned runner image SHA)
    │  cargo build --locked --release --target x86_64-unknown-linux-musl
    ▼
static binary: coco-gateway  (musl, no dynamic linking)
    │
    ▼
Dockerfile  ←  FROM gcr.io/distroless/static@sha256:<pinned>
    │  COPY coco-gateway /app/coco-gateway
    ▼
Docker image  →  pushed to GHCR with SHA256 digest published
    │
    ▼
Phala CVM launch  ←  image digest registered with Phala KMS
    │  dstack measures the image into MRTD at boot
    ▼
MRTD  →  published in GitHub release notes alongside image digest
    │
    ▼
coco verify  ←  independent party runs this
    │  rebuilds image via scripts/reproduce.sh
    │  compares derived MRTD to released MRTD
    ▼
PASS / FAIL
```

**Key properties of this pipeline:**

- **Musl static binary:** No dynamic library dependency on the host OS. The binary's behavior is fully determined by its source and the Rust toolchain version. Any glibc-linked binary inherits the OS's libc version as an uncontrolled variable — musl removes this.
- **Pinned base image digest:** `FROM distroless/static@sha256:…` is bit-for-bit stable. `FROM alpine:latest` is not. A floating tag can silently change the measured image.
- **Locked Cargo.lock:** `cargo build --locked` fails if `Cargo.lock` is inconsistent with `Cargo.toml`. No silent dependency drift.
- **Pinned CI runner:** The GitHub Actions runner image is pinned by SHA where possible. Non-deterministic runner environments are the most common reproducibility failure in practice.
- **`scripts/reproduce.sh`:** Anyone can run this script at a given commit and arrive at the same image digest. The script is the proof artifact.

**What `R[a4]` does not cover:**

- The Docker image's OS layer (distroless or Alpine base): we trust the upstream maintainer's digest. This is `l` territory — for v1 we accept `l4` by pinning the digest, accepting that we cannot independently bootstrap the base image.
- The Phala dstack OS and firmware: these are `o` and `f` — outside our build pipeline. KRAB scores these `o1` and `f0` as a declared trust delegation.

---

## KRAB Scorecard

| Dimension | Score | Justification |
|---|---|---|
| **A: Attestation** | `A2[Phala TDX]` | TDX silicon root of trust. Phala's dstack paravisor sits in the launch TCB — consciously accepted. Quote delivered via `/dev/tdx_guest` through tappd sidecar. Intel PCS verifies the hardware signature. |
| **R: Reproducibility** | `R[f0/o1/l4/a4]` | **f0:** Phala firmware is CSP-controlled, not independently verifiable. **o1:** dstack OS is source-available (GitHub) but not yet deterministically buildable by an independent party. **l4:** base Docker image pinned by SHA256 digest — bit-for-bit stable. **a4:** gateway binary reproducible from source via locked Rust toolchain + `scripts/reproduce.sh`. |
| **B: Session Binding** | `B2` | `GET /attest?nonce=<hex>` hashes caller nonce into `reportData`. `coco verify` checks nonce match + enforces 5-minute TTL. Replay of a prior quote to a different session is structurally prevented. |
| **K: Key Release** | `K3` | Phala KMS releases the sealing key only to a CVM whose MRTD matches the registered value. Debug-mode quotes explicitly rejected. No dynamic session binding enforced at the KMS layer (that would be K4 — post-v1 target). |

**KRAB Vector: `A2[Phala TDX] | R[f0/o1/l4/a4] | B2 | K3`**

This is a coherent, production-worthy profile. The gaps (f0, o1, K3 instead of K4) are all conscious trust delegations to Phala, not structural weaknesses in CoCo's design. Each gap is documented here rather than hidden.

---

## Upgrade Path (post-v1)

| Upgrade | Change | New score |
|---|---|---|
| Phala dstack OS becomes maintainer-signed | `o1 → o2` | `R[f0/o2/l4/a4]` |
| KMS session binding enforced at policy level | `K3 → K4` | `K4` |
| Move to bare-metal TDX (e.g. Equinix Metal) | `A2 → A3` | `A3` |
| Bootstrap base Docker image from source (Nix or similar) | `l4 → l4` (already strong; `l` is already R4 via pinned digest) | — |
| Publish multi-party signed MRTD set (2+ signers) | Strengthens trust in `a4` claim | `R2+` on app layer |

---

## References

- [KRAB Framework](https://github.com/vkobel/coco-krab-framework) — the scoring model used above
- [Turnkey Whitepaper — Verifiable Foundations](https://whitepaper.turnkey.com/foundations) — reproducible build approach (StageX, PCR derivation, Boot Proof / App Proof model)
- [Edgeless Systems — Reproducible Builds for Confidential Computing](https://www.edgeless.systems/blog/reproducible-builds-for-confidential-computing) — Bazel-based approach; rationale for why reproducibility is required for meaningful attestation
- [Trail of Bits — Enhancing Trust for SGX Enclaves](https://blog.trailofbits.com/2024/01/26/enhancing-trust-for-sgx-enclaves/) — Nix-based reproducible SGX enclave builds
- [Trail of Bits — WhatsApp Private Processing Audit](https://blog.trailofbits.com/2026/04/07/what-we-learned-about-tee-security-from-auditing-whatsapps-private-inference/) — post-boot env var injection as a measurement gap (informs R6 above)
- [Intel TDX Documentation](https://www.intel.com/content/www/us/en/developer/tools/trust-domain-extensions/documentation.html) — MRTD, RTMR register semantics
- [Phala dstack](https://github.com/Phala-Network/dstack) — CVM runtime, tappd sidecar, secret injection
