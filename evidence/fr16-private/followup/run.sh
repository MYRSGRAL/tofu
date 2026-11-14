#!/bin/bash
set -euo pipefail
export CARGO_TARGET_DIR=/var/tmp/codex-myrs/transient/fr16-followup-821c/target
export CARGO_BUILD_JOBS=1
unset RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER
out=evidence/fr16-private/followup
python "$out/generate_mutants.py" --check
find crates -type f -print0 | sort -z | xargs -0 sha256sum > "$out/source-before.sha256"
sha256sum Cargo.toml Cargo.lock tests/fixtures/native-broker-v1.json tests/isolatedfixture/nativehooks/frozen-vectors.json docs/specs/native-broker-bootstrap-v1.md >> "$out/source-before.sha256"
cargo +1.85.1 test --locked -p yonik-linux --lib --no-run --message-format=json > "$out/build.jsonl" 2> "$out/build.log"
python - <<'PY'
import json
from pathlib import Path
out=Path('evidence/fr16-private/followup')
rows=[json.loads(line) for line in (out/'build.jsonl').read_text().splitlines()]
elves=[r['executable'] for r in rows if r.get('reason')=='compiler-artifact' and r.get('executable') and r['target']['name']=='yonik_linux']
assert len(elves)==1, elves
(out/'elf-path.txt').write_text(elves[0])
PY
elf=$(cat "$out/elf-path.txt")
cp "$elf" evidence/fr16-private/artifacts/followup-tests
# Только private output_transport; ни old3, ни новые kernel queries.
"$elf" worker::output_transport:: --nocapture > "$out/tests.log" 2>&1
cargo +1.85.1 check --locked -p yonik-linux > "$out/check.log" 2>&1
cargo +1.85.1 clippy --locked -p yonik-linux --all-targets -- -D warnings > "$out/clippy.log" 2>&1
sha256sum -c "$out/source-before.sha256" > "$out/source-after.log"
cargo +1.85.1 fmt --all -- --check > "$out/fmt.log" 2>&1
cargo +1.85.1 clean > "$out/clean.log" 2>&1
