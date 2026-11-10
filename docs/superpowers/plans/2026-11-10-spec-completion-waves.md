# Завершение спецификации yonik по волнам

Основание: `docs/PLAN.md`, `docs/specs/requirements.md`,
`docs/specs/validation-and-roadmap.md` и соответствующие платформенные контракты.
Пользователь поручил завершить всю спецификацию, вести независимые задачи
параллельно в отдельных PR, проводить ревью, исправлять и сливать после проверок.
Конец волны не завершает задачу. Последовательность продолжает действующий план.

## Общие ограничения

- Существующие подписки, ключи и незакоммиченные изменения сохраняются.
- Linux: Rust, текущий нативный Qt GUI, системный helper; не менять toolkit.
- Никаких Docker build/up, deploy, release и установки helper в пользовательскую ОС.
- Сетевые эффекты проверяются в собственной namespace или одноразовой VM.
- Отдельный исполнитель владеет назначенными файлами; независимый reviewer
  проверяет весь диапазон base/head. Root интегрирует и сливает после CI.
- Cargo target вне checkout, в своём transient-каталоге под flock его корня,
  jobs=2. Evidence сохраняется вне TTL до штатной очистки target.
- Процент готовности увеличивается только при полной приёмке пункта Pxx.

## Первая волна — выполнено

- [x] #43: проверка переводов RU/EN и запуск в CI, `374ec75`.
- [x] #44: переносимый bounded scheduler, `3768988`.
- [x] #45: матрица импорта с padding Base64, `830af0d`.
- [x] #46: проверки восстановления хранения и безопасный crash fixture, `e33fa19`.

Все четыре PR в main, проверки зелёные. P04/P06/P10/P11 остаются частичными.

## Вторая волна — рабочие функции

### Task 1: Системный runtime — P09/P12/P13

Ownership: текущие изменения `helper/*`, `helper_main.rs`, `managed_network/*`,
`network/*`, `network.rs`, `yonik-application/src/plan.rs`, `tests/network_journal.rs`.
Продолжить существующий незакоммиченный SystemSessionHost в основной task-ветке.
Критерий: типизированный Prepare/Apply/Stop, общее владение journal и protection,
worker lifecycle, безопасные stale/cancel/restart и реальное изолированное
доказательство; не объявлять transport readiness по факту запуска процесса.
System GUI wiring следует после принятия IPC/runtime; DNS/LAN не подделывать.

Текущий runtime-срез реализует типизированный IPC, общий journal и worker-first
recovery; unit tests, clippy и три namespace gates проходят.
[Отчёт](../../validation/2026-11-10-system-session-runtime.md).
Installed session VM-матрица и TERM→KILL остаются непройденными: Task 1
не принят, `system_tun:false`, client/GUI ещё не подключены.

### Task 2: Исполнение scheduler — P10

Ownership: `yonik-linux/src/subscriptions.rs`, новый `subscription_scheduler.rs`,
объявление/экспорт в `yonik-linux/src/lib.rs`, целевые тесты.
Использовать `yonik_subscription::scheduler` и существующий SubscriptionService.
Критерий: bounded host исполняет due refresh, отменяет при смене источника/выходе,
подтверждает stale/cancel завершения, сохраняет review до commit и не включает
скрытый DIRECT. Время/окна/сеть/питание поступают событиями, без UI polling.

### Task 3: Постоянные настройки GUI — P19

Ownership: desktop backend/bridge/QML и новый модуль настроек.
Критерий: существующие пользовательские параметры переживают перезапуск,
запись атомарна, повреждённый/неизвестный формат не приводит к потере данных,
секреты в preferences не копируются. Настройки активной сессии не меняются
задним числом. GUI smoke и unit tests подтверждают сохранение и восстановление.

### Task 4: Arch-пакет — P24

Ownership: новые файлы `packaging/arch/*`, целевые Python-проверки и документация.
Критерий: воспроизводимый PKGBUILD текущего desktop/helper/worker с правильными
путями и metadata системной интеграции, безопасное удаление без пользовательских
данных. Не скачивать непроверенные ядра, не включать unproven capabilities,
не устанавливать пакет/helper на пользовательскую ОС. Проверка архива отдельно.

### Task 5: Ранний Android gate — P03

Сначала ограниченная инвентаризация доступных SDK/NDK/Java/native wrappers и
точный runnable план согласно platforms.md. Device tool подтвердил отсутствие
Android Emulator в `/home/myrs/Android/Sdk`; это ограничение runtime-проверок,
но не причина останавливать Linux и подготовку проверяемого Android-host.
Результат исследования определяет следующий implementation PR.

Исследование завершено: SDK API 35 и Java 17 доступны, NDK/Gradle/native Android
артефакты отсутствуют. Пользователь явно отложил Android: продолжать Linux,
Android-инструменты не устанавливать и build-зависимости не добавлять.
Подготовленная на будущее совместимая связка:
AGP 8.7.3, Gradle 8.9, NDK 27.0.12077973, существующий SDK 35/Build Tools 34.
Совместимость проверена по официальным release notes Android Developers.
Android implementation/runtime-проверки отложены до нового указания пользователя.
Полная цель сохраняет P03/P26, текущая исполняемая очередь ограничена Linux.

### Task 6: Семантика составных правил — P07

Ownership: domain evaluation и его тесты, только соответствующие абзацы README.
Проверить расхождение: routing-and-tray.md требует AND доменной и CIDR-групп,
а evaluator использует OR. Сначала воспроизвести регрессию и установить контракт;
затем привести модель и объяснение к спецификации, сохранив DNS fail-closed,
приоритеты приватных VPN/LAN и OR значений внутри одной группы. Не включать
native mixed capability без отдельной доказанной интеграции.

## Следующие волны

- [ ] После Task 1: системный client/controller и GUI Connect/Stop/recovery.
- [ ] P14/P08: оба engine, DNS/Fake-IP fail-closed, поколения/TTL, матрица D-01…D-07.
- [ ] P15/P07/P20: каталог приложений, семантика политики, два редактора и A-01…A-08.
- [ ] P16/P17: приватные VPN/LAN/split DNS, смена сети/sleep и ru-direct snapshots.
- [ ] P04/P05/P06: оставшиеся входы/обновления, proxy peer verification, backup/keyring.
- [ ] P11/P21/P22: ошибки/экспорт, живой tray lifecycle и batch GUI.
- [ ] P23: измерения Qt/GTK/Slint и аппаратные G-UI/G-POWER.
- [ ] P24/P25: install/uninstall/update/rollback в VM, notices/SPDX и Linux-приёмка.
- [ ] P03/P26: технический Android-host, приложение, оба engine и физическая приёмка.

После каждого PR обновлять фактический остаток; после каждой волны сразу
назначать следующую доступную работу. Внешний blocker фиксировать для конкретного
критерия, продолжая остальные независимые требования.
