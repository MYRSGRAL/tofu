# P11 Corrupt/Full: штатная кнопка production GUI

`tools/tests/check_scheduler_production_ui.py` загружает исходный qrc URL через
оригинальный QQmlApplicationEngine::load. Main.qml, errorText, positive snapshot
и context не подменяются. Отдельный невидимый TextEdit читает native clipboard;
драйвер наблюдает production window и находит реальные элементы по objectName.

Для каждого actual producer проверяется Settings → Diagnostics, visible/enabled
штатной copyDiagnosticsButton и её локализованный текст. Сигнал clicked вызывает
существующий production onClicked; результат обязан быть «Скопировано»/«Copied».
Полученный через TextEdit.paste экспорт проходит независимый strict V2 validator.
Это offscreen проверка обработчика кнопки, без доказательства физических mouse
hit testing и compositor presentation.

RU case затем открывает Appearance и вызывает настоящий languageChoice.activated,
который отправляет set_language. Изменение подтверждается backend snapshot и
сохранённым preferences.json. В текущей реализации этот lifecycle очищает scheduler
contexts. Их сохранение после смены языка НЕ заявляется. EN case запускает новый
actual producer; observer переносит сохранённый production preferences.json до
инициализации Main. Новый host заново получает Corrupt/Full, затем проверяется
штатная EN-кнопка. Каждый case требует closed snapshot и terminal Qt exit.

Corrupt использует private encrypted repository с повреждённым synthetic payload;
Full использует прежний approved lower-only capacity hook. Pins/capabilities/ABI,
production QML и Rust не изменены. Старые tampered JSON проверки остаются только
validator controls; они не доказывают actual backend stale.

Evidence вне TTL: `/home/myrs/projects/yonik/local/validation/p11-ui-132/`.
Итоговые cases: `final-corrupt-ru`, `final-corrupt-en`, `final-full-ru`,
`final-full-en`; каждый содержит qt.log, strict V2 export, actual/final/closed
snapshots. Source manifest, сохранённые binaries, SHA256, build/clippy/fmt/Ruff
и Python logs находятся рядом. Ранние каталоги являются диагностическими прогонами.

Полный P11, остальные причины, Android и физические G-UI/G-POWER не закрыты.

Проверки: четыре actual UI cases PASS; свежая GUI сборка обоих examples PASS;
Rust 1.85.1 GUI/test-support clippy --all-targets -- -D warnings PASS; fmt --check
PASS; pinned Ruff 0.17.4 check и format --check для 106 tools-файлов PASS;
strict V2 5/5 и factory coverage 2/2 PASS. Компилятор C++16 печатает Qt-header
SFINAE diagnostics, gates завершаются с exit 0. Перед cleanup external refs=[];
штатный cargo clean завершён, собственный transient root удалён.
