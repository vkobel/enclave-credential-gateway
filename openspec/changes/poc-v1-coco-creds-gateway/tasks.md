## 1. Workspace and Library Dependency Setup

- [ ] 1.1 Create a new Cargo workspace at the project root (`Cargo.toml` with `[workspace]`) containing a single member `crates/coco-gateway`
- [ ] 1.2 Scaffold `crates/coco-gateway/` with `Cargo.toml` (binary crate, edition 2021) and `src/main.rs` stub
- [ ] 1.3 Add `nono-proxy` as a git dependency pinned to a specific commit: `nono-proxy = { git = "https://github.com/always-further/nono", rev = "<pin>", package = "nono-proxy" }`
- [ ] 1.4 Add remaining dependencies: `tokio` (full), `axum`, `hyper`, `hyper-rustls`, `tower-http`, `rustls`, `webpki-roots`, `reqwest` (for tappd call), `zeroize`, `hex`, `serde`, `serde_json`, `subtle`, `tracing`, `tracing-subscriber`
- [ ] 1.5 Verify `nono_proxy::config::ProxyConfig`, `nono_proxy::config::RouteConfig`, `nono_proxy::token::constant_time_eq`, and `nono_proxy::route::RouteStore` are importable — compile a minimal `main.rs` that constructs a `ProxyConfig`

## 2. Phantom Token Gateway — Core Implementation

- [ ] 2.1 Implement env var loading at startup: read `COCO_PHANTOM_TOKEN` (required; exit non-zero if absent), `COCO_LISTEN_PORT` (optional, default `8080`), and upstream credential env vars (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GITHUB_TOKEN`)
- [ ] 2.2 Build `Vec<RouteConfig>` with the three hardcoded route entries (`openai` → `https://api.openai.com`, `anthropic` → `https://api.anthropic.com`, `github` → `https://api.github.com`), setting `credential_key` to the env var name for each
- [ ] 2.3 Implement phantom token validation middleware for Axum: extract `Proxy-Authorization` header, support both `Bearer <token>` and `Basic base64(user:<token>)` formats, validate using `nono_proxy::token::constant_time_eq`; return `407` on failure
- [ ] 2.4 Implement proxy handler: match request path prefix against routes → resolve upstream host → strip `Proxy-Authorization` → use nono-proxy's credential injection (`reverse` module / `RouteConfig` inject mode) → forward request via `hyper-rustls` → stream response back
- [ ] 2.5 Return `404` for unmatched path prefixes and `503` when the required upstream credential env var is absent

## 3. Attestation Endpoint

- [ ] 3.1 Implement `GET /attest` handler: call Phala tappd at `http://localhost:8090/prpc/Tappd.TdxQuote` with a Unix-timestamp nonce in `report_data` using `reqwest`; decode base64 response, hex-encode, return JSON `{ "quote": "<hex>", "platform": "tdx", "debug": <bool> }`
- [ ] 3.2 Parse the raw TDX quote bytes to inspect `td_attributes` bit 0; log `ERROR: TDX debug mode detected` to stderr when set and include `"debug": true` in the response
- [ ] 3.3 Return `503` with an explanatory message from `GET /attest` when tappd is unreachable (local dev / non-Phala environment)

## 4. HTTP Server Wiring

- [ ] 4.1 Wire Axum router: `GET /attest` → attestation handler (no auth); all other routes → token validation middleware → proxy handler
- [ ] 4.2 Bind listener to `0.0.0.0:<COCO_LISTEN_PORT>` and start the Axum server

## 5. Docker Packaging

- [ ] 5.1 Write a multi-stage `Dockerfile`: `cargo-chef` prepare + cook stage, `cargo build --release` stage, minimal runtime stage (`debian:bookworm-slim` with CA certificates). The build fetches `nono-proxy` via Cargo git dep — no submodule checkout needed
- [ ] 5.2 Write `docker-compose.yml` at project root: single `coco-gateway` service, port `8080:8080`, env var pass-throughs for `COCO_PHANTOM_TOKEN`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GITHUB_TOKEN`, `restart: unless-stopped`
- [ ] 5.3 Add a `.dockerignore` excluding `nono/`, `.git/`, `target/`, and local dev files

## 6. Documentation Updates

- [ ] 6.1 Update README.md "Recommended Build Path" section to describe Path B (POC) → Path C (target) evolution
- [ ] 6.2 Update README.md roadmap: push Semantic AI Firewall to Phase 5+
- [ ] 6.3 Add egress enforcement gap section to README.md or DEPLOY.md: describe the threat model weakening, mitigation options (egress firewall, Path C), and Path B vs Path C comparison table
- [ ] 6.4 Document deployment steps in `DEPLOY.md`: provision secrets via `phala cvms secrets set`, push image to GHCR, deploy via Phala dashboard or CLI

## 7. CI and Image Publishing

- [ ] 7.1 Add a GitHub Actions workflow (`.github/workflows/docker.yml`) that builds and pushes the image to GHCR on push to `main`

## 8. Phala Cloud Deployment

- [ ] 8.1 Deploy to Phala Cloud TDX CVM and verify `COCO_PHANTOM_TOKEN` is available as an env var inside the running container

## 9. Validation

- [ ] 9.1 Call `GET /attest` on the deployed gateway and confirm the response contains a hex-encoded TDX DCAP QuoteV4 with `"platform": "tdx"` and no debug flag
- [ ] 9.2 Run end-to-end test: configure an OpenAI Python client with `base_url=https://<cvm-host>/openai/v1` and phantom token, send a chat completion request, confirm a valid response is received
- [ ] 9.3 Confirm via network capture or logging that `OPENAI_API_KEY` never appears in the agent process or outbound request from the agent host
