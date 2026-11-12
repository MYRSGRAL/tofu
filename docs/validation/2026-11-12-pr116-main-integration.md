# PR116: интеграция актуального main

Ordinary merge main `1cd4a32db5b2c889039661ce0657eff0a3031842` в PR116
после fast-forward `a5cf6cc` → `d17d0ee837d1fa5497f5fafd1794d37a212e59fb`.
Рабочая копия перед интеграцией чистая. `d14b2be` — ancestor d17;
деревья d14 и d17 совпадают. PR120 был объединён в PR116, а не main.

Два конфликта backend разрешены сохранением обеих независимых веток ACK:
preset и SourceChange при unavailable controller и отмене stale envelope.
P17 CAS, busy/cancel, maintenance controller и catalog invalid_ip сохранены.
Main SourceChange cancellation/drain/reload ACK, V2 contexts, OFD lock,
helper wiring, examples и import tests включены ordinary merge.
PR113 и отдельный PR128 не включались; pins/capabilities/ABI не менялись.

Свежие логи, manifest исходников, toolchain и бинарник с SHA-256:
`/home/myrs/projects/yonik/local/validation/pr116-main-integration/`.
Все Cargo команды: with-rust-slot, jobs=1, Rust 1.85.1, один процесс задачи,
отдельный transient target с shared flock task-root на весь прогон.

Android и физические G-UI/G-POWER отложены. Прежние proxy/FakeDNS campaigns
не повторяются; mixed compiler Unsupported и system_tun=false не пересматриваются.

Приёмка:

- cargo test desktop/linux/import/engine --locked: PASS, штатные ignored fixtures.
- clippy 1.85.1 workspace all-targets и desktop gui,test-support all-targets,
  --locked, -D warnings: PASS.
- GUI build и cargo fmt --all --check: PASS.
- RU/EN: 321 inline-пара, 102 ключа, 13 QML; factory 1 PASS,
  diagnostic export Python 5 PASS.
- Настоящий Qt offscreen preset workflow и unavailable-controller: PASS;
  trace заканчивается closed=true. Внешние native campaigns не повторялись.

GUI clippy обнаружил увеличение HostConfig: возврат большого Err из внутренней
configure заменён на Err(()), поскольку caller использует только факт отказа.
Повторный GUI clippy прошёл. Первый failure log сохранён рядом с финальным.
Системный GCC16 выдаёт C++ предупреждения из Qt headers; Rust clippy чистый.
