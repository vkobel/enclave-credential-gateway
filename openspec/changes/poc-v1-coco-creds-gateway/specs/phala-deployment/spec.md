## ADDED Requirements

### Requirement: Docker image built with multi-stage Dockerfile
The gateway SHALL be packaged as a Docker image using a multi-stage build: a Rust build stage (using `cargo-chef` for layer caching) and a minimal runtime stage (distroless or `debian:bookworm-slim`). The final image SHALL contain only the `coco-gateway` binary and required TLS CA certificates. The build stage SHALL fetch the `nono-proxy` git dependency as part of the normal Cargo build process — no manual module copying or submodule checkout is required in the Dockerfile.

#### Scenario: Image builds successfully
- **WHEN** `docker build` is run from the project root
- **THEN** the build completes without error, fetches `nono-proxy` from git, and produces an image under 150 MB

#### Scenario: Image contains no build toolchain
- **WHEN** the final Docker image is inspected
- **THEN** it contains the `coco-gateway` binary and CA certificates but no Rust compiler, `cargo`, or source files

#### Scenario: No nono submodule required in Docker build
- **WHEN** the Dockerfile is inspected
- **THEN** it does not reference the `nono/` submodule directory — the dependency is resolved via Cargo's git dependency mechanism

### Requirement: docker-compose.yml defines the gateway service
The repository SHALL include a `docker-compose.yml` at the project root that defines a single `coco-gateway` service. It SHALL reference the published image, expose port `8080`, and declare all required environment variables with values sourced from the host environment.

#### Scenario: Compose file is valid
- **WHEN** `docker compose config` is run
- **THEN** it exits without error

#### Scenario: Required env vars declared
- **WHEN** the compose file is inspected
- **THEN** `COCO_PHANTOM_TOKEN`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, and `GITHUB_TOKEN` are declared as environment pass-throughs

### Requirement: Secrets provisioned via Phala CLI before deployment
Before deploying to Phala Cloud, the operator SHALL provision secrets using `phala cvms secrets set`. The gateway SHALL read these secrets from env vars injected by Phala's TEE boot mechanism. No secret values SHALL be present in `docker-compose.yml` or the container image.

#### Scenario: Secrets available as env vars inside container
- **WHEN** the container starts on Phala Cloud after secrets are provisioned
- **THEN** `echo $COCO_PHANTOM_TOKEN` inside the container returns the value set via `phala cvms secrets set`

#### Scenario: No secrets in image or compose file
- **WHEN** the `docker-compose.yml` and container image are inspected
- **THEN** no secret values are present in plaintext (only variable references like `${COCO_PHANTOM_TOKEN}`)

### Requirement: End-to-end validation — OpenAI client through gateway
As a deployment acceptance test, an OpenAI Python client configured with `base_url=https://<cvm-host>/openai/v1` and `api_key=<phantom-token>` (sent via `Proxy-Authorization`) SHALL successfully complete a chat completion request, with the response containing a non-empty `choices` array.

#### Scenario: Agent request succeeds end-to-end
- **WHEN** an OpenAI client sends a `POST /openai/v1/chat/completions` request with the phantom token
- **THEN** the gateway forwards to `api.openai.com`, the real key is injected, and the client receives a valid completion response

#### Scenario: Real API key not visible to agent
- **WHEN** the end-to-end test is performed
- **THEN** the agent process environment and network captures show only the phantom token, never `OPENAI_API_KEY`

### Requirement: GET /attest returns valid TDX quote on Phala deployment
After deployment, the operator SHALL be able to verify the deployment by calling `GET /attest` and confirming the returned quote parses as a valid TDX DCAP QuoteV4 with debug bit unset.

#### Scenario: Attestation validation on deployed instance
- **WHEN** the operator calls `GET https://<cvm-host>/attest`
- **THEN** the response contains a `"quote"` field with a hex-encoded TDX DCAP QuoteV4 and `"platform": "tdx"`
