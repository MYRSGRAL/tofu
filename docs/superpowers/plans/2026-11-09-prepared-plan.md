# Проверка актуальности PreparedPlan

Основание: data-and-contracts.md §4, architecture.md §7.

1. Ввести EnvironmentStamp (поколения сети, application mappings, прав и engine)
   и ConfigDigest фиксированного размера. Digest вычисляет доверенный host по
   проверенному неизменяемому конфигу; reducer не читает файлы и не хэширует данные.
2. `Session::prepare` после config-check выдаёт непрозрачный PreparedPlan и
   инвалидирует старый preparation token. План нельзя произвольно сконструировать,
   клонировать или повторно применить штатным безопасным API.
3. `Session::apply` потребляет план, сверяет свежий snapshot и digest. Только
   успешная сверка переводит Preparing → Starting. Raw Event::Validated удаляется,
   чтобы в публичном API не было обхода guard.
4. Устаревший план/утрата прав до начала Apply переводят сессию в Error без
   сетевых ресурсов; caller начинает новую подготовку. План остановленной или
   чужой операции игнорируется, не меняя новую сессию.
5. Проверить все компоненты stamp, digest mismatch, отмену, чужую сессию и
   невозможность обхода/повторного использования. Обновить прежние lifecycle-тесты,
   пройти fmt/clippy/tests/MSRV/release/Android compile-check, review и PR.

Host обязан получить snapshot непосредственно перед эффектами и применить тот же
проверенный immutable artifact. Этот guard не решает сам по себе TOCTOU с ОС,
авторизацию IPC, реальные привилегии или загрузку/компиляцию конфигурации.
