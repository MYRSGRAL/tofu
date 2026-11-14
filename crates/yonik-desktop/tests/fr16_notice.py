"""FR16: сообщение об отбрасывании не занимает панель до явного чтения."""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))
import desktop_ui_smoke as smoke


def main() -> None:
    binary = Path(sys.argv[1]).resolve(strict=True)
    output = Path(sys.argv[2]).resolve()
    original = smoke.script

    def script(url: str, width: int, height: int) -> list[dict[str, object]]:
        steps = original(url, width, height)
        # Тот же wheel, порядок, atEnd/assertions и deadline, плюс проверка причины.
        step = steps[77]
        assert step["ui"] == {
            "action": "wheel",
            "object": "diagnosticsScroll",
            "delta": -1200,
        }
        step["ui_when"] = {"logsWithheldNotice": {"visible": False}}
        return steps

    smoke.script = script
    results = []
    for width, height, scale in [(420, 760, "1"), (380, 600, "1.5")]:
        results.append(
            smoke.run(
                binary, output / f"{width}-{height}-{scale}", width, height, scale
            )
        )
    output.mkdir(parents=True, exist_ok=True)
    (output / "result.json").write_text(
        json.dumps(results, ensure_ascii=False, indent=2) + "\n"
    )


if __name__ == "__main__":
    main()
