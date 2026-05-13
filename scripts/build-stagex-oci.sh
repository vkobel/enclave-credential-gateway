#!/bin/sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
IMAGE_PREFIX="${IMAGE_PREFIX:-coco-credential-gateway}"
OUTPUT_DIR="${OUTPUT_DIR:-${ROOT}/dist}"
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-0}"
TARGET_PLATFORM="${TARGET_PLATFORM:-linux/amd64}"

case "$OUTPUT_DIR" in
	/*) ;;
	*) OUTPUT_DIR="${ROOT}/${OUTPUT_DIR}" ;;
esac

mkdir -p "$OUTPUT_DIR"

build_target() {
	target="$1"
	name="$2"

	SOURCE_DATE_EPOCH="$SOURCE_DATE_EPOCH" docker buildx build \
		--platform "$TARGET_PLATFORM" \
		--target "$target" \
		--output "type=oci,dest=${OUTPUT_DIR}/${name}.oci.tar,rewrite-timestamp=true" \
		-f "$ROOT/Containerfile.stagex" \
		"$ROOT"
}

build_target server "${IMAGE_PREFIX}-server"
build_target cli "${IMAGE_PREFIX}-cli"
