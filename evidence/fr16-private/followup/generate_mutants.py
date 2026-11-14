"""Создаёт изолированные source-mutant fixtures, не меняя канонический код."""

import argparse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
MODULE = ROOT / "crates/yonik-linux/src/worker/output_transport"


def render() -> dict[Path, str]:
    """Каждая fixture содержит исходную функцию с ровно одной source-заменой."""
    parent = MODULE.with_suffix(".rs").read_text().split("#[cfg(test)]\nmod tests")[0]
    codec = (MODULE / "compatibility.rs").read_text()
    parent = "\n".join(
        line for line in parent.splitlines() if not line.startswith("//!")
    )
    codec = "\n".join(line for line in codec.splitlines() if not line.startswith("//!"))
    codec = codec.replace("#[test]\nfn ", "pub(super) fn ")
    codec = codec.replace('include_bytes!("fixtures/', 'include_bytes!("')
    changes = {
        "pipefs": ("fstatfs(fd)?.f_type as u64 != 0x5049_5045", "false"),
        "roles": ("bytes[96..100] != [0, 2, 1, 2]", "false"),
        "resources": (
            "legacy.extend_from_slice(&bytes[112..]);",
            "legacy.truncate(80); legacy[7] = 1; legacy[10] = 0;",
        ),
        "lifetime": (".with_generation(generation)", ".with_generation(2)"),
    }
    baseline = parent + "\nmod compatibility {\n" + codec + "\n}\n"
    result = {MODULE / "fixtures/baseline.rs.fixture": baseline}
    for name, (before, after) in changes.items():
        assert baseline.count(before) == 1, name
        result[MODULE / f"fixtures/mutant_{name}.rs.fixture"] = baseline.replace(
            before, after
        )
    return result


def main() -> None:
    """Режим check сверяет fixtures с актуальным исходником перед сборкой."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    for path, source in render().items():
        if args.check:
            assert path.read_text() == source, path
        else:
            path.write_text(source)


if __name__ == "__main__":
    main()
