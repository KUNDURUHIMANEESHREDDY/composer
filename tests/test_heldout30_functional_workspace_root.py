"""Pinned held-out repository root package with Cargo workspace members.

The existing upstream Git verifier imports a real reviewed capability into
the repository's ROOT library package. Cargo's default package selection is
used; this is not a full-workspace-member integration claim.
"""
import argparse
import test_heldout30_functional_anyhow as suite

CASES = {
    "regex": ("rust-lang/regex", "regex"),
    "rayon": ("rayon-rs/rayon", "rayon"),
    "http": ("hyperium/http", "http"),
    "clap": ("clap-rs/clap", "clap"),
    "indexmap": ("bluss/indexmap", "indexmap"),
    "walkdir": ("BurntSushi/walkdir", "walkdir"),
    "syn": ("dtolnay/syn", "syn"),
    "uuid": ("uuid-rs/uuid", "uuid"),
    "crossbeam": ("crossbeam-rs/crossbeam", "crossbeam"),
    "thiserror": ("dtolnay/thiserror", "thiserror"),
}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--target-case", choices=sorted(CASES), required=True)
    case = parser.parse_args().target_case
    repo, crate = CASES[case]
    suite.TARGET = repo
    suite.EXPECT_WORKSPACE = True
    suite.INDEPENDENT_ORACLE = suite.INDEPENDENT_ORACLE.replace(
        "use anyhow::ported_totient;", f"use {crate}::ported_totient;"
    )
    suite.main()


if __name__ == "__main__":
    main()
