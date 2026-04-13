## Why

`nono-proxy` proves the phantom-token credential injection pattern locally, but agents running in cloud VMs, CI runners, or remote laptops need the same protection without hosting their own proxy. Before deploying to a Phala Cloud TDX CVM, we first prove the core proxy data plane works correctly on plain infrastructure — easier feedback loop, no TEE dependencies, clear done signal.

This is Phase 1a of the POC. Phase 1b (`poc-v1b-cvm-attestation`) promotes this binary to Phala Cloud and adds the `/attest` endpoint.

## What Changes

- **New binary `coco-gateway`:** A standalone reverse proxy built on `nono-proxy` as a Cargo git library dependency, bound to `0.0.0.0:8080`, loading the phantom token and upstream credentials from env vars. Composes nono-proxy's individual modules (`RouteStore`, `reverse`, `credential`, `token::constant_time_eq`) inside a custom Axum server — does not use `nono_proxy::start()` since it forces ephemeral token generation incompatible with the pre-shared remote token model.
- **Phantom token validation:** `COCO_PHANTOM_TOKEN` is a pre-shared 64-char hex token loaded at startup; agents include it in `Proxy-Authorization: Bearer <token>` (or `Basic` format) on every request. Validated with constant-time comparison via `nono_proxy::token::constant_time_eq`.
- **Route dispatching:** Path-prefix routing (`/openai/` → `api.openai.com`, `/anthropic/` → `api.anthropic.com`, `/github/` → `api.github.com`) via `nono-proxy`'s `RouteStore` and `RouteConfig`.
- **Credential injection:** Strips the phantom token header, injects `Authorization: Bearer <UPSTREAM_KEY>` from env vars before forwarding.
- **Docker Compose packaging:** Single-container deployment runnable with `docker compose up` on any machine (local laptop, plain VM). No Phala or TEE dependencies.

## Capabilities

### New Capabilities

- `phantom-token-gateway`: Remote phantom-token authentication, route dispatching, and upstream credential injection. Accepts agent requests with `Proxy-Authorization: Bearer <phantom-token>`, resolves path prefix to upstream, strips phantom token, injects real credential from env, and streams the response back.
- `docker-packaging`: Multi-stage Dockerfile and `docker-compose.yml` for running `coco-gateway` on any Docker-capable machine.

### Modified Capabilities

*(none — no existing specs)*

## Impact

- **New crate:** `coco-gateway` (Rust binary) in a new Cargo workspace at the project root
- **Dependency:** `nono-proxy` as Cargo git dependency (pinned commit)
- **Submodule `nono`:** Retained for reference; not used as a path dependency
- **No breaking changes** to existing `nono` submodule or its crates
- **Egress enforcement gap:** Agents route through the gateway voluntarily via `BASE_URL` — no kernel-level enforcement. Documented as known POC limitation. Addressed in Path C.
