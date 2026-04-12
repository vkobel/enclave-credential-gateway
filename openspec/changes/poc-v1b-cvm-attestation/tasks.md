## Prerequisite

`poc-v1a-proxy-plain` must be complete and locally validated before starting this change.

---

## 1. Attestation Endpoint

- [ ] 1.1 Add `reqwest` dependency to `crates/coco-gateway/Cargo.toml`
- [ ] 1.2 Implement `GET /attest` handler: call Phala tappd at `http://localhost:8090/prpc/Tappd.TdxQuote` with a Unix-timestamp nonce in `report_data`; decode base64 response, hex-encode, return JSON `{ "quote": "<hex>", "platform": "tdx", "debug": <bool> }`
- [ ] 1.3 Parse the raw TDX quote bytes to inspect `td_attributes` bit 0; log `ERROR: TDX debug mode detected` to stderr when set and include `"debug": true` in the response
- [ ] 1.4 Return `503` with an explanatory message from `GET /attest` when tappd is unreachable
- [ ] 1.5 Wire `GET /attest` into the Axum router (no auth required on this route)

## 2. CI and Image Publishing

- [ ] 2.1 Add a GitHub Actions workflow (`.github/workflows/docker.yml`) that builds and pushes the image to GHCR on push to `main`

## 3. Phala Cloud Deployment

- [ ] 3.1 Deploy to Phala Cloud TDX CVM and verify `COCO_PHANTOM_TOKEN` is available as an env var inside the running container

## 4. Documentation

- [ ] 4.1 Update README.md to document the Phase 1a → 1b two-step POC path
- [ ] 4.2 Add egress enforcement gap section: threat model weakening, mitigation options (egress firewall, Path C), and Path B vs Path C comparison table
- [ ] 4.3 Write `DEPLOY.md`: provision secrets via `phala cvms secrets set`, push image to GHCR, deploy via Phala dashboard or CLI

## 5. CVM Validation

- [ ] 5.1 Call `GET /attest` on the deployed gateway; confirm response contains a hex-encoded TDX DCAP QuoteV4 with `"platform": "tdx"` and no debug flag
- [ ] 5.2 Run end-to-end test: configure an OpenAI Python client with `base_url=https://<cvm-host>/openai/v1` and phantom token; confirm a valid response
- [ ] 5.3 Confirm `OPENAI_API_KEY` never appears in the agent process or outbound requests from the agent host
