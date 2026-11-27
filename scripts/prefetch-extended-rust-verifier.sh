#!/usr/bin/env bash
set -euo pipefail
# The default verifier is unchanged. These are explicit, narrowly scoped,
# prebuilt runner images; Docker verification still uses --pull=never and
# --network=none with an image content ID and the same sandbox restrictions.
case "${1:-}" in
  nightly-gungraun-0.20.0)
    dockerfile=scripts/Dockerfile.gungraun-verifier
    tag=composer-verifier-gungraun:0.20.0
    ;;
  nightly-rustc-dev-2026-11-25)
    dockerfile=scripts/Dockerfile.rustc-dev-verifier
    tag=composer-verifier-rustc-dev:nightly-2026-11-25
    ;;
  *)
    echo "ERROR unsupported extended verifier profile" >&2
    exit 2
    ;;
esac
docker build --pull=false --platform=linux/amd64 --file "$dockerfile" --tag "$tag" scripts
image_id="$(docker image inspect --format '{{.Id}}' "$tag")"
if [[ ! "$image_id" =~ ^sha256:[0-9a-f]{64}$ ]]; then
    echo "ERROR extended verifier did not produce a Docker content ID" >&2
    exit 1
fi
echo "PASS built explicit extended verifier tag=$tag image_id=$image_id"
