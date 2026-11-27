"""Full behavior tests of explicitly selected library members in exact pinned
virtual Cargo workspaces. These do not claim entire-workspace test coverage.
"""
import argparse
import test_heldout30_functional_anyhow as suite

CASES = {
    "rustls": ("rustls/rustls", "rustls", "rustls"),
    "tower": ("tower-rs/tower", "tower", "tower"),
    "futures": ("rust-lang/futures-rs", "futures", "futures"),
    "tracing": ("tokio-rs/tracing", "tracing", "tracing"),
    "petgraph": ("petgraph/petgraph", "crates/petgraph", "petgraph"),
    "rkyv": ("rkyv/rkyv", "rkyv", "rkyv"),
    "serde": ("serde-rs/serde", "serde", "serde"),
}

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--target-case", choices=sorted(CASES), required=True)
    case = parser.parse_args().target_case
    repo, member, crate = CASES[case]
    suite.TARGET = repo
    suite.TARGET_MEMBER_DIR = member
    suite.TARGET_MEMBER_PACKAGE = crate
    suite.EXPLICIT_CONTRACT_TEST = case == "rustls"
    suite.INDEPENDENT_ORACLE = suite.INDEPENDENT_ORACLE.replace(
        "use anyhow::ported_totient;", f"use {crate.replace('-', '_')}::ported_totient;"
    )
    suite.main()

if __name__ == "__main__":
    main()
