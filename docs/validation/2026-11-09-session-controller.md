# Контур сессии и Android compile-check

## Реализация

Новый crate `tofu-application` содержит чистый reducer без I/O, runtime, GUI
и внешних зависимостей. Host сериализует команды и подтверждает результаты
сетевых операций. Реализован начальный цикл Preparing → Starting → Checking →
Connected/Limited и отмена/остановка с подтверждением cleanup.

Запуск ядра не переводит сессию сразу в Connected. Недоступность тестового
endpoint даёт Limited, сохраняя applied-generation. Ошибка после начала запуска
сначала требует Stopping и CleanupConfirmed; CleanupFailed сохраняет отдельное
состояние и не разрешает новое подключение. Причина исходного сбоя сохраняется
до очистки и следующей попытки.

Каждый принятый этап меняет request token; запоздалые и повторные события
игнорируются. Новый begin меняет configuration generation. Stop инвалидирует
предыдущий token; повторный Stop идемпотентен для управляющего состояния.
Операции при исчерпании счётчиков возвращают ошибку без частичной мутации.

Controller ID предоставляет host. Он должен быть уникален среди экземпляров,
callbacks которых ещё возможны, включая пересоздание controller. Token является
корреляцией, не средством авторизации и не криптографическим доказательством.

## Проверки

- Rust 1.97.0 и MSRV 1.85.1: тесты workspace и clippy по всем targets без warnings.
- В новом crate: 7 integration tests и 1 unit test границ счётчиков.
  В domain: прежние 5 integration tests и compile-fail doctest.
- Проверены неверные переходы, busy, поздние callbacks, разные controller ID,
  отмена Preparing, повторный Stop, сбой старта и cleanup/retry.
- На Rust 1.85.1 прошла release-сборка workspace.
- `cargo check --workspace --target aarch64-linux-android --locked` прошёл
  на Rust 1.85.1 для обоих crates. Проверка добавлена в workflow.

Toolchain и Android std target установлены только в собственный transient-каталог
с lock корня. Глобальные настройки Rust и сеть хоста не менялись.

## Ограничения

Android compile-check подтверждает компиляцию общего кода, но не Android host,
JNI/FFI, native engines, передачу TUN FD, protect, VpnService или батарею.
G-ANDROID остаётся открытым. Ни Linux helper, ни сетевой журнал ресурсов этим
reducer не реализованы. Host-события пока не подключены к реальным ядрам.

Recovery, reconfiguration/rollback, desired-profile, persistence, планировщик
и привязка PreparedPlan к network/application generation — следующие этапы.
Наличие enum State не является доказательством успешного сетевого подключения.
