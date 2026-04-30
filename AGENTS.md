# Repository Guidelines

## Scope

These instructions apply to the whole repository.

## Project Shape

- `crates/coco-gateway`: Rust gateway, proxy, auth, profiles, admin API, registry, and integration tests.
- `crates/coco-cli`: Rust CLI for token management, env rendering, and tool adapters.
- `profiles/routes/*.yaml` and `profiles/tools/*.yaml`: source of truth for built-in route and tool definitions embedded at build time.
- `examples/profile.json`: compose-mounted example runtime profile.
- `docs/USING.md`: detailed client setup examples. Keep README concise and link here for long per-tool walkthroughs.

## Development Rules

- Keep route and tool behavior consistent across `profiles/routes`, `profiles/tools`, CLI adapter output, README route summaries, and e2e tests.
- Prefer small, reversible diffs. Do not add dependencies unless explicitly needed.
- Prefer `coco activate` in new docs and scripts.
- Do not duplicate long usage guides in README. README should cover overview, quickstart, route locations, core commands, and tests.

## Verification

- Run `cargo fmt --check` and `cargo test --workspace` for normal changes.
- Run `./scripts/test-e2e.sh` for changes touching gateway routing, auth, registry tokens, Docker compose, CLI env/tool behavior, or README testing instructions.
- Live OpenAI/Anthropic e2e checks are optional and should be reported as skipped when credentials are unavailable.
