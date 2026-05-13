# Enclave Credential Gateway - TEE Security Target

> Platform target: Phala Cloud TDX CVM.
> Framework: [KRAB](https://github.com/vkobel/coco-krab-framework) for scoring attestation, reproducibility, session binding, and key release.

This document describes the target TEE security profile for v1. It is not a claim that the current repository already provides these guarantees.

## Current Implementation Status

Working today:

- Path-based gateway/proxy with OpenAI, Anthropic, and GitHub route profiles.
- Phantom token registry stored as Blake3 token hashes in `tokens.json`.
- Per-token scope enforcement before credential resolution.
- Admin API protected by `GATE_ADMIN_TOKEN`, compared constant-time in memory.
- Docker Compose + Caddy deployment scaffold.

Not implemented yet:

- `GET /attest` and client-side quote verification.
- Pinned reproducible release pipeline and MRTD publication.
- Sealed or encrypted credential store.
- Phala KMS key release integration.
- Audit log, credential redaction, and token expiry.

---

## Target KRAB Vector

```text
A2[Phala TDX] | R[f0/o1/l4/a4] | B2 | K3
```

| Dimension | Target score | Meaning |
|---|---|---|
| **A - Attestation** | `A2[Phala TDX]` | Silicon-rooted TDX quote. Phala's dstack paravisor sits in the launch TCB, which is a conscious trust delegation to Phala. |
| **R - Reproducibility** | `R[f0/o1/l4/a4]` | Firmware opaque (`f0`); dstack OS source-available but not independently reproducible (`o1`); base image pinned by SHA256 digest (`l4`); gateway binary reproducible from a locked toolchain (`a4`). |
| **B - Session Binding** | `B2` | `GET /attest?nonce=<hex>` hashes the caller nonce into `reportData`; `gate verify` checks nonce match and quote freshness. |
| **K - Key Release** | `K3` | Phala KMS releases the sealing key only to a CVM whose measurement matches the registered value. |

Post-v1 target: `A2[Phala TDX] | R[f0/o2/l4/a4] | B2 | K4`, with stronger dstack provenance and owner-direct credential injection.

---

## Threat Model

The target TEE boundary is designed to defend against:

- Infrastructure operators reading real credentials from gateway memory or storage.
- Agent processes reading real vendor keys from their own host environment.
- Prompt injection that asks an agent to reveal a key it never had.
- Supply-chain substitution of the gateway binary, once releases are reproducible and attested.

It does not defend against:

- A malicious agent making authorized calls within its phantom token scope.
- Phala serving a modified dstack paravisor, which is the accepted A2 trust delegation.
- TDX side channels.
- Runtime input attacks that occur after measurement, unless those inputs are separately validated or attested.

Current implementation note: today the gateway reduces credential exposure to clients and agents, but it does not yet protect credentials from the host or infrastructure operator because the TEE deployment and sealed storage pieces are not implemented.

---

## Security Requirements

### R1 - TDX Attestation Endpoint

`GET /attest` should return a valid Intel TDX QuoteV4 from the tappd sidecar:

- Caller supplies `?nonce=<hex>`; gateway hashes it into `reportData`.
- If `td_attributes` bit 0 is set, the gateway logs an error and returns `"debug": true`.
- If tappd is unreachable, `/attest` returns `503`; proxy routes continue serving.
- The endpoint is unauthenticated because it is the public proof surface.

### R2 - Reproducible Release Pipeline

The v1 release process should make the binary-to-source link independently checkable:

- Commit an exact Rust toolchain version.
- Build with `cargo build --locked`.
- Pin Docker base images by SHA256 digest.
- Publish GHCR image digests and derived MRTD values.
- Add a reproduction script that rebuilds a release image and prints comparable proof material.

The current `Dockerfile` is a development/deployment scaffold. `Containerfile.stagex`
and `scripts/build-stagex-oci.sh` are the first reproducible OCI build scaffold,
but the full release pipeline still needs published image digests, MRTD material,
and verification tooling.

### R3 - Sealed Credential Storage

The target credential store encrypts credentials at rest with a key released only to the expected TDX measurement. Credential values must never appear in API responses, logs, audit entries, or client config.

Current implementation note: vendor credentials are read from environment variables and injected into upstream requests. There is no sealed credential store yet.

### R4 - Admin Token

Current behavior: `GATE_ADMIN_TOKEN` is supplied at deploy time, held in memory, and compared constant-time for `/admin/*` requests.

Target hardening: store only a hash or sealed representation of the admin secret after startup, and include admin-token handling in the attested deployment story.

### R5 - Client Verification

`gate verify <gateway-url>` should support two levels of verification.

Normal mode:

1. Generate a fresh nonce.
2. Call `GET /attest?nonce=<hex>`.
3. Verify the TDX QuoteV4 signature against Intel PCS.
4. Assert the debug bit is unset.
5. Verify `reportData` binds the nonce.
6. Enforce quote freshness.
7. Compare MRTD to a pinned value or release artifact.
8. Print a pass/fail summary with the binary reference.

Reproducibility mode:

- Build the same release locally from the published source and lockfiles.
- Recreate the release image or equivalent measured artifact.
- Compare the local digest and expected measurement material against the published release.
- Compare that expected material to live enclave evidence, including MRTD/RTMR values and equivalent platform registers where applicable.

The normal path answers "is this gateway attested and fresh?" The heavier path answers "can I independently reproduce what this gateway claims to be running?"

### R6 - Measurement Gap: Post-Boot Inputs

The TDX measurement does not automatically cover runtime environment variables, compose files, mounted volumes, or injected secrets. v1 should explicitly document and reduce this gap:

- Validate expected env vars at startup and log consumed names, never values.
- Pin release images and document deployment config used for the published MRTD.
- Move credential bootstrap toward owner-direct attested injection after v1.

Owner-direct injection means the credential owner's device verifies the enclave attestation, then encrypts the credential to an ephemeral public key bound into the enclave quote. That is the path from K3 to K4.

---

## Target Build Pipeline

```text
source commit
    -> pinned Rust toolchain + Cargo.lock
    -> locked release build
    -> pinned Docker base image
    -> GHCR image digest
    -> Phala CVM launch
    -> published MRTD
    -> gate verify
```

The current repo has the source, Cargo lockfile, and StageX OCI build scaffold
pieces. A full release workflow, published golden digests, MRTD publication, and
`gate verify --reproduce` integration are still roadmap work.

---

## References

- [KRAB Framework](https://github.com/vkobel/coco-krab-framework)
- [Intel TDX Documentation](https://www.intel.com/content/www/us/en/developer/tools/trust-domain-extensions/documentation.html)
- [Phala dstack](https://github.com/Phala-Network/dstack)
- [Edgeless Systems - Reproducible Builds for Confidential Computing](https://www.edgeless.systems/blog/reproducible-builds-for-confidential-computing)
- [Trail of Bits - WhatsApp Private Processing Audit](https://blog.trailofbits.com/2026/04/07/what-we-learned-about-tee-security-from-auditing-whatsapps-private-inference/)
