# Linux: ограниченная генеративная проверка public parser

База: `37638032f96c1730edefbb185e9380e370c458ed`. Production parser, ABI,
pins, capabilities и GUI не изменялись. Проверяются `import_bytes` и
`ServerIntent::from_server`, используются существующие fixtures p04-seeded.

`crates/yonik-import/tests/robustness.rs` добавляет:

- соседние значения 19 999 / 20 000 / 20 001 записей, 63 / 64 / 65 уровней,
  8 MiB ± 1 байт и 64 KiB ± 1 байт URI;
- IPv4, IPv6, IDNA и крайние порты, отказ CIDR в endpoint;
- неизвестные вложенные native options: точное сохранение native и отказ Intent;
  escaped duplicate keys отклоняются;
- 12 288 мутаций JSON/URI/Base64 с seed `0x921420261112`:
  truncation, replacement, deletion, insertion, затем прямой и Base64 вход;
- сохранение original либо полного failure input, редактирование diagnostics,
  отрицательный контроль детектора synthetic credential, отсутствие роста FD.

Это deterministic mutation campaign, не coverage-guided fuzzing. Исходный
корпус — три существующих файла `tests/corpus/p04-seeded`. Все фактически
проверенные mutation inputs сохраняются runner-ом в `corpus.bin`: повторяющиеся
little-endian u32 length + bytes. Только синтетические credentials.

Linux runner `crates/yonik-import/tests/evidence/run_robustness.py` принимает
путь готового test binary и каталог evidence. Сборка выполняется отдельно через
`local/orchestration/with-rust-slot`, `CARGO_BUILD_JOBS=1`, MSRV 1.85.1;
`CARGO_TARGET_DIR` находится в task transient root под удерживаемым flock.
Runner задаёт 512 MiB RLIMIT_AS, 64 FD, CPU 60 секунд, wall timeout 90 секунд,
отключает core dumps. Измерение ru_maxrss относится ко всему test binary;
проверка FD сравнивает начало и конец кампании. Это ограниченная приёмка на данном
корпусе, а не доказательство ограниченности памяти для всех возможных входов.

Результат: все пять новых тестов прошли. Кампания без panic и зависания,
peak RSS 30 996 KiB. Полный `cargo +1.85.1 test --locked -p yonik-import`,
`clippy --locked -p yonik-import --all-targets -- -D warnings`, fmt --check,
pinned Ruff 0.17.4 check и format --check прошли.

Evidence вне TTL: `/home/myrs/projects/yonik/local/evidence/p04-robust-9214/`:
`summary.json`, `corpus.bin`, `campaign.log`, `tests.log`, `clippy.log`,
`robustness`, `sources.sha256`, копия теста и runner. Summary содержит SHA256
бинарника и корпуса. Negative control включён в campaign.log отдельным тестом.

Предел 32 MiB decoded недостижим через public API при ограничении encoded body
8 MiB: Base64 не расширяет decoded относительно encoded; отдельное достижение
этой границы не заявляется. Android и физические G-UI/G-POWER отложены.
