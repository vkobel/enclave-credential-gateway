# Building

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
won't match):

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
