# Enclave Credential Gateway - TEE Security Target

> Platform: the [Caution](https://caution.co) platform. Caution runs the gateway in a
> confidential enclave and provides attestation and reproducible measurement
> verification. Its current TEE backing is AWS Nitro Enclaves; Caution plans to support
> additional TEEs (e.g. TDX, SEV), so this document targets **Caution** and treats
> substrate-specific details (NSM, PCRs, AWS root CA) as the current backing.
> Framework: [KRAB](https://github.com/vkobel/coco-krab-framework) for scoring attestation, reproducibility, session binding, and key release.

This document describes the target TEE security profile for v1. It is not a claim that the current repository already provides these guarantees.

## Current Implementation Status

Working today:

- Path-based gateway/proxy with OpenAI, Anthropic, and GitHub route profiles.
- Phantom token registry stored as Blake3 token hashes in `tokens.json`.
- Per-token scope enforcement before credential resolution.
- Admin API protected by `GATE_ADMIN_TOKEN`, compared constant-time in memory.
- Reproducible server and CLI OCI artifacts built through the pinned [StageX](https://codeberg.org/stagex/stagex) Rust pallet, with documented current hashes and verified byte-for-byte no-cache rebuilds.
- Caution-ready `Procfile` (server image, `http_port` behind Caddy, `app_sources` for `caution verify`, `locksmith` for secrets).
- Docker Compose + Caddy local development scaffold.

Provided by the Caution platform, not by this repository:

- Attestation: Caution's `bootproofd` serves an AWS Nitro NSM attestation document at `/attestation` (POST a nonce). The gateway implements no attestation endpoint of its own.
- Measurement: PCR0 (enclave image), PCR1 (kernel/boot), PCR2 (application).
- Client verification: `caution verify --attestation-url <url>` reproduces the enclave from pinned source and matches PCRs.
- Secret provisioning: Caution Locksmith (`locksmith: true`).
- Optional end-to-end encryption to the enclave: Caution steve (`e2e: true`).

Not implemented yet:

- `gate verify` parity with `caution verify` (the gateway's own verifier).
- CI release publishing, published golden artifacts, and PCR publication.
- Attested credential injection over steve (owner-direct provisioning).
- Audit log, credential redaction, and token expiry.

---

## Target KRAB Vector

Scored for Caution's current AWS Nitro backing. As Caution adds substrates, the `A` anchor and lower-layer `R` scores would be re-derived per substrate; `B` and `K` are properties of the gateway and provisioning flow and carry over.

```text
A1[AWS Nitro] | R[f0/o4/l4/a4] | B2 | K1
```

| Dimension | Target score | Meaning |
|---|---|---|
| **A - Attestation** | `A1[AWS Nitro]` | NSM attestation rooted in the AWS Nitro Attestation PKI. AWS provider PKI sits in the attestation trust boundary — a conscious delegation to AWS. The application is packed into the measured EIF (PCR2), so the measurement chain reaches the workload. |
| **R - Reproducibility** | `R[f0/o4/l4/a4]` | Nitro firmware/hypervisor opaque (`f0`); EnclaveOS + `linux-nitro` kernel reproduced from pinned StageX inputs and re-measured by `caution verify` (`o4`); gateway is a fully static musl binary with no dynamic libraries, built from the digest-pinned StageX Rust pallet (`l4`); gateway binary reproducible and measured as PCR2 (`a4`). |
| **B - Session Binding** | `B2` | `caution verify` posts a fresh 32-byte nonce; NSM binds it into the attestation document; `bootproof-sdk` verifies the nonce and enforces certificate validity/freshness. |
| **K - Key Release** | `K1` | Today secrets are provisioned by Caution Locksmith: shard-holders verify the enclave's attestation before releasing shards, but release is gated by an operator quorum identity rather than an automated independent verifier enforcing exact measurements. |

Post-v1 target: `A1[AWS Nitro] | R[f0/o4/l4/a4] | B2 | K4`, reached by replacing Locksmith (for the solo operator) with **owner-direct attested injection over steve**: the owner runs `caution verify` (exact PCR match), then pushes credentials over steve's attestation-bound channel into enclave RAM. That binds release to both exact measurements and a live session (`K4`).

Verified facts:

- AWS Nitro NSM attestation document is COSE_Sign1, signed by a certificate chain rooted in the AWS Nitro Attestation PKI root CA.
- `caution verify` reproduces the EIF from the manifest's pinned `app_source`, `enclave_source`, and `framework_source` commits and compares PCR0/PCR1/PCR2.
- Debug mode zeros PCRs; `caution verify` rejects all-zero PCR evidence.

Assumptions / unknowns:

- `o4`/`l4`/`a4` reproducibility depends on Caution's build tooling and the pinned StageX pallets, which are themselves source-available and inspectable but constitute a build-toolchain dependency.
- Locksmith's per-release freshness and exact-measurement rigor are operator-driven; the `K1` score reflects that human gate.
- steve `e2e` is not enabled in the current `Procfile`; `B2` reflects the `caution verify` attestation path, not a per-credential-request bound session.

---

## Threat Model

The target TEE boundary is designed to defend against:

- Infrastructure operators reading real credentials from gateway memory or storage.
- Agent processes reading real vendor keys from their own host environment.
- Prompt injection that asks an agent to reveal a key it never had.
- Supply-chain substitution of the gateway binary, since the EIF is reproducible and PCR-attested.

It does not defend against:

- A malicious agent making authorized calls within its phantom token scope.
- AWS operating the Nitro hypervisor and NSM, which sit inside the accepted `A1[AWS Nitro]` trust boundary.
- Nitro side channels.
- Runtime input attacks that occur after measurement, unless those inputs are separately validated or attested.

Current implementation note: today the gateway reduces credential exposure to clients and agents. Protection from the host or infrastructure operator depends on a Caution/Nitro deployment with Locksmith-provisioned secrets; secrets passed via plain environment are not protected.

---

## Security Requirements

### R1 - Attestation Endpoint (platform-provided)

Attestation is provided by Caution's `bootproofd`, not by the gateway:

- The endpoint at `/attestation` returns an AWS Nitro NSM attestation document (COSE_Sign1).
- The caller supplies a fresh nonce, which NSM binds into the document.
- PCR0/PCR1/PCR2 measure the enclave image, kernel/boot, and application.
- Debug mode zeros the PCRs, which `caution verify` and any conforming verifier must reject.

The gateway must not implement its own attestation endpoint; doing so would sit outside the measured boot chain.

### R2 - Reproducible Release Pipeline

The release process makes the binary-to-source link independently checkable:

- Build the gateway through the digest-pinned StageX Rust pallet with `cargo build --frozen --release` after an offline `cargo fetch`.
- Caution packs the reproducible image into the EIF; PCR0/PCR1/PCR2 become the measured identity.
- `caution verify --attestation-url <url>` reproduces the EIF from the manifest's pinned source commits and confirms the PCRs match the live enclave.

The byte-for-byte StageX OCI tarball reproducibility is the deterministic foundation that makes the EIF PCRs reproducible. CI-published golden artifacts and PCR publication remain roadmap work.

### R3 - Credential Provisioning

Real credentials must never be baked into the image, the Procfile, or any committed file in plaintext.

- Current path: Caution Locksmith decrypts secrets only inside the enclave and exports them into the run command's environment. See [docs/LOCKSMITH.md](../docs/LOCKSMITH.md).
- Target path (solo operator): owner-direct attested injection over steve, holding credentials only in enclave RAM. See R6.

Credential values must never appear in API responses, logs, audit entries, or client config.

### R4 - Admin Token

Current behavior: `GATE_ADMIN_TOKEN` is provisioned (via Locksmith or environment), held in memory, and compared constant-time for `/admin/*` requests.

Target hardening: store only a hash or sealed representation of the admin secret after startup, and include admin-token handling in the attested provisioning story.

### R5 - Client Verification

Verification has two levels.

Platform verifier (available today):

```sh
caution verify --attestation-url https://<gateway>/attestation
```

`caution verify` generates a fresh nonce, fetches the NSM document plus the enclave manifest, extracts PCR0/1/2, reproduces the EIF from the manifest's pinned source commits, and verifies via `bootproof-sdk`: AWS Nitro root CA chain, certificate validity period, COSE signature, nonce match, and PCR equality. It rejects debug-mode (all-zero PCR) evidence.

`gate verify` (roadmap): the `gate` CLI reaching parity with `caution verify`, so a user can verify the gateway with the same tool they already use for tokens and activation — reusing `bootproof-sdk` for the attestation check and either depending on Caution's `enclave-builder` or shelling out to `caution verify` for the reproduce step.

### R6 - Measurement Gap: Post-Boot Inputs

The PCR measurement does not automatically cover runtime environment variables, the run command's injected secrets, or mounted data. v1 should explicitly document and reduce this gap:

- Validate expected env vars at startup and log consumed names, never values.
- Pin the deployed `Procfile`/manifest used for the published PCRs.
- Move credential bootstrap toward owner-direct attested injection.

Owner-direct injection means the credential owner's device verifies the enclave attestation (PCR match via `caution verify`), then sends credentials over steve's attestation-bound channel — steve publishes its verifying key in the attestation document's user data and binds an X25519 session key to it — so credentials reach enclave RAM without plaintext on the operator's disk or in the repo. That is the path from `K1` to `K4`.

---

## Target Build Pipeline

```text
source commit
    -> pinned StageX Rust pallet + Cargo.lock
    -> locked offline release build (reproducible OCI image)
    -> Caution EIF build (caution apps build)
    -> published PCR0/PCR1/PCR2
    -> AWS Nitro Enclave launch
    -> NSM attestation at /attestation
    -> caution verify  (and, later, gate verify)
```

The current repo has the source, Cargo lockfile, reproducible StageX OCI artifacts, and a Caution-ready `Procfile`. A full CI release workflow, published golden PCRs, and `gate verify` parity are still roadmap work.

---

## References

- [KRAB Framework](https://github.com/vkobel/coco-krab-framework)
- [Caution platform](https://docs.caution.co/)
- [Caution attestation concepts](https://docs.caution.co/concepts/attestation/)
- [AWS Nitro Enclaves](https://docs.aws.amazon.com/enclaves/latest/user/nitro-enclave.html)
- [distrust EnclaveOS](https://git.distrust.co/public/enclaveos)
- [bootproof attestation SDK](https://git.distrust.co/public/bootproof)
- [Trail of Bits - WhatsApp Private Processing Audit](https://blog.trailofbits.com/2026/04/07/what-we-learned-about-tee-security-from-auditing-whatsapps-private-inference/)
