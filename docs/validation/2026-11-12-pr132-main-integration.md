# PR132: объединение с принятым main

Ordinary merge родителей 9324cb1b0b410787699f6549b533874789e23358 и
37638032f96c1730edefbb185e9380e370c458ed. Сохранены P17/P07, typed P11 contexts,
fences и maintenance из main; lower-only capacity hook и вынесенный production
bridge из PR132. Конфликты разрешены объединением тестовых модулей backend и
сохранением mod bridge_impl в main.rs. Новых контрактов, pins и ABI нет.

Rust 1.85.1, locked, jobs=1, общий slot-wrapper: desktop test-support suite PASS
(library 89 passed/1 ignored, backend 46 passed/1 ignored, остальные targets и P10
в полном логе), GUI/test-support all-targets clippy -D warnings PASS, GUI example
build PASS, fmt --all --check PASS. Pinned Ruff 0.17.4 check/format --check tools
PASS; strict V2 5/5, factory coverage 2/2 PASS.

Первый GUI all-targets cargo test FAILED при линковке p05_native (CXX bridge
symbols отсутствуют в integration target); не скрыт и не объявлен PASS.

Свежий actual Full RU контроль через оригинальный qrc Main.qml и штатную кнопку
экспорта PASS: strict V2, language lifecycle, closed snapshot, terminal Qt exit.
Это offscreen проверка обработчика, без физического mouse/compositor gate.
Четыре исходные Qt кампании не повторялись: examples/bridge/harness сохранены;
Main.qml содержит принятый P17, поэтому выполнен свежий Full контроль.
Android, физические G-UI/G-POWER и полный P11 не заявляются.

Evidence и бинарник вне TTL: /home/myrs/projects/yonik/local/validation/pr132-main-37638032/.
parents.json, source.patch, source-manifest.json, binary.sha256, полные логи
проверок и full-ru/ сохраняют привязку исходников и runtime. cleanup-refs.json
фиксирует отсутствие внешних ссылок перед штатным cargo clean.
