# Изолированная проверка отказов Secret Service

База: `2be9a1bc8fcd553594e36e8a4c91614c15ba6251`.
Область: `yonik-linux::subscription_master_key`, без пользовательского keyring,
изменения default collection, установки сервисов или новых зависимостей.
Контракты: data-and-contracts §4/6, platforms §2, architecture §3.
Отдельного security.md в данной базе нет.

## Стенд

Интеграционный тест запускает отдельный `dbus-daemon --config-file <private-config> --nofork` для каждого
сценария. Конфигурация не содержит service directories: автоматическая активация
реального keyring запрещена. zbus предоставляет Secret.Service/Collection/Item. Дочерний процесс
того же тестового бинарника вызывает настоящий публичный production API;
`DBUS_SESSION_BUS_ADDRESS` и `XDG_RUNTIME_DIR` передаются явно только ему.
Окружение основного процесса не меняется. Так как secret-service 4.0.0 не имеет
конструктора с Connection, инъекция адреса выполнена на границе процесса,
без добавления обходного production API. В fixture принимается только штатный
DH algorithm; plain-сессии и отключение шифрования не используются.
Ответ DH fixture содержит публичное значение 2; положительное расшифрование
настоящего ключа этот стенд не доказывает.

Сценарии: отсутствующее имя сервиса; отсутствующий ключ существующей БД;
locked item; locked default collection; предложенный unlock prompt;
блокировка между поиском и GetSecret; ошибка GetSecret; malformed IV;
ошибка ciphertext/padding; неоднозначный результат поиска.
Проверяются режимы чтения и создания, кроме отсутствующего ключа новой БД,
где создание является нормальным поведением и не входит в проверку отказа.
Создание поверх отказов запрещено: учитываются CreateItem/CreateCollection,
SetAlias/Unlock; любой вызов проваливает тест. Для cancel проверено отсутствие
самого Unlock: приложение не показывает prompt и не должно запрашивать отмену.
Это не проверка доставки сигнала Completed(dismissed) от реального desktop.

## Ограничения

Это интеграционная проверка D-Bus с production API, не аппаратная приёмка,
не проверка login-session, KDE Wallet, перезагрузки или реального UI unlock.
Fixture не сохраняет секреты. Ошибки сравниваются по стабильным enum-вариантам,
сторонние тексты ошибок не возвращаются пользователю. Существующий API возвращает
Zeroizing; его Debug в закреплённой версии zeroize 1.9.1 скрывает значение.

## RED → GREEN

До исправления targeted test завершился кодом 101: оба режима `race` вернули
Unavailable вместо Locked; оба режима `malformed` завершили дочерний процесс
panic `generic-array: left 0, right 16`. Остальные сценарии прошли.
Исправления: точное сопоставление D-Bus IsLocked; локальный catch_unwind только
для чтения/дешифрования GetSecret. Production DH и политика создания сохранены.
Перехват рассчитан на штатный panic=unwind. При panic=abort он не защищает процесс.
Глобальный panic hook не заменяется: сообщение зависимости о длине IV может
попасть в stderr, но ключ и сторонний текст D-Bus туда адаптер не пишет.

## Итоговые проверки

Rust 1.85.1, `CARGO_BUILD_JOBS=1`, отдельный transient target под BSD flock
корневого task-каталога с начала использования.

- `cargo +1.85.1 fmt --all -- --check`: успешно.
- `cargo +1.85.1 test -p yonik-linux --lib --test credentials_bus --test subscriptions`:
  352 + 2 + 13 успешно; 24 платформенных теста ignored. Тест чтения настоящего
  Secret Service не запускался. Private fixture проверяет 19 комбинаций.
- `cargo +1.85.1 clippy -p yonik-linux --all-targets -- -D warnings`: успешно.

Сборка и clippy завершены до введения глобального slot-wrapper; последующие
тяжёлые команды обязаны использовать with-rust-slot. Изменений Cargo.lock нет.

## Исправления fixture после независимого review

Cipher-сценарий теперь передаёт корректный IV и ciphertext длиной 15 байт:
отказ детерминирован, случайный валидный PKCS#7 padding исключён.
Запуск daemon получает guard до чтения адреса. Чтение stdout асинхронное,
ограничено 4097 байтами и deadline 3 секунды; пустой/неверный адрес отклоняется.
При отмене/ошибке guard убивает и reaps только собственного child, удаляет
собственный runtime. Новый тест проверяет прерываемость чтения и cleanup.

Две регрессии RED (старый valid-block ciphertext и блокировка current-thread)
→ GREEN. credentials_bus: 4 top-level tests PASS (включая child harness и
19 private-bus комбинаций), scoped clippy и fmt PASS. Production credentials
в этом delta не менялись. Логи вне TTL:
/home/myrs/projects/yonik/local/validation/pr66-fixture-fix/.
Внутренний трёхсекундный timeout проверен по коду; тест отдельно проверяет
отмену внешним deadline 50 мс, не утверждает runtime-прогон истечения 3 секунд.
