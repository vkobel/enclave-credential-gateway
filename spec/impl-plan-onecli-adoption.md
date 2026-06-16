# Implementation Plan — Adopt OneCLI, Shrink Custom Code, Pave Caution

> First implementation plan. Branch `claude/onecli-feature-comparison-0d13mj`,
> rebased onto `feat/service-token-registry` (which already lands the RAM-only
> `credstore`, `.caution/` quorum bundle, Locksmith provisioning, and
> `gate creds`). Read alongside `spec/roadmap-alt-onecli.md` (the file-level
> liftability matrix) and `spec/roadmap.md`.

## Goal

Make OneCLI's gateway features usable from this repository while (a) deleting as
much of our bespoke gateway mechanics as possible, (b) keeping the StageX
reproducible single-target artifact, and (c) preserving the Caution/enclave
path as the differentiator. End state: we own the **enclave shell + resolution
policy**; OneCLI (or its building-block crates) owns the **request-injection
mechanics**.

## The hard constraint that shapes everything

OneCLI is **not distributed as a consumable library.** `apps/gateway` is a
`[[bin]]`-only crate (`name = "onecli-gateway"`, no `[lib]`, not published to
crates.io), it requires a **PostgreSQL control plane at runtime** (`db.rs`,
`connect.rs::PolicyEngine`, `policy.rs`, `apps.rs`), and it uses a cloud/OSS
`#[cfg]/#[path]` module-swap design. You cannot `cargo add onecli-gateway`, and
importing it wholesale would drag Postgres + a Next.js control plane into the
enclave — the opposite of "keep StageX minimal."

So "work with their crates" resolves to two concrete, maintenance-light levers:

1. **Reusable published crates OneCLI already builds on** (zero fork): `aws-sigv4`
   + `aws-credential-types` (SigV4 signing), `rcgen` + `rustls` (MITM CA), and the
   **`ap-*` Agent Access crates** (`ap-client`, `ap-proxy-client`,
   `ap-proxy-protocol`, `ap-noise` @ 0.9.0 — Bitwarden vault). These are real
   crates.io deps; adopt them directly.
2. **A DB-free `gateway-core` lib** carved out of OneCLI's own modules (`inject`,
   `secret_inject`, `crypto`, `gateway::finalizers`, and later `ca` +
   `gateway::mitm`). These have no Postgres coupling. Consume via a **pinned git
   dependency**.

### Decision needed: how we consume `gateway-core`

- **Option A (recommended): upstream a `[lib]` split.** Open a PR on OneCLI that
  adds `src/lib.rs` re-exporting the DB-free modules behind a default-off
  `embeddable` feature; keep `main.rs` as-is. We then depend via
  `onecli-gateway = { git = "…", rev = "…", default-features = false, features = ["embeddable"] }`.
  Zero vendored logic, upstream carries maintenance.
- **Option B (fallback): thin fork branch.** If upstream is slow, maintain a fork
  whose only delta is `lib.rs` (pure `pub mod` re-exports, no logic), so periodic
  rebases onto upstream are trivial and conflict-free.
- **Option C (avoid): vendor/copy modules.** Maximizes our maintenance burden;
  contradicts the goal. Use only as a last resort for a single small file.

Until A/B lands, Step 1 below (published-crate adoption) is unblocked and
delivers value immediately.

## What we did particularly well — KEEP (independent of the enclave work)

These are genuinely better than OneCLI's equivalents for our threat model and
should survive the migration:

1. **RAM-only zeroizing credential store (`credstore.rs`).** No secrets at rest,
   no database. OneCLI persists AES-256-GCM ciphertext in Postgres and holds the
   key in a host-readable `SECRET_ENCRYPTION_KEY` env var (OSS) or KMS (cloud) —
   both readable by the operator. Our store is strictly stronger for an enclave.
   **Keep as the secret store; do not adopt OneCLI's `db.rs` store.**
2. **Embedded build-time route profiles (`profiles/routes/*.yaml` via
   `include_str!`).** No runtime profile loading → deterministic, reproducible,
   DB-free. OneCLI resolves everything from Postgres at request time. Our model is
   the right fit for a measured artifact. **Keep YAML as the source of truth;
   extend its schema rather than replace it.**
3. **Scope-enforcement-before-credential-resolution + constant-time token
   checks.** Clean security invariant OneCLI does not express as crisply. **Keep.**
4. **StageX single-target reproducible OCI build + golden hashes.** The
   measurement anchor. **Keep; treat as a constraint on every dependency we add.**
5. **Caution Locksmith / quorum bundle / steve attested `gate creds push`.** The
   enclave provisioning path — the thing OneCLI structurally lacks. **Keep; this
   is the moat.**

## What gets FULLY REPLACED (delete our custom code)

- **Bespoke header/URL credential injection in `proxy.rs`** (`insert_extra_headers`,
  the `InjectMode::Header`/`UrlPath` formatting, manual header surgery) → replaced
  by OneCLI's `Injection` enum + `apply_injections` (`inject.rs`) and the
  per-provider `build_injections` (`secret_inject.rs`). `proxy.rs` shrinks to a
  thin shell: resolve route → resolve credential from `credstore` → hand to the
  adopted injection engine.
