## Context

`nono-proxy` is a well-tested localhost sidecar that already implements the phantom-token validation, path-prefix routing, credential injection, and outbound TLS primitives needed by this POC. It binds to `127.0.0.1` with an OS-assigned port and generates an ephemeral session token delivered to child processes via env var.

The CoCo POC promotes this pattern into a long-running remote service inside a Phala Cloud TDX CVM. The key constraints are:
- Phala Cloud runs Docker Compose workloads inside Intel TDX CVMs.
- TLS terminates at Phala's edge infrastructure (outside the enclave) — full attested TLS is a next-step, not a POC requirement.
- Phala injects operator secrets as env vars via X25519+AES-256-GCM encryption; plaintext is only accessible inside the TEE at runtime.
- `attestation-rs` (lunal-dev) supports Phala/dstack TDX quote generation via its `attest` feature.

The submodule at `./nono` is retained for reference. Investigation of `nono-proxy`'s `lib.rs` reveals a clean public library API: `pub use config::ProxyConfig`, `pub use server::{start, ProxyHandle}`, and all submodules (`audit`, `config`, `connect`, `credential`, `filter`, `reverse`, `route`, `server`, `token`) are `pub`. The gateway is a new Rust binary (`coco-gateway`) that depends on `nono-proxy` as a Cargo git library and composes its individual modules inside a custom Axum server.

A key constraint: `nono_proxy::start()` auto-generates an ephemeral session token (returned in `ProxyHandle.token`) and does not accept a pre-set token. The remote model requires a pre-shared `COCO_PHANTOM_TOKEN` from Phala secrets, so the gateway composes nono-proxy's component modules directly rather than using `start()`.

## Goals / Non-Goals

**Goals:**
- Compose `nono-proxy`'s library modules into a standalone `coco-gateway` binary bound to `0.0.0.0:8080`
- Load phantom token and upstream credentials from env vars (Phala secrets injection)
- Route `/openai/`, `/anthropic/`, `/github/` path prefixes to their respective upstreams
- Validate phantom token from `Proxy-Authorization` header with constant-time comparison
- Strip phantom token, inject real upstream credential, stream response back
- Expose `GET /attest` returning raw TDX DCAP QuoteV4 as JSON
- Package as a Docker image deployable via `docker-compose.yml` on Phala Cloud
- Validate end-to-end: OpenAI client through gateway to api.openai.com

**Non-Goals:**
- In-enclave TLS termination (Phala edge handles TLS for the POC)
- Multi-tenancy or per-agent identity
- Policy engine (method/path rules beyond routing)
- Portable encrypted vaults — Phala secrets are sufficient for POC
- Reproducible builds / MRTD pinning
- Rate limiting, token budgets, or audit logging

## Decisions

### D1: Use nono-proxy as a library dependency for component reuse

**Decision:** Add `nono-proxy` as a Cargo git dependency (pinned to a specific commit). Import and compose its public modules (`RouteStore`, `reverse` handler, `credential::CredentialStore`, `filter::ProxyFilter`, `token::constant_time_eq`) inside a custom Axum server. Do not call `nono_proxy::start()`.

**Rationale:** `nono-proxy` already exposes a clean public library API (`ProxyConfig`, `start()`, `ProxyHandle`, all submodules `pub`). However, `start()` auto-generates an ephemeral session token internally — incompatible with the pre-shared `COCO_PHANTOM_TOKEN` needed for remote agents. By composing the individual modules, we get the battle-tested proxy data plane (reverse proxy, credential injection, route dispatch, host filtering, constant-time token comparison) without being locked into nono's local-only token lifecycle. The server wiring is ~80 lines of Axum boilerplate, substantially less than copying and maintaining five modules.

**Alternatives considered:**
- Copy and adapt modules from submodule: Works but creates maintenance drift against an evolving `0.6.0-alpha` crate. The library API makes this unnecessary.
- Use `nono_proxy::start()` directly: Forces ephemeral tokens that change on every restart — agents can't be preconfigured with a stable token.
- Fork nono-proxy to add a `token` parameter to `start()`: Viable long-term (part of Path C), unnecessary coupling for the POC.

