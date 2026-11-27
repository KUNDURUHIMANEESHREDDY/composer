"""Independent, reproducible Rust contract vectors using upstream Hypothesis.

The untrusted Rust implementation is NEVER imported or executed by Python.
Hypothesis generates cases; standalone mathematical definitions supply expected
values; Composer's offline Docker verifier executes them against Rust.
This is bounded differential testing, not proof of universal equivalence.
"""
from __future__ import annotations

from math import gcd
from hypothesis import given, settings, strategies as st


def _inputs(lower: int, upper: int) -> list[int]:
    seen: set[int] = set()

    @settings(max_examples=128, derandomize=True, deadline=None, database=None)
    @given(st.integers(min_value=lower, max_value=upper))
    def collect(n: int) -> None:
        seen.add(n)

    collect()
    assert len(seen) >= 100, f"insufficient independent examples: {len(seen)}"
    return sorted(seen)


def independent_contract(kind: str, target_function: str) -> tuple[str, int]:
    if kind == "totient":
        values = set(_inputs(1, 2048))
        values.update([1, 2, 3, 4, 8, 9, 10, 13, 27, 100, 1000, 2048])
        # Independent definition of Euler's totient, not the source's
        # optimized prime-factor algorithm.
        tuples = [(n, sum(gcd(n, k) == 1 for k in range(1, n + 1)))
                  for n in sorted(values)]
        rust_type = "u32"
        ret_type = "u128"
    elif kind == "cube":
        values = set(_inputs(-30000, 30000))
        values.update([-27000, -1000, -64, -27, -8, -1, 0, 1, 2, 8, 27, 64, 999, 1000, 1001, 27000])
        cubes = {root ** 3 for root in range(-32, 33)}
        tuples = [(n, n in cubes) for n in sorted(values)]
        rust_type = "i32"
        ret_type = "bool"
    else:
        raise ValueError(f"unrecognized independent oracle kind: {kind}")

    if kind == "totient":
        rows = "\n".join(f"        ({n}_{rust_type}, {answer}_{ret_type})," for n, answer in tuples)
    else:
        rows = "\n".join(f"        ({n}_{rust_type}, {str(answer).lower()})," for n, answer in tuples)
    rust = (
        "\n#[test]\n"
        f"fn hypothesis_independent_{kind}_oracle() {{\n"
        f"    let vectors: &[({rust_type}, {ret_type})] = &[\n{rows}\n    ];\n"
        "    for &(input, expected) in vectors {\n"
        f"        assert_eq!({target_function}(input), expected, "
        '"unexpected result for input={input}");\n'
        "    }\n"
        "}\n"
    )
    return rust, len(tuples)