- **Our hand-rolled per-provider quirks** (Anthropic OAuth vs api-key,
  OpenAI/Codex OAuth, GitHub Basic, extra-header dedup) → replaced by
  `secret_inject::build_injections`, which already encodes all of these.
- **AWS support we never had** → adopt `gateway::finalizers::aws_sigv4` (net-new,
  not a replacement).
- **(Optional, later) our explicit-proxy + phantom-rewrite request path** → could
  be replaced by OneCLI's MITM pipeline (`ca` + `gateway::mitm`/`forward`/
  `response`/`websocket`) to get transparent HTTPS. This is the `HTTPS_PROXY`/
  CONNECT post-v1 theme already in `roadmap.md`. **Opt-in feature only** — see
  constraints.

## What we DO NOT adopt

`db.rs`, `connect.rs::PolicyEngine`, `policy.rs`, `cache.rs`, `telemetry*.rs`,
the Next.js dashboard, and the cloud stubs (`budget`, `approval`,
`granular_access`, `partner` — real impls live in OneCLI's closed cloud repo).
All assume a Postgres/cloud control plane. **Exception:** mine `apps.rs` for
upstream route definitions as *data* to grow `profiles/routes/*.yaml` — never
import the code.

## First implementation — sequenced steps

Ordered for early value and to keep each step independently shippable and
StageX-clean.

**Step 0 — Done.** Rebase this branch onto `feat/service-token-registry`.

**Step 1 — Adopt published building-block crates (no fork; unblocked now).**
- Add `aws-sigv4` + `aws-credential-types`; port `aws_sigv4.rs` as a `finalizer`
  step in `proxy.rs` behind a route-profile flag. First net-new capability.
- Keep all additions out of the default enclave feature set if they enlarge the
  measured surface (see Step 5).

**Step 2 — Extend the YAML profile schema to emit injection rules.**
- Teach `profile.rs` to express `Injection` rules (`set_header`/`replace_header`/
  `remove_header`/`set_param`) and per-path patterns, mirroring OneCLI's
  `InjectionRule`. Source of truth stays the embedded YAML (our win #2).
- Regenerate the OpenAI/Anthropic/GitHub profiles in the new schema; keep e2e
  green. No new runtime dep, no DB.

**Step 3 — Carve out / consume `gateway-core` (pending the Option A/B decision).**
- Replace the bespoke injection code in `proxy.rs` with
  `gateway_core::inject::apply_injections` + `secret_inject::build_injections`.
  `credstore` remains the value source; the engine only transforms the request.
- Delete the now-dead injection helpers from `proxy.rs`. Target: `proxy.rs` is a
  resolver shell over imported mechanics.

**Step 4 — (Optional) vault sourcing via `ap-*` crates, enclave-internal.**
- Add Bitwarden sourcing through `ap-client`/`ap-noise` as a credstore *backfill*
  running **inside** the enclave, behind a default-off feature. Honors the
  "secrets never leave the TEE" invariant.

**Step 5 — Keep StageX minimal: feature-gate everything heavy.**
- Default server build (the measured artifact) compiles only: resolver +
  `credstore` + `gateway-core` injection. `cargo features`: `sigv4`, `mitm`,
  `vault` are **off by default** so the enclave's measured bytes stay stable and
  reproducible.
- Re-run the StageX byte-for-byte reproduction (`scripts/build-stagex-oci.sh`)
  after each step; record any golden-hash change deliberately.

**Step 6 — (Post-v1) transparent MITM mode.**
- Behind a non-default `mitm` feature, adopt `ca` + `gateway::mitm` with the **CA
  private key sealed via Locksmith/steve**, never an env var. Realizes the
  `HTTPS_PROXY`/CONNECT roadmap theme without weakening the enclave model.

## Maintenance & ownership after this lands

| Layer | Owner | Maintenance cost |
|---|---|---|
| Enclave shell: `credstore`, resolver, scope/auth, StageX, Caution/Locksmith | us | ours to keep — it's the moat |
| Embedded profiles (YAML data, partly mined from `apps.rs`) | us | low; data, not logic |
| Injection mechanics, SigV4, MITM, vault | OneCLI / upstream crates | low if Option A; trivial-rebase if Option B |
| Postgres control plane, dashboard, cloud stubs | OneCLI | none — we never import it |

Net effect: our custom **gateway mechanics** code shrinks substantially
(`proxy.rs` injection logic deleted), our custom **security-critical** code
(`credstore`, scope, attested provisioning) stays ours by design, and ongoing
upkeep of injection/provider quirks moves upstream.

## Open decisions for the user

1. **Consumption strategy:** Option A (upstream `[lib]` PR) vs Option B (thin
   fork). Recommend A, fall back to B.
2. **Client contract:** keep our phantom tokens (`gate_…`, works with
   `gate activate`) as the auth model, or move to OneCLI's
   `Proxy-Authorization: Basic` to align with their MITM path? Recommend keeping
   phantom tokens for now; revisit at Step 6.
3. **MITM timing:** transparent interception is the biggest UX win but the biggest
   change to the measured artifact and CA-key custody. Recommend deferring to
   Step 6 (post-v1), keeping the default artifact explicit-proxy and minimal.
