# Building

## StageX OCI Images

`Containerfile.stagex` builds OCI image tarballs for both shipped Rust binaries:

- `server` target: `/usr/bin/enclave-credential-gateway`
- `cli` target: `/usr/bin/gate`

The build stage uses the pinned linux/amd64 StageX Rust pallet digest documented
in the Containerfile. The release build runs after `cargo fetch` with
`--network=none`, `--frozen`, and `--release`. Runtime images are `scratch`
images containing only the statically linked binary.

```bash
./scripts/build-stagex-oci.sh
```

By default, this writes:

```text
dist/coco-credential-gateway-server.oci.tar
dist/coco-credential-gateway-cli.oci.tar
```

Current expected linux/amd64 OCI tarball hashes:

```text
8d9bd084422e4638acf6bcd355da5c5e8eaa2f562875488f95f728f6376851ee  dist/coco-credential-gateway-server.oci.tar
83f0ddd4d907349d48225970e01be664a80c49727a634e6e2e7ed1fd3634239c  dist/coco-credential-gateway-cli.oci.tar
```

These are the SHA256 hashes of the exported OCI tar files. The OCI manifest
digests inside those tarballs are:

```text
sha256:d7db14622236e4d440e8cb0ad270a213e52f54b233785daa0b6ad042d7318272  server
sha256:c590c5f3fc5fea34f2867197733490d1adf004000e934eb63594a7466ce36441  cli
```

The script sets `SOURCE_DATE_EPOCH=0` unless overridden and asks BuildKit to
rewrite image layer timestamps to that value:

```bash
SOURCE_DATE_EPOCH=0 TARGET_PLATFORM=linux/amd64 OUTPUT_DIR=dist IMAGE_PREFIX=coco-credential-gateway \
  ./scripts/build-stagex-oci.sh
```

To build one target manually:

```bash
SOURCE_DATE_EPOCH=0 docker buildx build \
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
SOURCE_DATE_EPOCH=0 docker buildx build \
  --no-cache \
  --platform linux/amd64 \
  --target server \
  --output type=oci,dest=/tmp/coco-stagex-repro/coco-credential-gateway-server.oci.tar,rewrite-timestamp=true \
  -f Containerfile.stagex .

SOURCE_DATE_EPOCH=0 docker buildx build \
  --platform linux/amd64 \
  --target cli \
  --output type=oci,dest=/tmp/coco-stagex-repro/coco-credential-gateway-cli.oci.tar,rewrite-timestamp=true \
  -f Containerfile.stagex .
```

Then compare the tarballs with `cmp` or `shasum -a 256`.

The build currently targets linux/amd64 because the StageX Rust pallet reference
is pinned to a linux/amd64 digest. On arm64 hosts this runs under emulation. Add
an explicit arm64 digest before publishing multi-platform release artifacts.
