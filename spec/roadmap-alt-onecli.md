# Alternative Roadmap — Informed by OneCLI Comparison

> Status: proposal / discussion. This document does **not** supersede
> `spec/roadmap.md`. It records a file-level comparison against
> [`onecli/onecli`](https://github.com/onecli/onecli) (reviewed at `v1.37.0`)
> and proposes an alternative sequencing that absorbs the parts of OneCLI worth
> having while keeping the enclave/attestation work as our differentiator.

## TL;DR

- OneCLI solves the **same core problem** ("store credentials once, agents never
  see the keys") and is far more mature on product surface: MITM HTTPS
  interception, AES-256-GCM secret store, Bitwarden/1Password vault sourcing, a
  Next.js dashboard, AWS SigV4 signing, and dozens of app integrations.
- OneCLI has **no TEE / enclave / attestation / reproducible-build story** — a
  full-tree search found zero references. Its OSS build keeps the AES key in
  `SECRET_ENCRYPTION_KEY` (host-readable); its cloud build uses KMS
  (operator-trusted). **Neither protects credentials from the host/operator.**
  That gap is exactly our wedge.
- Recommendation: **do not hard-fork the monorepo.** Keep our slim single-binary
  gateway as the measured StageX artifact (the enclave) and lift the
  self-contained, DB-free OneCLI modules into our crate. Treat MITM and vault
  sourcing as opt-in, non-default features so the default measured artifact stays
  minimal and its bytes stay stable.

## How the two projects line up

| Capability | OneCLI (`v1.37.0`) | This repo (today) |
|---|---|---|
| Core model | Gateway injects real creds for placeholder keys | Same, via scoped phantom tokens (`gate_…`) |
| Request handling | MITM HTTPS interception (own CA, per-host leaf certs) | Explicit reverse proxy + phantom-token rewrite |
| Secret store | AES-256-GCM in PostgreSQL, decrypted at request time | Env-injected creds; registry stores Blake3 token hashes |
| Key custody | `SECRET_ENCRYPTION_KEY` env (OSS) / KMS (cloud) | Env today; **sealed-in-TEE planned** |
| Provider coverage | Large app catalog (`apps.rs`), OAuth refresh, SigV4, GitLab/JFrog/etc. | OpenAI, Anthropic, GitHub route profiles |
| External vaults | Bitwarden + 1Password providers | none |
| Control plane | Next.js dashboard + Postgres + API | CLI (`gate`) + admin API |
| **Confidentiality vs host/operator** | **none** | **target: TDX-sealed, attestable** |

## File-level liftability assessment

Reviewed under `apps/gateway/src` (~17k LOC Rust). Coupling is the deciding
factor: anything that drags in PostgreSQL or the cloud control plane must stay
**out** of the measured enclave artifact.

### Lift first — self-contained, no DB coupling

| Source module | LOC | What it gives us | Notes / changes needed |
|---|---|---|---|
| `crypto.rs` | ~120 | AES-256-GCM (via `ring`) encrypt/decrypt, Node-compatible `iv:tag:ct` format | Basis for the sealed at-rest format. **Key must come from enclave sealing/derivation, not an env var** — this is where we exceed OneCLI. |
| `inject.rs` | ~980 | `Injection` enum (`SetHeader`/`ReplaceHeader`/`RemoveHeader`/`SetParam`), `apply_injections`, path-pattern matching | Enriches our static-YAML model with per-path query/header rules. Our `CredentialSource` already matches OneCLI's field-for-field, so the data model aligns. |
| `secret_inject.rs` | ~410 | Per-provider injection logic: Anthropic OAuth vs api-key, OpenAI OAuth (`chatgpt-account-id`), generic header/param/value-format | Lets us express OAuth-token providers in profiles without a DB. |
| `gateway/finalizers/aws_sigv4.rs` | ~376 | AWS SigV4 request signing | Net-new capability (signed-request upstreams). Self-contained. |
| `gateway/transforms/github_commit_trailer.rs` | ~237 | GitHub commit-trailer rewrite | Niche; optional. |

### Lift selectively — useful but higher-coupling, gate behind features

| Source module(s) | LOC | What it gives us | Why it's not default |
|---|---|---|---|
| `ca.rs` + `gateway/mitm.rs` + `gateway/forward.rs` + `gateway/response.rs` + `gateway/websocket.rs` | ~3.4k | Transparent HTTPS interception — removes phantom-token rewrite friction; agents make normal calls | Biggest UX win **and** biggest change. Requires clients to trust our CA, and the **CA private key must be enclave-sealed** or it reopens the trust hole. Keep as opt-in mode; do not bloat the default measured artifact. |
| `vault/onepassword*.rs`, `vault/bitwarden*.rs` | ~1.7k | Source secrets from external password managers on demand | Philosophical tension: pulls secret material from outside the TEE. Viable as **enclave-internal vault clients**, but lower priority than sealing our own store. |

### Do not import — Postgres / cloud-control-plane coupled

`db.rs`, `connect.rs` (`PolicyEngine`), `policy.rs`, `cache.rs`,
`telemetry*.rs`, and the cloud stubs (`budget.rs`, `approval.rs`,
`granular_access.rs`, `partner.rs` — real impls live in OneCLI's closed cloud
repo) all assume a Postgres-backed control plane. Pulling them in would drag a
database into the enclave and balloon the measured surface.

**Exception — mine for data, not code:** `apps.rs` (~3k LOC) is a catalog of
upstream app/route definitions. Harvest it to expand our
`profiles/routes/*.yaml`; do not import the module.

## Proposed phase sequencing

The through-line is unchanged: **the enclave is the product.** Each phase that
borrows from OneCLI is framed so it strengthens, rather than dilutes, the
measured-artifact and key-custody story.

### Phase A — Finish the moat (keep `spec/roadmap.md` 1c → 3 intact)
TDX attestation endpoint, CI measurement publication, and `gate verify`. Nothing
here comes from OneCLI; it is the capability they structurally lack. Do this
first so the differentiator is real before we widen surface.

### Phase B — Richer injection & provider coverage (low risk)
Lift `inject.rs` + `secret_inject.rs` to add OAuth-token providers
(Anthropic/OpenAI) and generic header/param/query injection to our YAML
profiles. Mine `apps.rs` for additional route profiles (data only). No new
runtime dependency, no DB, measured artifact unchanged in shape.

### Phase C — Sealed credential store (the wedge)
Adopt `crypto.rs`'s AES-256-GCM format as our at-rest store, but provision the
key via **enclave sealing/derivation** so it never exists in host-readable env —
the concrete point where our security exceeds both OneCLI builds. Add the SigV4
finalizer for signed-request upstreams. This lands alongside roadmap Phase 3
(sealed store + `gate verify`).

### Phase D — Optional transparency & vault sourcing (post-v1, opt-in)
Behind a non-default feature flag, lift the MITM pipeline (`ca.rs` + `mitm` +
`forward`/`response`/`websocket`) with the **CA private key sealed in the
enclave**, plus optional Bitwarden/1Password clients run **inside** the TEE.
Default builds stay minimal so the measured bytes of the enclave artifact remain
stable and reproducible.

## Invariants any adoption must preserve

- The default server artifact (`Containerfile.stagex`) stays minimal and
  single-target; opt-in features (MITM, vaults) must not change its measured
  bytes.
- Key material — sealed-store key and any MITM CA private key — **never leaves
  the TEE** and is never read from a host-visible env var in the enclave build.
- Scope enforcement still happens **before** credential resolution; a 403 never
  touches upstream credentials.
- Constant-time comparison for all token/secret equality checks; no raw `==`.
