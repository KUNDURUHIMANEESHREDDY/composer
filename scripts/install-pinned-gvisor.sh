#!/usr/bin/env bash
# Install the UNMODIFIED upstream gVisor release bundle into a Linux Docker host.
# The SHA256 values are GitHub's release-20261123.1 asset digests, reviewed and
# pinned at https://github.com/gvisor/gvisor/releases/tag/release-20261123.1.
# This installer only downloads, checks, extracts and registers upstream runsc.
set -euo pipefail

readonly RELEASE="20261123.1"
readonly UPSTREAM="https://github.com/gvisor/gvisor/releases/download/release-${RELEASE}"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "gVisor runsc requires a Linux Docker host; no Windows fallback" >&2
  exit 1
fi

case "$(uname -m)" in
  x86_64)
    archive="gvisor-x86_64.tar.bz2"
    expected_sha256="95c23cd8eda66eab351717e5ee2b1a5703f848bb73ac855a7b281e1c6cd178e8"
    ;;
  aarch64)
    archive="gvisor-aarch64.tar.bz2"
    expected_sha256="0088b9c58959f58118c27375a1941db31fd9f3c62e484c4ed2094f25dda693bf"
    ;;
  *)
    echo "Unsupported gVisor architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

# Fail BEFORE download or installation if an existing runsc is detected.
# The CI runner is expected to start with a clean environment.
if command -v runsc >/dev/null 2>&1; then
  echo "Refusing to overwrite an already-installed runsc; provision a clean Linux host" >&2
  exit 1
fi

tmp="$(mktemp -d)"
trap 'rm -rf -- "$tmp"' EXIT
curl --fail --location --silent --show-error --retry 3 \
  --output "$tmp/$archive" "$UPSTREAM/$archive"

# Fixed, source-reviewed digest: a compromised upstream mirror or a republished
# asset with the same name cannot silently change the runtime under test.
printf '%s  %s\n' "$expected_sha256" "$tmp/$archive" | sha256sum --check --status || {
  echo "gVisor pinned release SHA256 mismatch: refusing installation" >&2
  exit 1
}

tar -tjf "$tmp/$archive" > "$tmp/entries.txt"
if grep -Eq '^/|(^|/)\.\.(/|$)' "$tmp/entries.txt"; then
  echo "gVisor bundle contains unsafe archive paths" >&2
  exit 1
fi

# Upstream's current bundle ships runsc, the containerd shim, and gvisor-bin
# sidecars together. Do not download/implement replacement sidecars.
sudo tar --no-same-owner -xjf "$tmp/$archive" -C /usr/local/bin
for bin in /usr/local/bin/runsc /usr/local/bin/containerd-shim-runsc-v1; do
  if [[ ! -x "$bin" ]]; then
    echo "Missing executable in pinned gVisor upstream bundle: $bin" >&2
    exit 1
  fi
done
if [[ ! -d /usr/local/bin/gvisor-bin ]]; then
  echo "Pinned gVisor upstream bundle has no sidecar directory" >&2
  exit 1
fi

/usr/local/bin/runsc --version | tee gvisor-version.txt
grep -Fq "$RELEASE" gvisor-version.txt || {
  echo "Unexpected runsc version after installing verified bundle" >&2
  exit 1
}
sudo /usr/local/bin/runsc install
sudo systemctl restart docker

printf '%s  %s\n' "$expected_sha256" "$archive" | tee gvisor-release-sha256.txt
sha256sum /usr/local/bin/runsc | tee gvisor-runsc-binary-sha256.txt
docker info --format '{{json .Runtimes}}' | tee docker-runtimes.json
grep -Fq '"runsc"' docker-runtimes.json || {
  echo "Docker did not register the pinned gVisor runsc runtime" >&2
  exit 1
}
