"""Проверить целостность docs-only delta без запуска продукта или probes."""

import hashlib
import json
import re
import subprocess
from pathlib import Path


def проверить_commit(root: Path, revision: str) -> None:
    assert re.fullmatch(r"[0-9a-f]{40}", revision), revision
    subprocess.run(
        ["git", "cat-file", "-e", f"{revision}^{{commit}}"],
        cwd=root,
        stderr=subprocess.DEVNULL,
        check=True,
    )


def является_предком(root: Path, before: str, after: str) -> bool:
    code = subprocess.run(
        ["git", "merge-base", "--is-ancestor", before, after],
        cwd=root,
        check=False,
    ).returncode
    assert code in (0, 1)
    return code == 0


def проверить() -> None:
    root = Path(__file__).resolve().parents[2]
    folder = root / "docs/validation"
    data = json.loads((folder / "2026-11-14-linux-criteria-302f8bf.json").read_text())
    for field in ("base", "accepted_main", "previous_delta_head"):
        проверить_commit(root, data[field])
    assert является_предком(root, data["base"], data["accepted_main"])
    assert является_предком(root, data["previous_delta_head"], data["accepted_main"])
    previous_parent = subprocess.check_output(
        ["git", "rev-parse", f"{data['previous_delta_head']}^"], cwd=root, text=True
    ).strip()
    assert previous_parent == data["base"]
    counts = data["plan_counts"]
    assert counts == {"denominator": 26, "ready": 3, "partial": 21, "deferred": 2}
    original = (
        (folder / "2026-11-11-linux-acceptance-index.md").read_text().splitlines()
    )
    expected = [
        (n, line)
        for n, line in enumerate(original, 1)
        if re.match(r"\| P\d\d \|", line)
        or re.match(r"\| \[(?:FR-|G-|D-|A-|T-|P-|L\d)", line)
    ]
    assert [(r["index_line"], r["original_row"]) for r in data["criteria"]] == expected
    assert len(expected) == 256
    assert all(not r["promotion"] for r in data["criteria"])
    for item in data["inputs"] + data["evidence"]:
        blob = subprocess.check_output(
            ["git", "rev-parse", f"{data['accepted_main']}:{item['path']}"],
            cwd=root,
            text=True,
        ).strip()
        assert blob == item["blob"], item["path"]
        if "sha256" in item:
            digest = hashlib.sha256((root / item["path"]).read_bytes()).hexdigest()
            assert digest == item["sha256"]
    for pr in data["pull_requests"]:
        проверить_commit(root, pr["head"])
        merge = pr["merge"]
        if merge is not None:
            проверить_commit(root, merge)
            assert является_предком(root, pr["head"], merge)
            merged = является_предком(root, merge, data["accepted_main"])
        else:
            merged = False
        assert pr["merge_in_accepted"] is merged
        assert (pr["state"] == "MERGED") is (merge is not None)
        revisions = [
            ("head_in_base", data["base"]),
            ("head_in_accepted", data["accepted_main"]),
        ]
        for field, revision in revisions:
            assert pr[field] is является_предком(root, pr["head"], revision)
        if pr["pr"] in (177, 185, 186, 189):
            assert pr["head_in_accepted"] and len(pr["checks"]) == 10
            assert all(c["conclusion"] == "SUCCESS" for c in pr["checks"])
        if pr["pr"] in (178, 181, 187, 188, 190):
            assert not pr["head_in_accepted"]
    for preserved in data["preserved_pending_refs"]:
        проверить_commit(root, preserved["sha"])
        assert preserved["in_accepted"] is является_предком(
            root, preserved["sha"], data["accepted_main"]
        )
    text = (folder / "2026-11-14-linux-delta-302f8bf.md").read_text()
    claims = [
        line
        for line in text.splitlines()
        if line.startswith("|") and not re.fullmatch(r"[\s|:-]+", line)
    ]
    assert data["delta_criteria"] == claims
    for target in re.findall(r"\]\(([^)]+)\)", text):
        if not target.startswith("https://"):
            assert (folder / target.split("#")[0]).is_file(), target


if __name__ == "__main__":
    проверить()
