# Maintenance: объединение с scheduler, журналом и notices

Родители: maintenance `7319ec06c7cb78d7b435e7282fe45ea44b2ab7db` и main
`d5858a37636f1ba6be5f570df0c71bf668b84a33`.

Разрешены конфликты команд/Actor slots и QML Settings: сохранены обе панели,
Notices, LogsSave/LogsTest, optional service/routing handles и matching ACK.
Обнаруженные при объединении несовместимости проверены RED→GREEN:

- Off-actor maintenance возвращает scheduler через `replacement()`, сохраняя
  интервал, фоновые настройки и временные барьеры. Загрузка после reopen получает
  актуальное monotonic время с учётом длительности копирования.
- Nested supervisor отвечает matching ACK на logs/routing команды; старый export
  в отказном logs ACK очищается. Maintenance не начинается при активном SaveJob,
  чтобы его завершение оставалось под обычным shutdown drain.

Проверки Rust 1.85.1 `--locked`, jobs=1, общий rust-slot:
48 library tests, 42 backend tests (1 ignored), 3 maintenance contracts — PASS.
GUI build и all-target pinned Clippy driver с `gui,test-support` — PASS.
RU/EN checker: 12 QML, 254 inline-пары, 96 ключей — PASS. Diff/fmt — PASS.

Evidence, исходные хеши и сохранённые executable находятся в
`/home/myrs/projects/yonik/local/validation/pr107-main/`.
Хеши записаны после проверок; это согласованность сохранённого evidence,
не независимая воспроизводимая аттестация сборки. GUI бинарник построен, но новый
полный GUI runtime/OS portal/keyring gate не запускался. Старое component Qt
proof остаётся историческим. Полная P06 не заявляется.
