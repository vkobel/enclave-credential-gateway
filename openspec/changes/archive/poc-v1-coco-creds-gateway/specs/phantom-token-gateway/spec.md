## ADDED Requirements

### Requirement: Gateway binds to all interfaces on a configured port
The gateway SHALL bind its HTTP listener to `0.0.0.0` on a configurable port (default `8080`) so it is reachable from outside the container via Phala's HTTPS ingress.

#### Scenario: Default port binding
- **WHEN** the gateway starts without a `COCO_LISTEN_PORT` env var
- **THEN** it listens on `0.0.0.0:8080`

#### Scenario: Configurable port binding
- **WHEN** `COCO_LISTEN_PORT=9090` is set in the environment
- **THEN** the gateway listens on `0.0.0.0:9090`

### Requirement: Phantom token loaded from environment variable
The gateway SHALL load the phantom token from the `COCO_PHANTOM_TOKEN` environment variable at startup. If this variable is absent or empty, the gateway SHALL refuse to start and exit with a non-zero status.

#### Scenario: Token present at startup
- **WHEN** `COCO_PHANTOM_TOKEN` is set to a non-empty string
- **THEN** the gateway starts successfully and uses that value for authentication

#### Scenario: Token absent at startup
- **WHEN** `COCO_PHANTOM_TOKEN` is not set or is empty
- **THEN** the gateway logs an error and exits with a non-zero status code before accepting any connections

### Requirement: Phantom token validation on every request
The gateway SHALL validate the `Proxy-Authorization` header on every proxied request using `nono_proxy::token::constant_time_eq`. It SHALL accept both `Bearer <token>` and `Basic base64(username:<token>)` formats. Requests missing this header or presenting an incorrect token SHALL receive a `407 Proxy Authentication Required` response and MUST NOT be forwarded upstream.

#### Scenario: Valid phantom token
- **WHEN** a request carries `Proxy-Authorization: Bearer <correct-token>`
- **THEN** the request proceeds to route dispatch

#### Scenario: Missing Proxy-Authorization header
- **WHEN** a request has no `Proxy-Authorization` header
- **THEN** the gateway returns `407 Proxy Authentication Required` and does not forward the request

#### Scenario: Wrong token value
- **WHEN** a request carries `Proxy-Authorization: Bearer <wrong-token>`
- **THEN** the gateway returns `407 Proxy Authentication Required` and does not forward the request

#### Scenario: Constant-time comparison prevents timing oracle
- **WHEN** two requests arrive with tokens of equal length but different values
- **THEN** the comparison time SHALL not differ in a way that reveals the correct token (constant-time comparison enforced)

### Requirement: Path-prefix route dispatch
The gateway SHALL use `nono_proxy::route::RouteStore` for path-prefix routing, configured programmatically with `RouteConfig` structs. The minimum required prefixes for the POC are:

| Prefix | Upstream host | Credential env var |
|--------|--------------|-------------------|
| `openai` | `https://api.openai.com` | `OPENAI_API_KEY` |
| `anthropic` | `https://api.anthropic.com` | `ANTHROPIC_API_KEY` |
| `github` | `https://api.github.com` | `GITHUB_TOKEN` |

Requests whose path does not match any registered prefix SHALL receive `404 Not Found`.

#### Scenario: OpenAI prefix routing
- **WHEN** a validated request targets `/openai/v1/chat/completions`
- **THEN** the gateway forwards it to `https://api.openai.com/v1/chat/completions`

#### Scenario: Unknown prefix
- **WHEN** a validated request targets `/unknown/something`
- **THEN** the gateway returns `404 Not Found` and does not make any upstream connection

### Requirement: Credential injection and phantom token stripping
After route dispatch, the gateway SHALL use nono-proxy's `reverse` module and `credential::CredentialStore` for credential injection:
1. Remove the `Proxy-Authorization` header from the outgoing request
2. Inject the upstream credential using `RouteConfig`'s `inject_mode` (header, url_path, query_param, or basic_auth) — defaulting to `Authorization: Bearer <upstream-key>` for header mode
3. Forward the modified request to the upstream over TLS

