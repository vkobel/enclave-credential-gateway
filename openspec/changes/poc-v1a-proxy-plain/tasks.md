## 1. Workspace and Library Dependency Setup

- [ ] 1.1 Create a new Cargo workspace at the project root (`Cargo.toml` with `[workspace]`) containing a single member `crates/coco-gateway`
- [ ] 1.2 Scaffold `crates/coco-gateway/` with `Cargo.toml` (binary crate, edition 2021) and `src/main.rs` stub
- [ ] 1.3 Add `nono-proxy` as a git dependency pinned to a specific commit: `nono-proxy = { git = "https://github.com/always-further/nono", rev = "<pin>", package = "nono-proxy" }`
- [ ] 1.4 Add remaining dependencies: `tokio` (full), `axum`, `hyper`, `hyper-rustls`, `tower-http`, `rustls`, `webpki-roots`, `zeroize`, `hex`, `serde`, `serde_json`, `subtle`, `tracing`, `tracing-subscriber`
- [ ] 1.5 Verify `nono_proxy::config::ProxyConfig`, `nono_proxy::config::RouteConfig`, `nono_proxy::token::constant_time_eq`, and `nono_proxy::route::RouteStore` are importable — compile a minimal `main.rs` that constructs a `ProxyConfig`

## 2. Phantom Token Gateway — Core Implementation

- [ ] 2.1 Implement env var loading at startup: read `COCO_PHANTOM_TOKEN` (required; exit non-zero if absent), `COCO_LISTEN_PORT` (optional, default `8080`), and upstream credential env vars (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GITHUB_TOKEN`)
- [ ] 2.2 Build `Vec<RouteConfig>` with the three hardcoded route entries (`/openai/` → `https://api.openai.com`, `/anthropic/` → `https://api.anthropic.com`, `/github/` → `https://api.github.com`), setting `credential_key` to the env var name for each
- [ ] 2.3 Implement phantom token validation middleware for Axum: extract `Proxy-Authorization` header, support both `Bearer <token>` and `Basic base64(user:<token>)` formats, validate using `nono_proxy::token::constant_time_eq`; return `407` on failure
- [ ] 2.4 Implement proxy handler: match request path prefix against routes → resolve upstream host → strip `Proxy-Authorization` → inject real credential → forward request via `hyper-rustls` → stream response back
- [ ] 2.5 Return `404` for unmatched path prefixes and `503` when the required upstream credential env var is absent

## 3. HTTP Server Wiring

- [ ] 3.1 Wire Axum router: all routes → token validation middleware → proxy handler
- [ ] 3.2 Bind listener to `0.0.0.0:<COCO_LISTEN_PORT>` and start the Axum server

## 4. Docker Packaging

- [ ] 4.1 Write a multi-stage `Dockerfile`: `cargo-chef` prepare + cook stage, `cargo build --release` stage, minimal runtime stage (`debian:bookworm-slim` with CA certificates)
- [ ] 4.2 Write `docker-compose.yml` at project root: single `coco-gateway` service, port `8080:8080`, env var pass-throughs for `COCO_PHANTOM_TOKEN`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GITHUB_TOKEN`, `restart: unless-stopped`
- [ ] 4.3 Add a `.dockerignore` excluding `nono/`, `.git/`, `target/`, and local dev files

## 5. Local Validation

- [ ] 5.1 Run `docker compose up` with dummy env vars; confirm server binds and returns `407` on `curl localhost:8080/openai/`
- [ ] 5.2 Set real `COCO_PHANTOM_TOKEN` and `OPENAI_API_KEY`; send an OpenAI chat completion request via the gateway and confirm a valid response
- [ ] 5.3 Confirm `OPENAI_API_KEY` never appears in any outbound request from the host (check via `tcpdump` or mitmproxy)
- [ ] 5.4 Confirm unmatched paths return `404` and missing upstream credential returns `503`
