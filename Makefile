.PHONY: build test fmt check e2e install build-oci build-enclave deploy verify

# --- Development ---------------------------------------------------------

build:
	cargo build --workspace

test:
	cargo test --workspace

fmt:
	cargo fmt

check:
	cargo fmt --check
	cargo test --workspace

e2e:
	./scripts/test-e2e.sh

# Install the gate CLI to /usr/local/bin
install:
	cargo build --release -p gate-cli
	cp target/release/gate /usr/local/bin/gate

# --- Reproducible OCI build ----------------------------------------------

build-oci:
	./scripts/build-stagex-oci.sh

# --- Caution enclave -----------------------------------------------------

# Build the enclave image locally (inspect PCRs, QEMU-test).
# Requires: caution CLI, amd64 Docker (use OrbStack ubuntu-amd64 on Apple Silicon).
build-enclave:
	caution apps build

# Deploy (or redeploy) to Caution. Runs caution init if .caution/ is absent.
deploy:
	@[ -d .caution ] || caution init
	caution apps create

# Verify a live deployment reproduces from source.
# ATTESTATION_URL must be set: make verify ATTESTATION_URL=https://gw.example.com
verify:
	@[ -n "$(ATTESTATION_URL)" ] || (echo "set ATTESTATION_URL=https://..." && exit 1)
	caution verify --attestation-url $(ATTESTATION_URL)
