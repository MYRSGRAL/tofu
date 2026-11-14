#!/bin/bash
set -euo pipefail
export CARGO_TARGET_DIR=/var/tmp/codex-myrs/transient/fr16-followup-821c/target
export CARGO_BUILD_JOBS=1
unset RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER
out=evidence/fr16-private/followup
cargo +1.85.1 clippy --locked -p yonik-linux --all-targets -- -D warnings > "$out/clippy.log" 2>&1
evidence/fr16-private/artifacts/followup-tests worker::output_transport:: --test-threads=1 --nocapture > "$out/final-tests-retained-elf.log" 2>&1
cargo +1.85.1 fmt --all -- --check > "$out/fmt.log" 2>&1
python "$out/generate_mutants.py" --check
sha256sum -c "$out/final-source-before.sha256" > "$out/final-source-after.log"
