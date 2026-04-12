## ADDED Requirements

### Requirement: Secrets provisioned via Phala CLI before deployment
Before deploying to Phala Cloud, the operator SHALL provision secrets using `phala cvms secrets set`. The gateway SHALL read these secrets from env vars injected by Phala's TEE boot mechanism. No secret values SHALL be present in `docker-compose.yml` or the container image.

#### Scenario: Secrets available as env vars inside container
- **WHEN** the container starts on Phala Cloud after secrets are provisioned
- **THEN** `echo $COCO_PHANTOM_TOKEN` inside the container returns the value set via `phala cvms secrets set`

#### Scenario: No secrets in image or compose file
- **WHEN** the `docker-compose.yml` and container image are inspected
- **THEN** no secret values are present in plaintext (only variable references like `${COCO_PHANTOM_TOKEN}`)

### Requirement: CI publishes image to GHCR on push to main
A GitHub Actions workflow SHALL build and push the `coco-gateway` Docker image to GHCR on every push to `main`.

#### Scenario: Image published on push
- **WHEN** a commit is pushed to `main`
- **THEN** the GitHub Actions workflow builds the image and pushes it to `ghcr.io/<org>/coco-gateway:latest`

### Requirement: End-to-end validation on CVM — OpenAI client through gateway
As a CVM acceptance test, an OpenAI Python client configured with `base_url=https://<cvm-host>/openai/v1` and phantom token SHALL successfully complete a chat completion request.

#### Scenario: Agent request succeeds end-to-end on CVM
- **WHEN** an OpenAI client sends `POST /openai/v1/chat/completions` with the phantom token against the deployed CVM
- **THEN** the gateway forwards to `api.openai.com`, the real key is injected from Phala secrets, and the client receives a valid completion response

#### Scenario: Real API key not visible to agent
- **WHEN** the CVM end-to-end test is performed
- **THEN** the agent process environment and network captures show only the phantom token, never `OPENAI_API_KEY`

### Requirement: GET /attest returns valid TDX quote on deployed CVM
After deployment, the operator SHALL verify the deployment by calling `GET /attest` and confirming the returned quote parses as a valid TDX DCAP QuoteV4 with debug bit unset.

#### Scenario: Attestation validation on deployed instance
- **WHEN** the operator calls `GET https://<cvm-host>/attest`
- **THEN** the response contains a `"quote"` field with a hex-encoded TDX DCAP QuoteV4 and `"platform": "tdx"` with no debug flag
