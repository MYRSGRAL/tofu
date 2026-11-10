# P10: owner fencing и backoff provider

Review базы da0e1ad750a360bc68f4cb6ea82a6cf89bf9ae91 выявил два P2:
чужой PropertiesChanged инвалидировал снимок, а успешный connect сбрасывал
backoff до восстановления подписки/снимка.

Сигнал теперь проверяется по type, sender, path, header interface и interface
в типизированном payload. Для PropertiesChanged sender должен совпадать с
уникальным владельцем соответствующего GetAll. Владельцы фиксируются до
GetAll каждого сервиса; subscribe-before-snapshot и отмена снимка при настоящем
owner/property событии сохранены. NameOwnerChanged принимается только от
шины по полному контракту сигнала. Неизвестные отправители не запускают GetAll.

Backoff растёт 1, 2, 4, 8, 16, 30 секунд и сбрасывается только после успешного
снимка и не менее 30 секунд работы provider; connect сам по себе не сбрасывает
задержку. Ошибка очередного снимка отменяет признак здоровой работы.

RED sender-регрессии: чужой sender ошибочно принят relevant. GREEN проверяет
также неверные path/header interface/payload interface и положительный контроль.
Private bus отправляет поддельный сигнал с отдельного соединения и проверяет,
что watch policy не изменяется даже временно. Прежняя startup race с задержанным
GetAll, настоящие metered/owner/power события и shutdown остаются в тесте.

Backoff-регрессия началась с compile RED отсутствующего helper; GREEN проверяет
рост без успешного восстановления, предел 30 секунд и границу сброса.
Полный daemon reconnect и DesktopHost/private-bus end-to-end этой проверкой
не подтверждены; исходные ограничения P10 остаются.

Логи вне git: /home/myrs/projects/yonik/local/validation/pr58-review/.
Свой Cargo target находится в transient yonik-policy-review под flock корня,
Rust 1.85.1, jobs=2. Зависимости не менялись.

Финальные проверки: cargo test -p yonik-linux -p yonik-desktop —
414 passed, 0 failed, 24 ignored; clippy этих crates с --all-targets
-- -D warnings, cargo fmt --all -- --check и git diff --check прошли.
Cargo.lock восстановлен после сверки семантического равенства; изменён был
только порядок package/dependencies. Qt/QML не затрагивались.
