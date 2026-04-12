## Context

Phase 1a (`poc-v1a-proxy-plain`) delivered a working `coco-gateway` binary that proxies requests, validates phantom tokens, and injects credentials — running on any Docker-capable machine. Phase 1b takes that same binary and:

1. Adds the `GET /attest` endpoint using Phala's `tappd` sidecar
2. Deploys to Phala Cloud TDX CVM with secrets injected via Phala's encrypted secret mechanism
3. Validates end-to-end with TDX attestation verified

The key constraints:
- Phala Cloud runs Docker Compose workloads inside Intel TDX CVMs.
- TLS terminates at Phala's edge infrastructure (outside the enclave) — full attested TLS is a next-step, not a POC requirement.
- Phala injects operator secrets as env vars via X25519+AES-256-GCM encryption; plaintext only accessible inside the TEE at runtime.
- `tappd` is already running in every Phala CVM with no additional setup.

## Goals / Non-Goals

**Goals:**
- Add `GET /attest` endpoint to `coco-gateway`, fetching TDX DCAP QuoteV4 from Phala tappd
- Assert debug bit is unset in the quote; log hard error if debug mode detected
- Deploy to Phala Cloud TDX CVM using the existing `docker-compose.yml`
- Provision secrets via `phala cvms secrets set`
- Publish image to GHCR via GitHub Actions
- Validate: `GET /attest` returns valid TDX quote; OpenAI client request succeeds end-to-end on CVM

**Non-Goals:**
- In-enclave TLS termination (Phala edge handles TLS)
- Multi-tenancy or per-agent identity
- Portable encrypted vaults (Phala secrets are sufficient for POC)
- Reproducible builds / MRTD pinning
- Rate limiting, token budgets, audit logging

## Decisions

### D1: Use Phala tappd sidecar for TDX quote generation (not attestation-rs)

**Decision:** Obtain the TDX quote by calling Phala's `tappd` sidecar at `http://localhost:8090/prpc/Tappd.TdxQuote` via `reqwest`. No external attestation library needed for the POC.

**Rationale:** `tappd` is already running in every Phala CVM — zero additional dependencies, ~15 lines of Rust. `attestation-rs` is the right abstraction for multi-platform support (Azure TDX, GCP, AWS Nitro) but adds build complexity the POC doesn't need yet.

**Alternatives considered:**
- `attestation-rs` (lunal-dev): Clean multi-platform abstraction. Deferred to post-POC when multi-platform matters.
- Direct ioctl to `/dev/tdx_guest`: Works but requires unsafe code and manual quote parsing.

### D2: GET /attest requires no authentication

**Decision:** The `/attest` endpoint is unauthenticated — any caller can fetch the attestation quote without a phantom token.

**Rationale:** The attestation quote is operator-transparency data, not sensitive. Operators and auditors need to verify the binary before provisioning credentials. Requiring a token would mean the operator needs credentials before they can verify what they're credentialing — circular.

### D3: Graceful 503 when tappd is unavailable

**Decision:** When tappd is unreachable, `GET /attest` returns `503 Service Unavailable` with an explanatory message. The gateway continues operating normally and the proxy endpoints remain functional.

**Rationale:** The Phase 1a binary already runs without tappd (local/plain VM). Phase 1b should not break that. The 503 is a clear signal that attestation is unavailable without taking the whole gateway down.

### D4: Report data nonce from Unix timestamp

**Decision:** Include the current Unix timestamp (seconds) as the `report_data` nonce in the tappd request.

**Rationale:** Prevents replayed quotes — operators can check the timestamp is recent. Simple to implement, no shared state required.

## Risks / Trade-offs

- **TLS terminates outside the enclave** → For the POC this is acceptable; traffic between Phala's edge and the container is on the internal CVM network. Document as known POC limitation. Phase 2 adds in-enclave TLS.
- **Pre-shared phantom token is long-lived** → If an agent host is compromised, the attacker holds the token until rotation. Mitigation: token is only useful against this gateway; real keys never leave the enclave. Rotation requires one Phala secret update and redeploy.
- **Debug mode TDX quote accepted silently** → If deployed on a debug TDX instance, the attestation is meaningless. Mitigation: gateway asserts debug bit at `/attest` and logs hard error.
- **Egress enforcement gap (inherited from Phase 1a)** → Documented with mitigation guidance in DEPLOY.md.

## Open Questions

- Does Phala's current CVM networking model allow the container to make outbound TLS connections directly, or does traffic route through a Phala egress proxy? (Expected: direct outbound is supported; verify during deployment.)
- What is the exact tappd request/response format for `Tappd.TdxQuote`? (Resolve: check Phala dstack documentation or inspect a running tappd instance.)
