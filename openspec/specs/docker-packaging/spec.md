## Requirements

### Requirement: Docker image built with multi-stage Dockerfile
The gateway SHALL be packaged as a Docker image using a multi-stage build: a Rust build stage (using `cargo-chef` for layer caching) and a minimal runtime stage (`debian:bookworm-slim` with CA certificates). The final image SHALL contain only the `coco-gateway` binary and required TLS CA certificates. The build stage SHALL fetch the `nono-proxy` git dependency as part of the normal Cargo build — no submodule checkout required.

#### Scenario: Image builds successfully
- **WHEN** `docker build` is run from the project root
- **THEN** the build completes without error, fetches `nono-proxy` from git, and produces an image under 150 MB

#### Scenario: Image contains no build toolchain
- **WHEN** the final Docker image is inspected
- **THEN** it contains the `coco-gateway` binary and CA certificates but no Rust compiler, `cargo`, or source files

#### Scenario: No nono submodule required in Docker build
- **WHEN** the Dockerfile is inspected
- **THEN** it does not reference the `nono/` submodule directory

### Requirement: docker-compose.yml defines the gateway service
The repository SHALL include a `docker-compose.yml` at the project root that defines a single `coco-gateway` service, exposes port `8080`, and declares all required environment variables sourced from the host environment.

#### Scenario: Compose file is valid
- **WHEN** `docker compose config` is run
- **THEN** it exits without error

#### Scenario: Required env vars declared
- **WHEN** the compose file is inspected
- **THEN** `COCO_PHANTOM_TOKEN`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, and `GITHUB_TOKEN` are declared as environment pass-throughs

### Requirement: Local validation — proxy data plane works end-to-end
As a local acceptance test, an OpenAI client configured with `base_url=http://localhost:8080/openai/v1` and phantom token SHALL successfully complete a chat completion request.

#### Scenario: Agent request succeeds end-to-end locally
- **WHEN** an OpenAI client sends `POST /openai/v1/chat/completions` with the phantom token via `docker compose up`
- **THEN** the gateway forwards to `api.openai.com`, the real key is injected, and the client receives a valid completion response

#### Scenario: Real API key not visible to agent
- **WHEN** the local test is performed
- **THEN** the agent process environment shows only the phantom token, never `OPENAI_API_KEY`
