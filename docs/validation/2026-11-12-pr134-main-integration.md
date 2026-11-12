# PR134: объединение локального импорта с принятым main

Обычный merge main 37638032f96c1730edefbb185e9380e370c458ed в
2f21b25584a7246b23e9258a46b15a431b990ad4. Единственный конфликт backend
разрешён объединением запретов maintenance при local_task и preset_job.
Сохранены FR07 text/file, P17 presets, P07 и P11; новые контракты не вводились.
SourceDataV2, format4, repository API, submitLocalImport, Bridge commands,
error catalog, maintenance ACK, pins и capabilities вручную не изменялись.
PR135/136/137/138 не включались.

Проверки Linux, Rust 1.85.1, locked, jobs=1, with-rust-slot:

- GUI/test-support Clippy all-targets, -D warnings: PASS.
- Backend без GUI: library 91 PASS/1 ignored, backend 54 PASS/2 ignored,
  editor 13 PASS, maintenance 3 PASS, preset_product 2 PASS.
- fmt --all --check и GUI build: PASS.
- Ruff 0.17.4 check tools и format --check tools: PASS.
- RU/EN: 340 inline-пар, 104 ключа, 13 QML; 18 тестов переводов: PASS.
- P11 factory coverage: 2 PASS.
- Actual Qt: local_import_smoke, preset_smoke, unavailable preset controller: PASS.
- P07 actual single-process: V1→V2, disabled/enabled duplicate, simple/advanced
  preservation, Explain и unsupported activation refusal: PASS.

Ограничения: прежний P07 многозапусковый harness передаёт удалённый
--editor-persistence; текущий smoke-test генерирует новый ephemeral key при
каждом запуске. Qt reopen этим прогоном не подтверждён. Адаптер вне репозитория
использует те же UI steps в одном процессе и актуальный DTO ru_direct:null.
Попытка cargo test с gui упёрлась в undefined DesktopBridge symbols при линковке
integration test; штатные backend tests без gui и отдельный GUI binary прошли.
Полные неизменённые Qt campaigns не повторялись. Android и физические G-UI/G-POWER
отложены.

Evidence, адаптер, исходные hashes, binary SHA-256, бинарник и все логи вне TTL:
/home/myrs/projects/yonik/local/validation/pr134-main-37638032/.
