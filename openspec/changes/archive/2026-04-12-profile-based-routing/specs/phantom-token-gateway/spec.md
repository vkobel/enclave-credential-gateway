## MODIFIED Requirements

### Requirement: Credential injection uses per-route header and format config
After route dispatch, the gateway SHALL inject the upstream credential using the route's configured `inject_header` and `credential_format`. The `{}` placeholder in `credential_format` SHALL be replaced with the credential value. The default `inject_header` is `Authorization` and the default `credential_format` is `Bearer {}`.

#### Scenario: Default header injection (Authorization: Bearer)
- **WHEN** a validated request is routed to a route with no explicit injection config
- **THEN** the forwarded request has `Authorization: Bearer <credential>` and no `Proxy-Authorization` header

#### Scenario: Custom header injection (x-api-key)
- **WHEN** a route is configured with `inject_header: "x-api-key"` and `credential_format: "{}"`
- **THEN** the forwarded request carries `x-api-key: <credential>` (no `Bearer` prefix)

#### Scenario: Missing credential for route
- **WHEN** the route's `credential_env` var is not set or empty at request time
- **THEN** the gateway returns `503 Service Unavailable` without forwarding