### D2: Pre-shared phantom token loaded from env var, not generated ephemerally

**Decision:** `COCO_PHANTOM_TOKEN` is operator-defined, stored in Phala secrets, and loaded at gateway startup. It is not regenerated on each boot.

**Rationale:** The local `nono` model generates an ephemeral token and delivers it to child processes via env var — impossible for remote agents. A pre-shared token is the minimal viable adaptation: the operator provisions it once, agents are configured with it out-of-band. Rotation is handled by updating the Phala secret and redeploying.

**Alternatives considered:**
- Per-request HMAC token: Requires shared key distribution and clock sync — unnecessary complexity for POC.
- mTLS client certificates: Correct long-term direction; deferred to Phase 2.

### D3: Use Phala tappd sidecar for TDX quote generation

**Decision:** For the POC, obtain the TDX quote by calling Phala's `tappd` sidecar at `http://localhost:8090/prpc/Tappd.TdxQuote` via `reqwest`. No external attestation library needed.

**Rationale:** `tappd` is already running in every Phala CVM — zero additional dependencies, ~15 lines of Rust. `attestation-rs` is the right abstraction for multi-platform support (Azure TDX, GCP, AWS Nitro) but adds build complexity the POC doesn't need yet.

**Alternatives considered:**
- `attestation-rs` (lunal-dev): Clean multi-platform abstraction, supported via a contributor PR for Phala. Deferred to post-POC when multi-platform matters.
- Direct ioctl to `/dev/tdx_guest`: Works but requires unsafe code and manual quote parsing.

### D4: Single tokio runtime, single HTTP server, routes multiplexed

**Decision:** Run a single `tokio` async runtime with one Axum (or Hyper) server on port 8080. The `/attest` route is handled by a dedicated handler; all other routes fall through to the proxy handler.

**Rationale:** Keeps the binary simple. No inter-process communication needed. Port 8080 is the single ingress point behind Phala's edge.

**Alternatives considered:**
- Separate processes for attestation and proxy: Unnecessary complexity, no security benefit in the POC.

### D5: Route config via compiled-in defaults for POC

**Decision:** For the POC, hardcode the three route entries (`/openai/` → `api.openai.com`, `/anthropic/` → `api.anthropic.com`, `/github/` → `api.github.com`) in the binary. Env-var or YAML config is a follow-up.

**Rationale:** Minimizes scope. The `RouteStore` pattern from `nono-proxy` supports dynamic config; we wire it statically for now.

### D6: Path B (standalone POC) → Path C (nono fork with client attestation)

**Decision:** Ship the POC as Path B — a standalone `coco-gateway` binary that uses nono-proxy as a library dependency. Document Path C as the target architecture: a nono fork where `coco-gateway` lives as a workspace crate alongside `nono-cli`, which gains a `--coco <url>` flag that verifies the CVM's TDX attestation before spawning the sandboxed child process with `NetworkMode::ProxyOnly` pointing at the remote CVM.

**Rationale:** Path B is the fastest route to a working POC. Path C requires forking nono, modifying its CLI, and maintaining a diverging codebase — a larger commitment best undertaken after the core gateway is proven. Path C restores kernel-level egress enforcement (Landlock `ProxyOnly` pointing at the remote CVM) and bakes attestation verification into the client, closing the two main gaps in Path B.

**Path C target architecture:**
```
nono fork workspace:
  crates/
    nono-cli/        <- modified: add --coco flag, attestation verification on startup
    nono-proxy/      <- unchanged
    nono/            <- unchanged
    coco-gateway/    <- new: CVM binary composing nono-proxy modules + /attest endpoint
```

**Alternatives considered:**
- Ship Path C directly: Higher quality but blocks POC delivery. The gateway data plane is identical in both paths.
- Path A (fork CLI only, no attestation in client): Weaker than C — the user trusts the CVM URL blindly. Not worth the fork maintenance cost without the attestation payoff.

### D7: Semantic AI Firewall deferred to Phase 5+

