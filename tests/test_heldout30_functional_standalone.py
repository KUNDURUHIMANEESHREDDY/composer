"""Full-scope functional adoption in frozen standalone held-out targets.

These targets have a root [package] and src/lib.rs at the pinned cohort SHA.
A completed result requires three independent offline Docker full-target test
runs, exact Git provenance, original-target immutability, and rejection of a
type-compatible wrong implementation. Failures remain failures.
"""
import argparse
import test_heldout30_functional_anyhow as suite

CASES = {
    "bytes": ("tokio-rs/bytes", "bytes"),
    "serde-json": ("serde-rs/json", "serde_json"),
    "hashbrown": ("rust-lang/hashbrown", "hashbrown"),
    "httparse": ("seanmonstar/httparse", "httparse"),
    "rand": ("rust-random/rand", "rand"),
    "smol": ("smol-rs/smol", "smol"),
    "hyper": ("hyperium/hyper", "hyper"),
    "mini-redis": ("tokio-rs/mini-redis", "mini_redis"),
}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--target-case", choices=sorted(CASES), required=True)
    args = parser.parse_args()
    repo, crate = CASES[args.target_case]
    suite.TARGET = repo
    suite.INDEPENDENT_ORACLE = suite.INDEPENDENT_ORACLE.replace(
        "use anyhow::ported_totient;", f"use {crate}::ported_totient;"
    )
    suite.main()


if __name__ == "__main__":
    main()
