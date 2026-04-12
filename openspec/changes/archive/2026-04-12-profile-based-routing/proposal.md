## Why

The hardcoded routes (OpenAI, Anthropic, GitHub) mean every new upstream requires a code change and redeploy. Operators need a way to define any route → upstream → credential mapping at boot time, without touching the binary.

## What Changes

- **New `profile.json` format**: a JSON file describing named routes with upstream URL, env var for credential, and optional injection config (`inject_header`, `credential_format`). Loaded at startup from a configurable path (`COCO_PROFILE`, default `/etc/coco/profile.json`).
- **Route table driven by profile**: if a profile is found, it replaces the hardcoded routes entirely. If no profile is found, the built-in defaults (openai / anthropic / github) are used as fallback.
- **Flexible credential injection**: routes can specify `inject_header` (default `Authorization`) and `credential_format` (default `Bearer {}`), enabling services that use `x-api-key` or other schemes.
- **Updated README**: concise quickstart with Docker run example and profile format reference.

## Capabilities

### New Capabilities

- `profile-routing`: JSON profile file that defines the gateway's route table at boot time — prefix → upstream + credential env var + optional injection overrides.

### Modified Capabilities

- `phantom-token-gateway`: credential injection now driven by profile config rather than hardcoded routes; `inject_header` and `credential_format` are now per-route configurable.

## Impact

- `crates/coco-gateway/src/main.rs`: replace hardcoded `route_definitions` with profile loader; `RouteEntry` gains `inject_header` and `credential_format` fields
- `Dockerfile` / `docker-compose.yml`: document volume mount for profile file
- `README.md`: rewritten quickstart
- No new Cargo dependencies (pure `serde_json` deserialization)