The credential store SHALL be configured to load credentials from environment variables (Phala secrets). If the required credential for the matched route is absent or empty, the gateway SHALL return `503 Service Unavailable` to the client without forwarding the request.

#### Scenario: Credential injected for OpenAI route
- **WHEN** a validated request is routed to `/openai/` and `OPENAI_API_KEY` is set
- **THEN** the forwarded request has `Authorization: Bearer <OPENAI_API_KEY>` and no `Proxy-Authorization` header

#### Scenario: Missing credential for route
- **WHEN** `OPENAI_API_KEY` is not set and a request is routed to `/openai/`
- **THEN** the gateway returns `503 Service Unavailable` without forwarding

### Requirement: Response streaming from upstream
The gateway SHALL stream the upstream response body back to the client without buffering the full body. This is required for SSE (server-sent events) and streaming completions from LLM providers.

#### Scenario: Streaming completion response
- **WHEN** the upstream returns a chunked or streaming response
- **THEN** the gateway begins forwarding chunks to the client before the upstream response is complete

### Requirement: Outbound TLS to upstreams using rustls
All connections from the gateway to upstream APIs SHALL use TLS, verified against the system CA store, implemented with `rustls`. Plaintext outbound connections SHALL NOT be permitted.

#### Scenario: Outbound connection is TLS
- **WHEN** the gateway forwards a request to `api.openai.com`
- **THEN** the connection uses TLS 1.2 or higher and verifies the upstream certificate

### Requirement: Gateway uses nono-proxy as a library dependency
The gateway SHALL depend on `nono-proxy` as a Cargo git dependency (pinned to a specific commit). It SHALL NOT copy or vendor nono-proxy source files. The following nono-proxy modules SHALL be consumed directly:
- `nono_proxy::route::RouteStore` for path-prefix routing
- `nono_proxy::config::{ProxyConfig, RouteConfig}` for route configuration
- `nono_proxy::token::constant_time_eq` for phantom token validation
- `nono_proxy::credential::CredentialStore` for credential loading
- `nono_proxy::reverse` for upstream request forwarding and credential injection
- `nono_proxy::filter::ProxyFilter` for host filtering and deny lists

The gateway SHALL NOT use `nono_proxy::start()` or `nono_proxy::server::ProxyHandle`, as these force ephemeral token generation incompatible with the pre-shared remote token model.

#### Scenario: Cargo.toml declares nono-proxy git dependency
- **WHEN** the `Cargo.toml` for `coco-gateway` is inspected
- **THEN** it contains a `nono-proxy` dependency pointing to the `always-further/nono` git repository at a pinned commit hash

#### Scenario: No vendored nono-proxy source files
- **WHEN** the `crates/coco-gateway/src/` directory is inspected
- **THEN** it does not contain copies of `reverse.rs`, `route.rs`, `token.rs`, `keystore.rs`, or `filter.rs`

### Requirement: Custom Axum server composes nono-proxy modules
The gateway SHALL run a custom Axum HTTP server (not nono-proxy's built-in TCP server). The Axum router SHALL:
1. Handle `GET /attest` via the attestation handler (no token required)
2. Route all other requests through token validation middleware, then to the proxy handler which uses nono-proxy's `reverse` module for upstream forwarding

#### Scenario: Axum server handles both attestation and proxy routes
- **WHEN** the gateway starts
- **THEN** a single Axum server on port `COCO_LISTEN_PORT` serves both `/attest` (unauthenticated) and all other paths (token-validated proxy)

### Requirement: Egress enforcement gap documented as known POC limitation
The gateway documentation SHALL explicitly state that in the remote proxy model (Path B), agents route traffic through the gateway voluntarily via `BASE_URL` configuration. Unlike the local nono model where Landlock enforces `NetworkMode::ProxyOnly` at the kernel level, a compromised remote agent can bypass the gateway and contact upstream APIs directly. Documentation SHALL recommend mitigations: cloud egress firewall rules, eBPF-based egress filtering, or upgrading to Path C.

#### Scenario: Documentation includes egress gap warning
- **WHEN** an operator reads the deployment documentation
- **THEN** it contains a clearly labeled section describing the egress enforcement gap and at least two concrete mitigation strategies
