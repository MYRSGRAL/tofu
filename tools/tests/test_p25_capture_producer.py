"""Чистые проверки bounded producer без compiler/Qt execution."""

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import _p25_capture_producer as p


def test_missing_is_retained(tmp_path: Path) -> None:
    task = tmp_path / "task"
    task.mkdir()
    source, build = task / "source", task / "build"
    retained = tmp_path / "retained"
    for root in (source, build, retained):
        root.mkdir()
    (build / "release").mkdir()
    (build / "release/elf").write_bytes(b"fixture")
    frozen = {
        "source": {"commit": "a" * 40, "tree": "b" * 40},
        "archive_sha256": "c" * 64,
    }
    commands = [{"argv": ["slot", "cargo", "build"], "cwd": str(source), "exit": 0}]
    result = p.retain(source, build, retained, frozen, commands, ["elf"], "target")
    assert result["status"] == "MISSING"
    assert (retained / "build-capture-manifest.json").is_file()
    assert result["missing"]
    assert result["final_linux_acceptance"] is False


def test_failed_command_cannot_be_observed(tmp_path: Path) -> None:
    with pytest.raises(ValueError):
        p.retain(tmp_path, tmp_path, tmp_path, {}, [{"exit": 1}], [], "target")


def test_output_symlink_ancestor_rejected(tmp_path: Path) -> None:
    actual = tmp_path / "actual"
    actual.mkdir()
    alias = tmp_path / "alias"
    alias.symlink_to(actual, target_is_directory=True)
    with pytest.raises(OSError):
        p._write(alias / "record.json", {"schema": 1})
    assert list(actual.iterdir()) == []


def test_output_cap_before_creation(tmp_path: Path) -> None:
    with pytest.raises(ValueError):
        p._write(tmp_path / "record.json", {"data": "x" * p.MAX_METADATA})
    assert not (tmp_path / "record.json").exists()
