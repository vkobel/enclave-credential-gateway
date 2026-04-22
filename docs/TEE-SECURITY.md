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

## Post-v1: Attested Credential Injection (no operator trust required)

> Replaces the v1 env-var injection path (R6 mitigation) with a model where
> credentials are injected by the owner's own device after the device has
> independently verified the enclave. No env var surface. Phala (or any
> operator) cannot see the credential, even at injection time.

### The problem with v1 injection

In v1, credentials enter the CVM via Phala's secret injection channel —
an encrypted channel that Phala controls and that gates delivery on MRTD
match. This is solid for the storage-at-rest guarantee (K3), but two
weaknesses remain:

1. **Phala sees the plaintext at injection time** — the secret is encrypted
   _to_ the CVM, but Phala's infrastructure is the carrier. A compromised
   injection path exposes the secret in transit.
2. **The env-var surface** — even if the channel is clean, the value lands
   as an env var inside the guest, which is outside the measured binary
   (the R6 gap documented above).

Both weaknesses share the same root: the credential owner is not the one
doing the injection — a third-party infrastructure operator is.

### The post-v1 model: owner-direct attested injection

Inspired by two sources:

- **Turnkey QoS** — Quorum of Signers protocol: each operator independently
  verifies the enclave attestation, ECDH-encrypts their secret share to the
  enclave's freshly generated ephemeral public key, and posts the encrypted
  share. Reconstruction happens inside the enclave in RAM. No operator, no
  infrastructure provider, and no single network path ever holds the
  plaintext. The ephemeral key is embedded in the attestation document
  itself — so encrypting to it _is_ attesting to the target.
- **d-inference (EigenInference/Darkbloom)** — the client is itself a
  hardened, attested binary (Secure Enclave-backed on Apple Silicon, SIP +
  Hardened Runtime locked, challenge-response verified). The client verifies
  the provider's attestation before submitting any secret. The trust model
  is peer attestation: two attested environments negotiating a shared secret,
  neither trusting the operator in between.

### Protocol design for CoCo

CoCo's version adapts this to TDX + a personal gateway:

**Participants:**
- `GW` — the CoCo gateway running inside the Phala TDX CVM
- `CLIENT` — the credential owner's device: a mobile app, a `coco` CLI on a
  laptop (potentially with its own Secure Enclave key on Apple Silicon), or
  any party holding the credential

**Flow:**

```
1. GW boots → generates a fresh ephemeral X25519 key pair (EPK_pub, EPK_priv)
              EPK_pub is included in the TDX reportData field at attestation time
              (hashed alongside the nonce: reportData = SHA256(nonce ‖ EPK_pub))

2. CLIENT calls GET /attest?nonce=<N>
   CLIENT receives TDX QuoteV4

3. CLIENT verifies the quote:
   a. Intel PCS signature check
   b. Debug bit unset
   c. reportData = SHA256(nonce ‖ EPK_pub) — confirms EPK_pub is bound to
      this specific, attested boot of this specific binary (MRTD match)
   d. MRTD matches pinned value from release notes
   e. Quote TTL within 5 minutes

4. CLIENT is now convinced:
   - The correct binary is running (MRTD)
   - The EPK_pub was generated by that binary on this boot (reportData binding)
   - No MITM can substitute a different public key without breaking the quote

5. CLIENT derives a shared secret:
   shared = X25519(CLIENT_ephemeral_priv, EPK_pub)
   Encrypts the credential value with AES-256-GCM keyed from the shared secret
   Sends: { ciphertext, CLIENT_ephemeral_pub, credential_name, nonce } to
   POST /admin/credentials/inject

6. GW decrypts inside the enclave:
   shared = X25519(EPK_priv, CLIENT_ephemeral_pub)
   credential_value = AES-256-GCM-Decrypt(shared, ciphertext)
   Seals to /data/credentials.enc
   EPK_priv is zeroed from memory

Result: Phala's infrastructure never held the plaintext. The credential
transited from CLIENT memory directly into enclave memory, encrypted end-to-end
by a key that only existed for the duration of this exchange.
```

