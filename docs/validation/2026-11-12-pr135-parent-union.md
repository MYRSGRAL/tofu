# PR135: интеграция parent134

Обычный merge точного parent `69142b44d522b57e5bf5f05a2b991912a5a54641`
в исходный PR135 `7ff75c5405ac669f1bd56fa9ac5e657bc486e154`.
Конфликтов нет. ManualEntry и прежние manual fixtures побайтово сохранены.
Main передаёт submitted в существующий submitLocalImport; ресурс формы и
PresetPanel зарегистрированы вместе. Backend/editor/maintenance совпадают с parent.
Pins, capabilities, ABI и контракты не изменены.

Проверки интеграции:
- MSRV 1.85.1 desktop lib/tests: 335 PASS, 15 ignored; P10 harness GREEN.
- GUI clippy all-targets --locked -- -D warnings: PASS; GUI build: PASS.
- fmt --all --check: PASS.
- Ruff 0.17.4 check и format --check tools и двух manual Python fixtures: PASS.
- RU/EN: 372 пары, 104 ключа; 18 тестов переводов PASS.
- QtTest manual: 8 PASS.
- Actual Qt: VLESS + TLS/WS через Main → review → verified backup → approve:
  PASS; redaction trace/log PASS. Адаптер сокращает прежний harness до одного
  случая; полные неизменённые 12+24 матрицы не повторялись.

Расширенный Ruff всего desktop/tests выявил прежние нарушения в сторонних
fixtures; они вне scope и не исправлялись. C++ Qt выдаёт предупреждения, Rust
clippy завершился успешно. P07 reopen в ephemeral GUI не подтверждён;
существующее принятое barrier proof сохраняется. Android и физические
G-UI/G-POWER отложены. Независимый review выполняет root.

Evidence, бинарник, source/binary hashes, parents, merge diff и полные логи:
`/home/myrs/projects/yonik/local/validation/pr135-union/`.
Собственный transient target очищен через cargo clean (7.2 GiB), root удалён.
