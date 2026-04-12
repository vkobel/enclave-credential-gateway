## Context

`nono-proxy` is a well-tested localhost sidecar that already implements the phantom-token validation, path-prefix routing, credential injection, and outbound TLS primitives needed by this POC. It binds to `127.0.0.1` with an OS-assigned port and generates an ephemeral session token delivered to child processes via env var.

Phase 1a promotes this pattern into a long-running remote service runnable on any Docker-capable machine. No TEE-specific tooling is involved — the goal is to prove the proxy data plane works correctly before adding the Phala/TDX layer in Phase 1b.

## Goals / Non-Goals

**Goals:**
- Compose `nono-proxy`'s library modules into a standalone `coco-gateway` binary bound to `0.0.0.0:8080`
- Load phantom token and upstream credentials from env vars
- Route `/openai/`, `/anthropic/`, `/github/` path prefixes to their respective upstreams
- Validate phantom token from `Proxy-Authorization` header with constant-time comparison
- Strip phantom token, inject real upstream credential, stream response back
- Package as a Docker image runnable with `docker compose up` on any machine
- Validate end-to-end locally: OpenAI client through gateway to api.openai.com

**Non-Goals (deferred to Phase 1b):**
- `GET /attest` endpoint and TDX quote generation
- Phala Cloud deployment and tappd integration
- In-enclave TLS termination
- CI / GHCR image publishing
- Multi-tenancy, policy engine, rate limiting, audit logging

## Decisions

### D1: Use nono-proxy as a library dependency for component reuse

**Decision:** Add `nono-proxy` as a Cargo git dependency (pinned to a specific commit). Import and compose its public modules (`RouteStore`, `reverse` handler, `credential::CredentialStore`, `filter::ProxyFilter`, `token::constant_time_eq`) inside a custom Axum server. Do not call `nono_proxy::start()`.

**Rationale:** `nono-proxy` already exposes a clean public library API. However, `start()` auto-generates an ephemeral session token internally — incompatible with the pre-shared `COCO_PHANTOM_TOKEN` needed for remote agents. By composing the individual modules, we get the battle-tested proxy data plane without being locked into nono's local-only token lifecycle. The server wiring is ~80 lines of Axum boilerplate.

**Alternatives considered:**
- Copy and adapt modules from submodule: Creates maintenance drift against an evolving crate. Unnecessary given the library API.
- Use `nono_proxy::start()` directly: Forces ephemeral tokens that change on every restart — agents can't be preconfigured with a stable token.
- Fork nono-proxy: Viable long-term (Path C), unnecessary coupling for the POC.

### D2: Pre-shared phantom token loaded from env var, not generated ephemerally

**Decision:** `COCO_PHANTOM_TOKEN` is operator-defined and loaded at startup. Not regenerated on each boot.

**Rationale:** Remote agents can't receive a dynamically generated token via env var the way local child processes can. A pre-shared token is the minimal viable adaptation. Rotation is handled by updating the env var and redeploying.

**Alternatives considered:**
- Per-request HMAC token: Requires shared key distribution and clock sync — unnecessary complexity for POC.
- mTLS client certificates: Correct long-term direction; deferred to Phase 2.

### D3: Single tokio runtime, single Axum server, routes multiplexed

**Decision:** Run a single `tokio` async runtime with one Axum server on port 8080. All routes fall through to the proxy handler.

**Rationale:** Keeps the binary simple. No inter-process communication needed.

### D4: Route config compiled-in for POC

**Decision:** Hardcode the three route entries (`/openai/`, `/anthropic/`, `/github/`) in the binary. Env-var or YAML config is a follow-up.

**Rationale:** Minimizes scope. The `RouteStore` pattern supports dynamic config; we wire it statically for now.

### D5: Path B (standalone POC) → Path C (nono fork with client attestation)

**Decision:** Ship the POC as Path B — a standalone `coco-gateway` binary. Document Path C as the target architecture: a nono fork where `coco-gateway` lives as a workspace crate alongside `nono-cli`, which gains a `--coco <url>` flag that verifies the CVM's TDX attestation before spawning the sandboxed child process with `NetworkMode::ProxyOnly` pointing at the remote CVM.

**Rationale:** Path B is the fastest route to a working POC. Path C restores kernel-level egress enforcement and bakes attestation verification into the client, closing the two main gaps in Path B.

## Risks / Trade-offs

- **Egress enforcement gap** → Agents voluntarily route through CoCo via `BASE_URL`. A compromised agent can bypass the gateway. Mitigation: document the gap prominently; recommend egress firewall rules. Path C closes this gap.
- **nono-proxy API stability** → The crate is `0.6.0-alpha`. Mitigation: pin to a specific git commit. Breakage would be compile-time errors, not silent behavioral changes.
- **`start()` bypass means reimplementing server wiring** → By not using `nono_proxy::start()`, we own TCP listener setup and request routing. Mitigation: ~80 lines of Axum boilerplate — substantially less than copying five modules.

## Open Questions

- Which `nono-proxy` commit is stable for the modules we consume (`RouteStore`, `reverse`, `credential`, `token`)? (Resolve: inspect submodule at `./nono` and pin that commit.)
- Is `axum` preferable over raw `hyper` for the request handler? (Leaning axum for ergonomics; no strong reason to use raw hyper at POC scale.)