**KRAB impact:** This moves the K score from K3 to K4 (secret release
enforced at the session level, not just at the KMS policy level), and
eliminates the env-var measurement gap (R6 is fully resolved — no host-
controlled input carries the credential).

New vector: `A2[Phala TDX] | R[f0/o1/l4/a4] | B2 | K4`

### Quorum extension (optional, for multi-party control)

If the credential is particularly sensitive (e.g. an org-wide API key shared
across a team), Shamir's Secret Sharing can be layered on top:

- The credential value is split K-of-N at the owner's device before injection
- Each of N parties independently verifies the attestation and injects their
  share via the protocol above (each share individually ECDH-encrypted to EPK_pub)
- The gateway collects shares and reconstructs inside the enclave only when
  K shares have arrived
- No single party can reconstruct the credential; no single injected share
  is the credential

This is the Turnkey QoS model applied to CoCo's gateway rather than to a
cryptographic key management system. The engineering lift is the Shamir
implementation inside the gateway (or a client-side library that the `coco`
CLI ships). The protocol is otherwise identical to the single-party case.

### Client surface options

| Client | Attestation verification | Injection transport | Notes |
|---|---|---|---|
| `coco inject` CLI (laptop) | Intel PCS via `dcap-rs` | HTTPS POST to `/admin/credentials/inject` | Available immediately post-v1; no hardware dependency |
| Mobile app (iOS/Android) | Intel PCS API call from the app | HTTPS POST | Verification happens on-device; user sees MRTD + binary reference before confirming |
| Apple Silicon `coco` CLI | Secure Enclave-backed CLIENT key; attested via MDA chain (d-inference model) | HTTPS POST | Peer attestation: both ends attested. Highest assurance. |
| Multi-party `coco inject --quorum` | Each party independently verifies and injects a share | N separate HTTPS POSTs | Threshold reconstruction inside enclave |

### Why this is the right long-term model

The v1 Phala injection channel is a trust delegation — it works, it's
acceptable for personal use, and it's honest in the KRAB vector. But it
means the security guarantee is "Phala promises not to look." The post-v1
model makes that promise unnecessary: **the credential is physically
unreadable by Phala** because it arrives encrypted to a key that only the
enclave generated, bound to the enclave's own attestation. The operator
becomes a dumb network carrier. This is the difference between a promise
and a proof.

It also makes the injection flow auditable by the user in a way they can
actually verify: the `coco verify` step is not a separate action before
injection — it is structurally part of injection. You cannot inject without
verifying. The UX collapses to a single command:

```bash
coco inject openai sk-proj-…
# Fetches attestation, verifies MRTD, derives shared key, encrypts, posts.
# Prints: Verified MRTD a3f9… matches pinned release. Credential sealed.
```

---

## References

- [KRAB Framework](https://github.com/vkobel/coco-krab-framework) — the scoring model used above
- [Turnkey Whitepaper](https://whitepaper.turnkey.com) — QoS protocol, ephemeral key binding in attestation, Shamir share injection, Boot Proof / App Proof model
- [d-inference / EigenInference (Layr-Labs)](https://github.com/Layr-Labs/d-inference) — peer attestation model (attested client + attested server), Secure Enclave-backed client identity, challenge-response verification
- [Edgeless Systems — Reproducible Builds for Confidential Computing](https://www.edgeless.systems/blog/reproducible-builds-for-confidential-computing) — rationale for why reproducibility is required for meaningful attestation
- [Trail of Bits — Enhancing Trust for SGX Enclaves](https://blog.trailofbits.com/2024/01/26/enhancing-trust-for-sgx-enclaves/) — Nix-based reproducible enclave builds
- [Trail of Bits — WhatsApp Private Processing Audit](https://blog.trailofbits.com/2026/04/07/what-we-learned-about-tee-security-from-auditing-whatsapps-private-inference/) — post-boot env var injection as measurement gap (informs R6)
- [Intel TDX Documentation](https://www.intel.com/content/www/us/en/developer/tools/trust-domain-extensions/documentation.html) — MRTD, RTMR register semantics
- [Phala dstack](https://github.com/Phala-Network/dstack) — CVM runtime, tappd sidecar, secret injection
