#!/usr/bin/bash
set -euo pipefail
export CARGO_TARGET_DIR=/var/tmp/codex-myrs/transient/fr16-carrier-209/target
export FR16_TASK_ROOT=/var/tmp/codex-myrs/transient/fr16-carrier-209
export CARGO_BUILD_JOBS=1
unset RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER
out=docs/proposals/fr16-worker-output-only/carrier-evidence
python - <<'PY'
from pathlib import Path
import hashlib,tarfile
paths=sorted([p for root in ['crates','tests'] for p in Path(root).rglob('*') if p.is_file()]+[Path('Cargo.toml'),Path('Cargo.lock')]+list(Path('.cargo').glob('*')))
paths += sorted(p for p in Path('docs/specs').rglob('*') if p.is_file())
Path('local/fr16-carrier-209/source.sha256').write_text(''.join(f'{hashlib.sha256(p.read_bytes()).hexdigest()}  {p}\n' for p in paths))
with tarfile.open('local/fr16-carrier-209/source.tar.gz','w:gz') as archive:
 for p in paths: archive.add(p,arcname=str(p))
PY
cp local/fr16-carrier-209/source.sha256 "$out/source.sha256"
cargo +1.85.1 test --locked --offline -p yonik-linux --lib --features test-support --no-run --message-format=json > "$out/build.jsonl" 2> "$out/build.log"
sha256sum -c local/fr16-carrier-209/source.sha256 > "$out/source-after.log"
python - <<'PY'
import json,shutil
from pathlib import Path
rows=[json.loads(line) for line in Path('docs/proposals/fr16-worker-output-only/carrier-evidence/build.jsonl').read_text().splitlines()]
elves=[r['executable'] for r in rows if r.get('reason')=='compiler-artifact' and r.get('executable') and r['target']['name']=='yonik_linux']
assert len(elves)==1,elves
shutil.copy2(elves[0],'local/fr16-carrier-209/artifacts/carrier-tests')
PY
elf=local/fr16-carrier-209/artifacts/carrier-tests
"$elf" output_transport::carrier_tests:: --nocapture > "$out/carrier-tests.log" 2>&1
"$elf" system_client::output::tests:: --nocapture > "$out/prepared-tests.log" 2>&1
"$elf" worker::output_transport::compatibility::codec_bounds_roles_scope_and_legacy_rejection --exact > "$out/codec-tests.log" 2>&1
"$elf" worker::output_transport::compatibility::resource_validation_reuses_v2 --exact >> "$out/codec-tests.log" 2>&1
"$elf" worker::output_transport::golden_compat:: > "$out/golden-tests.log" 2>&1
cargo +1.85.1 clippy --locked --offline -p yonik-linux --all-targets -- -D warnings > "$out/clippy-default.log" 2>&1
cargo +1.85.1 clippy --locked --offline -p yonik-linux --all-targets --features test-support -- -D warnings > "$out/clippy-feature.log" 2>&1
cargo +1.85.1 fmt --all -- --check > "$out/fmt.log" 2>&1
sha256sum -c local/fr16-carrier-209/source.sha256 > "$out/source-final.log"
