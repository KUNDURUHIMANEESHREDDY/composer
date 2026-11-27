"""Reuse the already validated held-out behavioral integration harness in 3 more targets.

Same independent mathematical oracle and upstream original algorithm; only the
target Rust crate identifier and frozen target commit vary. No source rewrite.
"""
import argparse
import test_heldout30_functional_anyhow as suite

CASES = {
    "unicode-segmentation": ("unicode-rs/unicode-segmentation", "unicode_segmentation"),
    "num-traits": ("rust-num/num-traits", "num_traits"),
    "bitflags": ("bitflags/bitflags", "bitflags"),
}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--target-case", choices=sorted(CASES), required=True)
    args = parser.parse_args()
    repository, crate_name = CASES[args.target_case]
    suite.TARGET = repository
    suite.INDEPENDENT_ORACLE = suite.INDEPENDENT_ORACLE.replace(
        "use anyhow::ported_totient;", f"use {crate_name}::ported_totient;"
    )
    assert "use anyhow::ported_totient;" not in suite.INDEPENDENT_ORACLE
    suite.main()


if __name__ == "__main__":
    main()
