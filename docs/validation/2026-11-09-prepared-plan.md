# PreparedPlan: актуальность перед Apply

## Реализовано

`tofu-application` связывает одноразовый PreparedPlan с request token,
поколениями сети, application mappings, прав и engine, а также 32-байтным
SHA-256 digest проверенной конфигурации. Поля плана закрыты; Clone/Copy нет.
Digest не раскрывается через Debug.

`Session::prepare` вызывается доверенным host после config-check, остаётся в
Preparing и меняет token. Повторная подготовка со старым token отклоняется.
Только `Session::apply` с совпавшим свежим snapshot переводит сессию в Starting.
Событие Event::Validated удалено из публичного API, старого обходного пути нет.
После Starting по-прежнему нужны NetworkApplied и CheckPassed для Connected.

Уход прав или несовпадение stamp/digest до Apply даёт Error без запуска ядра.
После изменения окружения нужна новая подготовка. План отменённой или чужой
операции игнорируется до проверки окружения: он не может перевести новую сессию
в Error. Исчерпание счётчиков не приводит к частичной мутации.

## Проверки

- Rust 1.97.0 и MSRV 1.85.1: tests и clippy всех targets без warnings.
- В workspace 22 теста: 3 unit, 7 PreparedPlan integration, 7 lifecycle и 5 policy.
- Четыре compile-fail doctest: запрет Clone/повторного использования плана,
  отсутствие Event::Validated и отсутствие пользовательского PrivateVpn action.
- MSRV fmt/release и Android `cargo check --workspace --target aarch64-linux-android --locked`.
- Четыре Python regression tests прежних engine-стендов проходят.

Проверены отдельно поколения сети, mappings, прав и engine; digest mismatch;
отсутствие/утрата авторизации; отмена и замена сессии; foreign controller ID;
границы prepare/apply и неприменённая applied-generation до подтверждения host.

## Границы

Это логика корреляции и актуальности, не самостоятельная security boundary.
Host обязан получать snapshot непосредственно перед эффектами, не переиспользовать
поколения после ухода/возврата сети или прав, вычислять digest реально проверенного
immutable artifact и применять именно этот artifact. Reducer не читает ОС,
не хэширует файлы, не выдаёт права и не делает атомарной транзакцию с сетью.

Реальные config-check adapters, host Apply/rollback, привилегированный helper,
IPC-авторизация и защита от TOCTOU ещё не подключены. Android проверен только
на уровне компиляции общих crates; G-ANDROID остаётся открытым. Планы и credentials
из пользовательской подписки в этих тестах не используются.
