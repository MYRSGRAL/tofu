# Startup observer после объединения с maintenance/main

Root обычным merge объединил PR115 `33efb731` с PR107 `adf0b71`.
HostConfig сохраняет startup_observer и cfg(test-support) cleanup_fault;
оба доверенных Rust builder сохранены, GUI/env authority не добавлена.
Features startup-diagnostics и test-support объединены; self dev-dependency
включает observer в исходном integration harness без CLI features.
В tests/backend сохранены diagnostic timeout regression и LogsSave tests.

Rust 1.85.1 --locked: 48 unit, 44 backend (1 прежний ignored),
2 backend_startup, 4 dispatch и 3 maintenance contracts — PASS.
Pinned Clippy all-targets/test-support, fmt и RU/EN PASS.
Source-before-115.json записан до проверок, после них совпали 1379 hashes.
Retained lib/backend/startup binaries и логи:
local/validation/pr113-pr115-integration/manifest115.json.

Повторного GUI/native/VM запуска не было. Никаких изменений timeout,
параллелизма или ослабления UnsafePath. Причина исходного CI hang остаётся
UNKNOWN; observer обеспечивает диагностику следующего воспроизведения.
