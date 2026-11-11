# Интеграция PR99 без физических запусков

Обычные merge PR108 f9a58fbdaf8bede810907dad9759bace0610b65f и main
97daece04e963086d870ce215ff812758e50911e: 98c8d01 и 255a5d8.
Source HEAD и SHA256 исходников, lockfiles и внешних evidence — в manifest.json.
Fix f974f49 восстанавливает импорт resource_counters из offscreen CLI.
Production Rust, pins, ABI и capabilities не редактировались.

Свежая приёмка: 26 Python harness/offscreen/controls PASS, 7 counter tests PASS.
Invalid page и unmatched 1000/10000 datasets отклоняются oracle.
Fake-child dryproof: valid=true, 10 actions, normal exit; session/counter sources
подменены тестом. Это синтетический протокол, не физическая приёмка.
Старые runtime/availability campaigns не повторялись и не приписываются новому main.

Desktop fmt/clippy 1.85.1 -p yonik-desktop --all-targets --locked -- -D warnings PASS.
Model fmt/clippy --all-targets/test на 1.97.0 PASS (8 tests); существующий MSRV
модели 1.92 и pins сохранены. Ruff check/format scoped PASS, bash -n PASS.
Cargo через with-rust-slot, jobs=1, собственный transient target под flock.
После terminal cargo clean, проверка отсутствия внешних ссылок и удаление своего root.
Полные логи и no-physical-dryproof.json вне TTL:
/home/myrs/projects/yonik/local/validation/pr99-integration-20261112/.

Ограничения: первая suite выявила исправленный offscreen import. Следующая
поймала существующую гонку fixture в test_group_cleanup_kills_term_ignoring_descendant_after_leader_exit:
чтение созданного, ещё пустого PID-файла (ValueError). Следующий полный проход
26 tests PASS; fixture не менялся. Python 3.14 также выдаёт прежние ResourceWarning
внешнего reap. Android, G-UI/G-POWER и физические окна отложены. Review/merge — root.
