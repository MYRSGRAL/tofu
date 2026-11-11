#!/usr/bin/env bash
# Последовательные live-прогоны; таймаут ограничивает каждый процессный сценарий.
set -euo pipefail
umask 077

if [[ $# -ne 2 || $1 != /* || $2 != /* ]]; then
    echo 'Использование: bash bench/ui/run-comparison.sh /каталог/артефактов /новый/каталог/результатов' >&2
    exit 2
fi
artifact_dir=$1
result_dir=$2
repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)

# Проверка до создания результатов и до первого окна; никакой разблокировки.
unset QT_QPA_PLATFORM QT_QUICK_BACKEND GDK_BACKEND SLINT_BACKEND SLINT_RENDERER
state=$(gdbus call --session --dest org.freedesktop.ScreenSaver \
    --object-path /ScreenSaver --method org.freedesktop.ScreenSaver.GetActive)
if [[ $state != '(false,)' ]]; then
    echo "KDE заблокирован или состояние неизвестно: $state" >&2
    exit 1
fi
python - "$artifact_dir" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

root = Path(sys.argv[1])
manifest = json.loads((root / "manifest.json").read_text())
for toolkit in ("qt", "gtk", "slint"):
    name = f"yonik-bench-{toolkit}"
    expected = manifest["binary"][name]["sha256"]
    if hashlib.sha256((root / name).read_bytes()).hexdigest() != expected:
        raise SystemExit(f"SHA-256 не совпадает: {name}")
PY
mkdir -- "$result_dir"
for repeat in 1 2 3; do
    for rows in 1000 10000; do
        for toolkit in qt gtk slint; do
            timeout --signal=TERM --kill-after=10s 2100s \
                python "$repo_dir/tools/ui_benchmark.py" \
                --binary "$artifact_dir/yonik-bench-$toolkit" \
                --rows "$rows" --idle-seconds 900 --timeout 30 \
                --output "$result_dir/$toolkit-$rows-run$repeat"
        done
    done
done
