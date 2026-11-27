"""Functional pinned upstream Git capability added to a binary-only target.

All original hyperfine binaries and test targets must still pass. Composer
adds a new public library facade and the independent contract in disposable
copies only; the original Git checkout remains untouched.
"""
import test_heldout30_functional_anyhow as suite

def main():
    suite.TARGET = "sharkdp/hyperfine"
    suite.ALLOW_BINARY_LIBRARY_ADAPTER = True
    suite.INDEPENDENT_ORACLE = suite.INDEPENDENT_ORACLE.replace(
        "use anyhow::ported_totient;", "use hyperfine::ported_totient;"
    )
    suite.main()

if __name__ == "__main__":
    main()
