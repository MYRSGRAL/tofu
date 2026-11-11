# P07/P20 L15: диагностика черновика

База: `d0a281b4c23b45f65db1ddb74c6b14378b399794`.

Desktop преобразует сохранённый Draft в существующую domain Policy. ID,
порядок, disabled и default сохраняются. Диагностика вычисляется после успешной
CAS-записи и при загрузке; ошибка записи сохраняет предыдущий отчёт и форму.
Report относится к routing_revision того же снимка. Shadowed означает включение
условий в старшее правило; PotentialOverlap не доказывает существование потока.
При усечении или ошибке входа UI показывает неполноту. Нет полного доказательства
совместимости по отсутствию предупреждений.

`routing_explain` принимает expected_revision, request_id и bounded target
(до 253 байт). Используется evaluate_destination без I/O. Результат содержит
только источник/действие или Unknown; пути приложений, домены и IP обратно не
публикуются. Для имён без DNS witness и правил приложения без атрибуции результат
Unknown. Literal IP даёт синтетический предварительный расчёт, не наблюдение
потока. VPN/LAN не настроены в Draft и не добавляются скрыто. Domain evaluator
сохраняет собственный приоритет private/LAN/user/default; этот срез его не меняет.

ACK сопоставляется с request_id; объяснение дополнительно привязано к revision
и request_id в QML. Мутация очищает объяснение. Устаревшая команда получает
routing_stale и не меняет политику. Existing отмена routing-команд включает
routing_explain, поэтому Stop не оставляет UI pending до restart.

SystemSession, mode/guard и system_tun=false не меняются. Android отложен.
Это editor-срез, не P20 actual Apply и не закрытие A-gates.

Evidence и бинарник: `/home/myrs/projects/yonik/local/evidence/editor-l15`.
Manifest фиксирует исходники, lock, toolchain, target и jobs=1. Сборки идут через
with-rust-slot и flock корня собственного transient-каталога. Логи содержат
результаты scoped backend, DTO и actual Qt проверок. Первичный diagnostics RED
невалиден: компиляция зависимостей завершилась уже после изменения исходников.
Отдельный meaningful RED в green.log фиксирует false-known DNS fixture; исправление
требует Unknown без DNS witness.

Результаты: backend routing_regressions 5/5; editor DTO 7/7; actual Qt
qt-final 40/40, exit 0, включая точную конечную Policy и пустой полный report
после выключения #2. При немедленном клике после reorder smoke находил delegate
до завершения обновления bindings; добавлен отдельный UI-шаг и проверка selectedId=2
перед вводом. Предыдущие неудачные попытки сохранены в evidence.

`cargo fmt --all -- --check` и `git diff --check` проходят.
Полный `cargo clippy -p yonik-desktop --all-targets --features gui -- -D warnings`
останавливается на существующем collapsible_match в yonik-engine/src/lib.rs:166
с Rust 1.97. Engine не изменён. Ruff отсутствует; установка не выполнялась.
Отдельные RED для shadow/partial/incomplete/stale не подтверждены; GREEN для
этих сценариев есть, meaningful DNS Unknown RED/GREEN сохранён. Это ограничение
истории проверки, не утверждение о полном выполнении запрошенного RED-пакета.

Scoped clippy с `--no-deps --all-targets --features gui -- -D warnings` прошёл
(clippy-scoped.log); присутствуют предупреждения C++ toolchain, Rust lint errors нет.
Повторный actual Qt после фиксации manifest: qt-verified, 40/40, exit 0.
