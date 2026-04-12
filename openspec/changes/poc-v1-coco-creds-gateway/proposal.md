## Why

`nono-proxy` proves the phantom-token credential injection pattern locally, but agents running in cloud VMs, CI runners, or remote laptops need the same protection without hosting their own proxy. The POC demonstrates that `nono-proxy`'s core data plane can run inside a Phala Cloud TDX Confidential VM, making the pattern remotely accessible while keeping real credentials sealed in enclave memory — and providing a raw TDX attestation quote so operators can verify the running binary.

## What Changes

- **New binary `coco-gateway`:** A standalone HTTPS reverse proxy built on `nono-proxy` as a Cargo git library dependency, bound to `0.0.0.0:8080`, loading the phantom token and upstream credentials from environment variables injected by Phala's encrypted secret mechanism. Composes nono-proxy's individual modules (`RouteStore`, `reverse`, `credential`, `token::constant_time_eq`) inside a custom Axum server — does not use `nono_proxy::start()` since it forces ephemeral token generation incompatible with the pre-shared remote token model.
- **Phantom token validation (remote):** `COCO_PHANTOM_TOKEN` is a pre-shared 64-char hex token loaded at startup; agents include it in `Proxy-Authorization: Bearer <token>` (or `Basic` format) on every request. Validated with constant-time comparison via `nono_proxy::token::constant_time_eq`.
- **Route dispatching:** Path-prefix routing (`/openai/` → `api.openai.com`, `/anthropic/` → `api.anthropic.com`, `/github/` → `api.github.com`) via `nono-proxy`'s `RouteStore` and `RouteConfig`.
- **Credential injection:** Strips the phantom token header, injects `Authorization: Bearer <UPSTREAM_KEY>` from env vars before forwarding. Reuses `nono_proxy::reverse` and `nono_proxy::credential` modules directly.
- **`GET /attest` endpoint:** Returns the raw TDX DCAP QuoteV4 as JSON `{ "quote": "<hex>", "platform": "tdx" }` using `lunal-dev/attestation-rs`. Asserts debug bit is unset.
- **Docker Compose deployment on Phala Cloud:** Single-container deployment with secrets set via Phala CLI before launch.

## Capabilities

### New Capabilities

- `phantom-token-gateway`: Remote phantom-token authentication, route dispatching, and upstream credential injection running inside a TDX CVM. Accepts agent requests with `Proxy-Authorization: Bearer <phantom-token>`, resolves path prefix to upstream, strips phantom token, injects real credential from env, and streams the response back.
- `attestation-endpoint`: `GET /attest` endpoint that generates and serves a raw TDX DCAP QuoteV4 using `attestation-rs`. Includes debug-mode assertion and returns JSON with quote hex and platform identifier.
- `phala-deployment`: Docker Compose configuration and Phala Cloud secret injection workflow. Covers `docker-compose.yml`, secret provisioning via `phala cvms secrets set`, and startup validation.

### Modified Capabilities

*(none — no existing specs)*

## Impact

- **New crate:** `coco-gateway` (Rust binary) — composes `nono-proxy` library modules
- **Dependency:** `nono-proxy` as Cargo git dependency (pinned commit); `lunal-dev/attestation-rs` added to `Cargo.toml`
- **Submodule `nono`:** Retained for reference and potential Path C workspace integration, but not used as a path dependency in POC
- **Deployment:** Phala Cloud TDX CVM; requires `phala` CLI for secret provisioning
- **Egress enforcement gap:** In the remote model, agents route through CoCo voluntarily via `BASE_URL` — no kernel-level enforcement. Documented as known POC limitation with mitigation guidance (egress firewall, Path C upgrade)
- **Path B→C evolution:** POC ships as standalone binary (Path B). Target architecture is a nono fork with client-side attestation verification and kernel-level sandbox enforcement (Path C)
- **No breaking changes** to existing `nono` submodule or its crates
