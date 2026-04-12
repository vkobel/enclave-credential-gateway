## ADDED Requirements

### Requirement: Attestation endpoint available at GET /attest
The gateway SHALL expose a `GET /attest` HTTP endpoint that returns the current TDX attestation quote. This endpoint SHALL NOT require phantom token authentication — it is intended for operator transparency and pre-trust verification.

#### Scenario: Successful attestation response
- **WHEN** a client sends `GET /attest`
- **THEN** the gateway returns HTTP `200 OK` with `Content-Type: application/json` and a body of the form:
  ```json
  { "quote": "<hex-encoded-TDX-DCAP-QuoteV4>", "platform": "tdx" }
  ```

#### Scenario: Attestation endpoint requires no authentication
- **WHEN** a client sends `GET /attest` without a `Proxy-Authorization` header
- **THEN** the gateway returns `200 OK` with the attestation JSON (not `407`)

### Requirement: Quote fetched from Phala tappd sidecar
The gateway SHALL obtain the TDX quote by calling the Phala `tappd` sidecar at `http://localhost:8090/prpc/Tappd.TdxQuote` using `reqwest`. The base64-encoded response SHALL be hex-encoded before returning.

#### Scenario: TDX quote fetched from tappd
- **WHEN** a client requests `GET /attest` on a Phala Cloud CVM
- **THEN** the gateway calls tappd, decodes the base64 response, hex-encodes it, and returns it in the JSON response

#### Scenario: tappd unavailable
- **WHEN** the gateway runs outside a Phala CVM and tappd is not reachable
- **THEN** `GET /attest` returns `503 Service Unavailable` with an explanatory message; the gateway continues operating normally

### Requirement: Debug mode assertion
The gateway SHALL inspect the `td_attributes` field of the quote and assert that bit 0 (debug mode) is unset. If debug mode is detected, the gateway SHALL log a hard error to stderr and include `"debug": true` in the response.

#### Scenario: Production TDX instance (debug bit unset)
- **WHEN** the gateway runs on a production Phala Cloud TDX instance
- **THEN** the `/attest` response does not include `"debug": true`

#### Scenario: Debug TDX instance
- **WHEN** the gateway detects debug mode in the TDX quote
- **THEN** the gateway logs `ERROR: TDX debug mode detected — attestation is not trustworthy` to stderr and the `/attest` response includes `"debug": true`

### Requirement: Report data nonce binds quote to session
The gateway SHALL include a nonce derived from the current Unix timestamp (seconds) in the `report_data` field passed to tappd. This allows operators to detect replayed quotes.

#### Scenario: Quote contains timestamp nonce
- **WHEN** a client requests `GET /attest`
- **THEN** the returned quote's `report_data` field encodes the current Unix timestamp
