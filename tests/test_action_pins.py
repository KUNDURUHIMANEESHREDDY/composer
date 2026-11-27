"""Fail CI if any third-party GitHub workflow action floats on a mutable tag.

Only source-reviewed immutable commit hashes are accepted. This is a pinning
check, not proof that an upstream action commit is malicious-code-free.
"""
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
USE = re.compile(r"^\s*(?:-\s*)?uses\s*:\s*([^\s#]+)")
COMMIT_PIN = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.\-/]+@[a-fA-F0-9]{40}$")
DOCKER_DIGEST = re.compile(r"^docker://[^\s@]+@sha256:[a-fA-F0-9]{64}$")


def validate() -> list[str]:
    errors = []
    files = sorted((ROOT / ".github/workflows").glob("*.y*ml"))
    if not files:
        return ["No GitHub Actions workflow files found"]
    for path in files:
        for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            match = USE.match(line)
            if not match:
                continue
            action = match.group(1)
            if action.startswith("./"):
                continue
            if not (COMMIT_PIN.fullmatch(action) or DOCKER_DIGEST.fullmatch(action)):
                errors.append(f"{path.relative_to(ROOT)}:{line_no}: unpinned action {action}")
    return errors


if __name__ == "__main__":
    problems = validate()
    if problems:
        raise SystemExit("\n".join(problems))
    print("PASS all referenced external GitHub Actions pinned to immutable commits")
