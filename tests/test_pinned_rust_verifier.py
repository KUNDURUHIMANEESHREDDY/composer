"""Verify Composer CI never silently fetches a mutable Rust verifier tag."""
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/prefetch-pinned-rust-verifier.sh"
DIGEST = "f6c4a35d21dbb3c89d7a8d056c193e94fc09037f2b58dd858b09c39a47dd02d7"

def verify():
    source = SCRIPT.read_text(encoding="utf-8")
    assert f"docker.io/library/rust@sha256:{DIGEST}" in source
    assert "docker pull --platform=linux/amd64" in source
    assert 'docker image tag "$IMAGE" "$TAG"' in source
    assert "docker image inspect --format" in source
    workflows = list((ROOT / ".github/workflows").glob("*.yml"))
    assert len(workflows) >= 15, "No complete CI inventory"
    prefetch_count = 0
    for f in workflows:
        data = f.read_text(encoding="utf-8")
        assert not re.search(r"docker\s+pull\s+rust:1\.99-slim", data), f
        prefetch_count += data.count("bash scripts/prefetch-pinned-rust-verifier.sh")
    assert prefetch_count >= 15, "Expected every Docker verifier acceptance to use the pinned image"
    print(f"PASS {prefetch_count} exact-digest Rust verifier prefetches and no floating tag pulls")

if __name__ == "__main__":
    verify()
