# Системный IPC-клиент

База: `2be9a1bc8fcd553594e36e8a4c91614c15ba6251`.

`yonik_linux::system_client::SystemClient` использует system D-Bus либо явно
переданную Connection для embedding/приёмки. Адресат, object path и interface
фиксированы. Методы Capabilities, State, Prepare публичной защиты, Apply, Stop,
Unlock используют закрытые DTO v1 и JSON до 64 KiB. Неизвестные поля, версии,
типы, состояния и error codes отклоняются. UID, argv, policy и paths API не принимает.

Каждый вызов получает абсолютный `tokio::time::Instant` и `Cancellation`.
Локальная Deadline отличается от серверной Timeout; authorization отображается
в Denied. Ошибки не содержат wire body. Prepared не клонируется, скрывает Debug
и потребляется Apply; он связан с unique owner и connection peer.

Клиент подписывается на NameOwnerChanged до разрешения имени, отправляет RPC
только закреплённому unique owner и сверяет владельца после ответа. Смена имени,
включая release/reacquire тем же процессом, либо потеря потока необратимо
закрывает экземпляр. Восстановление требует явного создания нового клиента.
Drop останавливает монитор. Методы требуют `&mut self`, сериализуя RPC клиента.

Ни один RPC не повторяется автоматически. Отмена, deadline или потеря owner
после отправки мутации не доказывает отсутствие эффекта на сервере. Controller
должен сверить State (после OwnerLost — новым клиентом); повторять Apply нельзя.
VerifiedRules не означает Connected. Клиент не включает system_tun и не меняет
transport capability, backend или GUI.

Приёмка использует настоящий `dbus-daemon --session --nofork` с ephemeral address
и настоящий zbus object server. Проверяются шесть RPC, закрытый JSON, неверная
D-Bus сигнатура, busy/timeout/authorization, отмена до отправки и во время Stop,
единственная отправка мутации при deadline/отмене, owner loss во время Stop,
release/reacquire и явное переподключение. Сервер теста проверяет IPC, но не
подменяет доказательство runtime работы privileged helper.

Остаток: full system bus/polkit VM gate, service activation/installation,
production helper effects и session controller/GUI. На пользовательской машине
сервисы, keyring и сеть не изменялись. Зависимости не добавлены; Android отложен.

Содержательный RED: `malformed_helper_error_is_protocol_error` на настоящей
шине получил `Err(Unavailable)` вместо `Err(Protocol)` для helper error с телом
`u32`. Исправление отделяет неверный контракт известной ошибки helper от
транспортной недоступности. Повторная проверка включена в финальный прогон.

Финальные результаты (Rust 1.85.1, Cargo jobs=1):

- `cargo test -p yonik-linux --test system_client`: 3 passed, 0 failed.
- `cargo clippy -p yonik-linux --all-targets -- -D warnings`: exit 0.
- `cargo fmt --all -- --check` и `git diff --check`: exit 0.

GREEN:

```text
running 3 tests
test malformed_helper_error_is_protocol_error ... ok
test wrong_dbus_type_is_protocol_error ... ok
test real_bus_contract_errors_cancellation_and_owner_fence ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.69s

```
