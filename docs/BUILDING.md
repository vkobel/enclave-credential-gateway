# Building

Two layers of reproducibility:

- **StageX OCI image** (below) — the deterministic, byte-for-byte reproducible
  server image. This is the foundation.
- **Caution enclave** ([Caution verification](#caution-verification)) — Caution packs
  the image into an enclave and measures it; reproducibility is then checked through
  Caution's measurements rather than raw tarball hashes.

## StageX OCI Images

Each shipped Rust binary has its own single-target Containerfile:

- `Containerfile.stagex` → `/usr/bin/enclave-credential-gateway` (the server; the
  only artifact deployed to the enclave)
- `Containerfile.cli.stagex` → `/usr/bin/gate` (client-side CLI, never deployed)

Caution deploys only the server image. Both build stages use the pinned linux/amd64 [StageX](https://codeberg.org/stagex/stagex)
Rust pallet digest documented in the Containerfiles. The release build runs after
`cargo fetch` with `--network=none`, `--frozen`, and `--release`. Runtime images
are `scratch` images containing only the statically linked binary.

```bash
./scripts/build-stagex-oci.sh
```

The script runs with `--no-cache` by default so every invocation produces a
clean, reproducible artifact. Set `ALLOW_CACHE=1` to reuse cached layers during
local iteration (faster, but may emit a stale hash that a clean reproduction
won't match). `ALLOW_CACHE=1` is only honored for the default build; `--tag` and
`--check` reject it, since certified and verified hashes must come from a clean
build:

```bash
ALLOW_CACHE=1 ./scripts/build-stagex-oci.sh
```

By default, this writes:

```text
dist/coco-credential-gateway-server.oci.tar
dist/coco-credential-gateway-cli.oci.tar
```

The build prints the artifact hashes. To certify a commit, record them in an
annotated git tag:

```bash
./scripts/build-stagex-oci.sh --tag v0.1.0
git push origin v0.1.0
```

### Continuous verification

Pushing a `v*` tag triggers `.github/workflows/release-verify.yml`. On a clean
amd64 runner it rebuilds the tagged commit `--no-cache` and runs `--check`
against the hashes recorded on the tag — an independent reproduction on
different hardware.

- Pass/fail shows on the tagged commit, the Actions tab, and the README badge.
- A failed reproduction auto-files an issue labelled `reproducibility`.
- Only `v*` tags trigger it; don't tag feature branches you don't intend to
  release.

### Reproducing and verifying a tagged commit

Check out the tag and rebuild; the epoch is the commit's timestamp, so it
matches automatically:

```bash
git checkout v0.1.0
./scripts/build-stagex-oci.sh --check
```

`--check` rebuilds and compares against the hashes on the artifact tag pointing
at `HEAD` (or pass one with `--check <tag>`), exiting non-zero on any mismatch.
Inspect the recorded hashes with `git cat-file tag v0.1.0`.

The OCI manifest digests inside the tarballs can be extracted with:

```bash
tar -xOf dist/coco-credential-gateway-server.oci.tar manifest.json | python3 -m json.tool
```

The script defaults `SOURCE_DATE_EPOCH` to the latest git commit timestamp
(`git log -1 --pretty=%ct`) so any two builds from the same commit produce
identical artifacts. Override it explicitly if needed:

```bash
SOURCE_DATE_EPOCH=0 TARGET_PLATFORM=linux/amd64 OUTPUT_DIR=dist IMAGE_PREFIX=coco-credential-gateway \
  ./scripts/build-stagex-oci.sh
```

To build one image manually (pass the git commit epoch explicitly so the value
is unambiguous):

```bash
SOURCE_DATE_EPOCH=$(git log -1 --pretty=%ct) docker buildx build \
  --platform linux/amd64 \
  --output type=oci,dest=dist/coco-credential-gateway-server.oci.tar,rewrite-timestamp=true \
  -f Containerfile.stagex .
```

Use `-f Containerfile.cli.stagex` and a different `dest` for the CLI image.

To check reproducibility against an existing `dist/` build, rebuild into a
separate directory and compare bytes:

```bash
OUTPUT_DIR=/tmp/coco-stagex-repro ./scripts/build-stagex-oci.sh
cmp -s dist/coco-credential-gateway-server.oci.tar /tmp/coco-stagex-repro/coco-credential-gateway-server.oci.tar
cmp -s dist/coco-credential-gateway-cli.oci.tar /tmp/coco-stagex-repro/coco-credential-gateway-cli.oci.tar
```

linux/amd64 is the only supported reproducible target. The pinned StageX Rust
pallet (`sha256:2fbe7b…`) is a single-arch amd64 OCI image, not a multi-platform
index, so `--platform linux/arm64` has nothing to pull. On arm64 hosts the build
runs the amd64 pallet under emulation.

StageX's toolchain is arch-aware (`packages/core/rust/Containerfile` handles both
`amd64`/`x86_64` and `arm64`/`aarch64`), and both StageX Containerfiles keep the
matching `TARGETARCH` logic, but StageX does not publish a turnkey arm64 pallet
image. Producing arm64 artifacts would require building and pinning a StageX
arm64 rust pallet digest of your own — it is not a drop-in change.

## Caution Verification

The server is deployed through [Caution](https://docs.caution.co/), which packs the
StageX image into a confidential enclave and measures it. Caution's current TEE backing
is AWS Nitro Enclaves; Caution plans to support additional TEEs, so prefer the
Caution-level commands and concepts over substrate-specific details.

The deployment is described by the repo's `Procfile` (`containerfile`, `binary`/`run`,
`app_sources`, `http_port`, `locksmith`). Caution substitutes `${COMMIT}` so the build
is pinned to source.

Build the enclave and read its measurements:

```bash
caution apps build      # produces the enclave image and its measurements (PCR0/1/2 on Nitro)
```

Verify a deployed gateway reproduces from source:

```bash
caution verify --attestation-url https://<gateway>/attestation
```

`caution verify` generates a fresh nonce, fetches the platform attestation plus the
enclave manifest, reproduces the enclave from the manifest's pinned source commits, and
confirms the live measurements match — rejecting debug-mode (zeroed-measurement)
evidence. The byte-for-byte StageX reproducibility above is what makes those
measurements reproducible.

See the [caution-platform deployment guide](https://docs.caution.co/) for QEMU local
testing, the attestation endpoint, and production deployment.
