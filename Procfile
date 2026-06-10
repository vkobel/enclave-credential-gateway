# Caution Procfile - https://docs.caution.co/reference/procfile/
# Enclave Credential Gateway (server) — reproducible StageX build.

# --- Build ---------------------------------------------------------------
# Caution runs `docker build -f <containerfile> .` and deploys the LAST stage.
# Containerfile.stagex must therefore END with the `server` stage — the `cli`
# stage is a client-side tool and must never be the deployed enclave image.
containerfile: Containerfile.stagex

# The server stage is a fully static (crt-static musl) binary on `scratch` with
# TLS roots compiled in (webpki-roots), so upstream HTTPS needs no CA bundle.
# Extract just the binary so PCR2 measures only the gateway, nothing else.
binary: /usr/bin/enclave-credential-gateway
run: /usr/bin/enclave-credential-gateway

# --- Source verification -------------------------------------------------
# Caution substitutes ${COMMIT}; lets `caution verify` reproduce from source.
app_sources: https://github.com/vkobel/enclave-credential-gateway/archive/${COMMIT}.tar.gz

# --- Networking ----------------------------------------------------------
# Gateway binds 0.0.0.0:8080 (override with GATE_LISTEN_PORT). http_port puts
# Caution's Caddy in front with TLS on 443 — this replaces the repo's local
# Caddyfile / docker-compose caddy service in production.
http_port: 8080

# --- Secrets -------------------------------------------------------------
# Real upstream credentials + admin token must be decrypted ONLY inside the
# enclave. Do NOT bake them into the image or this Procfile. Provision via
# Locksmith: GATE_ADMIN_TOKEN, OPENAI_API_KEY, ANTHROPIC_API_KEY, GITHUB_TOKEN.
locksmith: true

# --- Resources (defaults shown; fine for this proxy) ---------------------
# memory: 512
# cpus: 2

# --- Optional ------------------------------------------------------------
# domain: gateway.example.com
e2e: true   # steve handles /e2p/* on the reserved port; admin CLI uses it for encrypted admin ops
# debug: false   # NEVER true in prod: zeros PCRs and opens SSH on port 22.
