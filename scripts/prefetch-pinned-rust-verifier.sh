#!/usr/bin/env bash
set -euo pipefail

# Exact OCI index digest of official upstream library/rust:1.99-slim.
# Resolved against Docker Hub on 2026-11-27; this is not proof of the
# publisher's security posture. Do not replace by floating rust:1.99-slim.
readonly IMAGE='docker.io/library/rust@sha256:f6c4a35d21dbb3c89d7a8d056c193e94fc09037f2b58dd858b09c39a47dd02d7'
readonly TAG='rust:1.99-slim'

docker pull --platform=linux/amd64 "$IMAGE"
# Composer's existing verifier uses this stable local tag with --pull=never.
# Only alias the already digest-verified content; never fetch tag contents.
docker image tag "$IMAGE" "$TAG"
source_id="$(docker image inspect --format '{{.Id}}' "$IMAGE")"
tag_id="$(docker image inspect --format '{{.Id}}' "$TAG")"
if [[ -z "$source_id" || "$source_id" != "$tag_id" ]]; then
  echo 'ERROR Docker verifier image/tag mismatch' >&2
  exit 1
fi
echo "PASS exact-digest upstream Rust verifier image $IMAGE"
