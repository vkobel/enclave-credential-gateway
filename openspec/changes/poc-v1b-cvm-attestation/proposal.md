## Why

With the proxy data plane proven on plain infrastructure (Phase 1a / `poc-v1a-proxy-plain`), this phase promotes the same binary to a Phala Cloud TDX Confidential VM — the actual target environment. The key additions are the `/attest` endpoint (operators can verify the binary running in the enclave) and Phala secret injection (real credentials sealed to the TEE, never visible to the host).

This is Phase 1b of the POC. It has a hard prerequisite: `poc-v1a-proxy-plain` must be complete and validated.

## What Changes

- **`GET /attest` endpoint:** Added to the existing `coco-gateway` binary. Calls Phala's `tappd` sidecar at `http://localhost:8090/prpc/Tappd.TdxQuote` to fetch a raw TDX DCAP QuoteV4, hex-encodes it, and returns JSON `{ "quote": "<hex>", "platform": "tdx", "debug": <bool> }`. Returns `503` when tappd is unreachable (graceful degradation for non-Phala environments). Asserts debug bit is unset.
- **Phala Cloud deployment:** The same `docker-compose.yml` from Phase 1a is deployed on a Phala CVM. Secrets (`COCO_PHANTOM_TOKEN`, `OPENAI_API_KEY`, etc.) provisioned via `phala cvms secrets set` — injected as env vars inside the TEE at boot, never visible to the host.
- **CI / image publishing:** GitHub Actions workflow builds and pushes the image to GHCR on push to `main`.
- **Documentation:** Deployment guide (`DEPLOY.md`), egress enforcement gap section, Phase 1a → 1b upgrade notes.

## Capabilities

### New Capabilities

- `attestation-endpoint`: `GET /attest` endpoint that fetches and serves a raw TDX DCAP QuoteV4 via Phala's tappd sidecar. Includes debug-mode assertion and returns JSON with quote hex and platform identifier.
- `phala-deployment`: Phala Cloud deployment workflow: secret provisioning via `phala cvms secrets set`, GHCR image publishing, and CVM deployment via Docker Compose.

### Modified Capabilities

- `phantom-token-gateway` (from `poc-v1a-proxy-plain`): Axum router updated to add the unauthenticated `GET /attest` route alongside the existing proxy routes.

## Impact

- **Dependency added:** `reqwest` for tappd HTTP call
- **Deployment:** Phala Cloud TDX CVM; requires `phala` CLI for secret provisioning
- **CI:** `.github/workflows/docker.yml` added
- **Egress enforcement gap:** Documented prominently; mitigation options (egress firewall, Path C) included in DEPLOY.md
- **No breaking changes** to the proxy behavior established in Phase 1a