**Decision:** Explicitly push the Semantic AI Firewall (LLM-evaluated intent blocking inside the enclave) to Phase 5+, after audit and attestation infrastructure is proven.

**Rationale:** Intent-based LLM evaluation inside the TEE introduces a latency bottleneck and a second model trust problem. The deterministic policy engine (method/path rules, endpoint filtering) already provided by nono-proxy's `EndpointRule` and `CompiledEndpointRules` covers the most valuable policy use cases. Adding semantic evaluation prematurely would pull in GPU/inference dependencies and complicate the enclave's measurement surface.

## Risks / Trade-offs

- **Egress enforcement gap (POC)** → Agents voluntarily route through CoCo via `BASE_URL`. A compromised agent can bypass the gateway and talk directly to upstream APIs. Mitigation: document the gap prominently; recommend pairing with cloud egress firewall (security groups, VPC rules) or eBPF-based egress filtering on agent hosts. Path C closes this gap with kernel-level Landlock enforcement.
- **TLS terminates outside the enclave** → For the POC this is acceptable; traffic between Phala's edge and the container is on the internal CVM network. Operators should treat this as a known POC limitation documented in SPEC.md. Mitigation: document prominently; Phase 2 adds in-enclave TLS.
- **Pre-shared phantom token is long-lived** → If an agent host is compromised, the attacker holds the token until the operator rotates it. Mitigation: token is only useful against this specific gateway; real upstream keys never leave the enclave. Rotation requires one Phala secret update and redeploy.
- **`attestation-rs` git dependency** → No pinned release; upstream may change. Mitigation: pin to a specific commit hash in `Cargo.toml`.
- **nono-proxy API stability** → The crate is `0.6.0-alpha` with an evolving API surface. Mitigation: pin to a specific git commit in `Cargo.toml`. The modules we consume (`RouteStore`, `reverse`, `credential`, `token`) are stable in design. Breakage would be compile-time errors, not silent behavioral changes.
- **Debug mode TDX quote accepted silently** → If deployed on a debug TDX instance, the attestation is meaningless. Mitigation: gateway asserts `td_attributes[0] & 0x01 == 0` at startup and at `/attest`; logs a hard error if debug mode detected.
- **`start()` bypass means reimplementing server wiring** → By not using `nono_proxy::start()`, we own TCP listener setup and request routing. Mitigation: ~80 lines of Axum boilerplate, substantially less than copying and maintaining five modules. The individual handler modules do the heavy lifting.

## Migration Plan

1. Create `crates/coco-gateway/` workspace member in a new Cargo workspace (not inside the nono submodule).
2. Add `nono-proxy` as a Cargo git dependency pinned to a specific commit. Verify key types (`ProxyConfig`, `RouteConfig`, `RouteStore`, `constant_time_eq`) are importable.
3. Write `main.rs`: env var loading → route config construction → Axum server with token validation middleware, proxy handler (composing nono-proxy modules), and `/attest` endpoint.
4. Add `attestation-rs` git dependency pinned to a specific commit.
5. Write `Dockerfile` (multi-stage: cargo chef + build + distroless runtime).
6. Write `docker-compose.yml` referencing `ghcr.io/<org>/coco-gateway:latest`.
7. Push image to GHCR, deploy to Phala Cloud, set secrets via `phala cvms secrets set`.
8. Validate: `GET /attest` returns valid TDX quote; OpenAI client request succeeds end-to-end.

**Rollback:** Redeploy previous image tag; Phala secrets are unchanged.

## Open Questions

- Does Phala's current CVM networking model allow the container to make outbound TLS connections directly, or does traffic route through a Phala egress proxy? (Expected: direct outbound is supported; verify during deployment.)
- Which `attestation-rs` commit is stable for Phala/dstack TDX? (Resolve: check lunal-dev/attestation-rs issues and Phala dstack compatibility matrix.)
- Is `axum` preferable over raw `hyper` for the request handler, given the small surface area? (Leaning axum for ergonomics; no strong reason to use raw hyper at POC scale.)
