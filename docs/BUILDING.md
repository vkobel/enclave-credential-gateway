# Building

## StageX OCI Images

`Containerfile.stagex` builds OCI image tarballs for both shipped Rust binaries:

- `server` target: `/usr/bin/enclave-credential-gateway`
- `cli` target: `/usr/bin/gate`

The build stage uses the pinned linux/amd64 [StageX](https://codeberg.org/stagex/stagex)
Rust pallet digest documented in the Containerfile. The release build runs after
`cargo fetch` with `--network=none`, `--frozen`, and `--release`. Runtime images
are `scratch` images containing only the statically linked binary.

```bash
./scripts/build-stagex-oci.sh
```

By default, this writes:

```text
dist/coco-credential-gateway-server.oci.tar
dist/coco-credential-gateway-cli.oci.tar
```

After every build the script prints the hashes, the exact command used to
produce them, and writes a per-commit sums file to the tracked `checksums/`
directory:

```text
checksums/sha256sums-<gitrev>.txt
```

Commit that file alongside the source change so the expected hashes travel
with the code. To verify a later rebuild against committed hashes:

```bash
(cd dist && shasum -a 256 -c ../checksums/sha256sums-<gitrev>.txt)
```

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

To build one target manually (pass the git commit epoch explicitly so the value
is unambiguous):

```bash
SOURCE_DATE_EPOCH=$(git log -1 --pretty=%ct) docker buildx build \
  --platform linux/amd64 \
  --target server \
  --output type=oci,dest=dist/coco-credential-gateway-server.oci.tar,rewrite-timestamp=true \
  -f Containerfile.stagex .
```

Use the `cli` target and a different `dest` for the CLI image.

To check reproducibility against an existing `dist/` build, rebuild into a
separate directory and compare bytes:

```bash
OUTPUT_DIR=/tmp/coco-stagex-repro ./scripts/build-stagex-oci.sh
cmp -s dist/coco-credential-gateway-server.oci.tar /tmp/coco-stagex-repro/coco-credential-gateway-server.oci.tar
cmp -s dist/coco-credential-gateway-cli.oci.tar /tmp/coco-stagex-repro/coco-credential-gateway-cli.oci.tar
```

For a stronger check, force one no-cache server build. The StageX build stage
compiles both binaries, so the CLI target can then be exported from that rebuilt
layer:

```bash
SOURCE_DATE_EPOCH=$(git log -1 --pretty=%ct) docker buildx build \
  --no-cache \
  --platform linux/amd64 \
  --target server \
  --output type=oci,dest=/tmp/coco-stagex-repro/coco-credential-gateway-server.oci.tar,rewrite-timestamp=true \
  -f Containerfile.stagex .

SOURCE_DATE_EPOCH=$(git log -1 --pretty=%ct) docker buildx build \
  --platform linux/amd64 \
  --target cli \
  --output type=oci,dest=/tmp/coco-stagex-repro/coco-credential-gateway-cli.oci.tar,rewrite-timestamp=true \
  -f Containerfile.stagex .
```

Then compare the tarballs with `cmp` or `shasum -a 256`.

The build currently targets linux/amd64 because the StageX Rust pallet reference
is pinned to a linux/amd64 digest. On arm64 hosts this runs under emulation. Add
an explicit arm64 digest before publishing multi-platform release artifacts.
