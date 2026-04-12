## 1. Profile Data Model

- [x] 1.1 Add `ProfileRoute` and `Profile` structs to `main.rs` with `serde::Deserialize`; `inject_header` and `credential_format` optional with defaults
- [x] 1.2 Extend `RouteEntry` with `inject_header: String` and `credential_format: String` fields

## 2. Profile Loading

- [x] 2.1 Implement `load_profile()` function: resolve path from `COCO_PROFILE` → `/etc/coco/profile.json` → `None`; parse JSON; skip routes missing `credential_env` with a warning; return `Vec<(String, RouteEntry)>`
- [x] 2.2 In `main()`, replace the hardcoded `route_definitions` block with a call to `load_profile()`; log "loaded N routes from profile at <path>" or "no profile found, using built-in defaults"

## 3. Credential Injection

- [x] 3.1 Update `proxy_handler` to use `entry.inject_header` and `entry.credential_format` (replace `{}` with credential) instead of the hardcoded `"Authorization: Bearer {}"` string

## 4. Docker Integration

- [x] 4.1 Add an example `profile.json` at project root (`examples/profile.json`) with httpbin, openai, anthropic, and github routes as reference
- [x] 4.2 Update `docker-compose.yml` to show profile volume mount as a comment (not active by default)

## 5. README

- [x] 5.1 Rewrite `README.md` with: what it does (2 sentences), quickstart (docker run with env vars), profile format reference, and curl example hitting the gateway
