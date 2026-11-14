"""Проверить целостность docs-only delta без запуска продукта или probes."""

import hashlib
import json
import re
import subprocess
from pathlib import Path


def проверить() -> None:
    root = Path(__file__).resolve().parents[2]
    folder = root / "docs/validation"
    data = json.loads((folder / "2026-11-14-linux-criteria-302f8bf.json").read_text())
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
        revisions = [
            ("head_in_base", data["base"]),
            ("head_in_accepted", data["accepted_main"]),
        ]
        for field, revision in revisions:
            code = subprocess.run(
                ["git", "merge-base", "--is-ancestor", pr["head"], revision],
                cwd=root,
                check=False,
            ).returncode
            assert code in (0, 1)
            assert pr[field] == (code == 0)
        if pr["pr"] in (177, 185, 186, 189):
            assert pr["head_in_accepted"] and len(pr["checks"]) == 10
            assert all(c["conclusion"] == "SUCCESS" for c in pr["checks"])
        if pr["pr"] in (178, 181, 187, 188, 190):
            assert not pr["head_in_accepted"]
    text = (folder / "2026-11-14-linux-delta-302f8bf.md").read_text()
    for target in re.findall(r"\]\(([^)]+)\)", text):
        if not target.startswith("https://"):
            assert (folder / target.split("#")[0]).is_file(), target


if __name__ == "__main__":
    проверить()
