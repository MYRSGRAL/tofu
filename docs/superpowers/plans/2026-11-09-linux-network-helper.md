# Системный Linux NetworkHost/helper

> Исполнителям: применять superpowers:executing-plans либо согласованное
> делегирование с ownership файлов. Выполнять задачи по порядку с RED/GREEN и
> независимым ревью перед слиянием.

**Цель:** реализовать привилегированный host, который применяет проверенный
сетевой план, учитывает собственные ресурсы и восстанавливается после сбоя без
удаления чужой сети и случайного снятия включённого kill switch.

**Архитектура:** пользовательский controller передаёт типизированное намерение
через D-Bus; helper устанавливает UID отправителя, авторизует действие через
polkit, сам компилирует/проверяет immutable artifact и выдаёт одноразовый handle.
Побочные эффекты сопровождаются устойчивым журналом intent/result. Восстановление
начинается со сверки ОС; записи журнала не доказывают владение существующим объектом.

**Стек:** Rust 1.85.1, существующие zbus/Tokio/rustix/serde; systemd, Netlink,
nftables, systemd-resolved. Новые зависимости требуют отдельного обоснования.

**Спека:** [Linux](../../specs/platforms.md),
[сеть](../../specs/networking.md#8-ipv6-mtu-и-отказ),
[контракты](../../specs/data-and-contracts.md),
[приёмка](../../specs/validation-and-roadmap.md).

## Ограничения

- Основная реализация — Rust. Существующий локальный SOCKS остаётся самостоятельным
  режимом; его `LOCAL_ENV` не является авторизацией системных изменений.
- Вход helper не содержит shell, executable path, произвольных argv/environment,
  systemd unit либо сырого engine JSON. Trusted registry и service templates
  принадлежат системной установке; пользовательский engine-dir не повышается.
- UID берётся из D-Bus credentials, не из полей сообщения. План привязан к peer,
  session/generation, network stamp, авторизации и digest immutable artifact.
- Нельзя восстанавливать всю сеть/DNS из старого snapshot. Отменяются только
  подтверждённые собственные изменения; неоднозначность остаётся в CleanupError.
- Kill switch ON сохраняет публичную блокировку после terminal failure/остановки;
  OFF восстанавливает обычную сеть. Explicit Unlock снимает только собственную
  блокировку. Сессионные private CIDR/DNS и публичная блокировка имеют разный срок жизни.
- Все интеграционные эксперименты — собственные user/PID/mount/net namespaces и
  private D-Bus. Системные службы/маршруты пользователя не изменяются.
- Планы и journal не содержат credentials, body подписки или секретные URL.
  Native JSON и непроверенный DNS/Fake-IP контракт не активируются обходным путём.

## Файлы и границы

| Путь | Ответственность |
| --- | --- |
| `crates/yonik-linux/src/managed_network/` | Журнал, проверка/сверка владения и последовательность эффектов |
| `crates/yonik-linux/tests/network_journal.rs` | Настоящие filesystem recovery/permission/lock проверки |
| `crates/yonik-linux/src/network/` | Существующий read-only topology/lookup; отдельные типизированные write-операции |
| `crates/yonik-linux/src/resolver/` | Существующие snapshots; отдельный owned resolved-adapter |
| `crates/yonik-linux/src/helper/` | Версионный D-Bus DTO, peer/polkit, immutable handles, trusted registry |
| `crates/yonik-linux/src/bin/yonik-network-helper.rs` | Системный процесс; без UI и универсального запуска команд |
| `packaging/linux/` | Фиксированные D-Bus/polkit/systemd файлы и registry установки |
| `tools/network_helper_smoke.py` | Полная изолированная проверка Apply/rollback/recovery |
| `crates/yonik-desktop/src/backend/` | Последующее подключение controller к подтверждённому helper state |

## Проверка ревью

1. Чужой объект с совпадающим именем/ID не удаляется при cleanup/recovery.
2. Права или сеть меняются во время polkit/config-check: Apply не использует старую авторизацию.
3. Процесс падает между intent, эффектом и result: recovery не считает операцию успешной автоматически.
4. Частичная очистка/недоступный DNS-manager сохраняет перечень остатков и не выдаёт успех.
5. Старый Unlock/Stop другого peer или поколения не снимает новую публичную блокировку.

## 1. Устойчивый журнал владения

Интерфейс первого компонента: `managed_network::Journal::open(&Path, Ownership)`;
`Ownership::new(owner_uid, session, generation, netns_device, netns_inode)`.
`intent` сохраняет запись до эффекта, `reconcile(Owned/Absent)` принимает результат
доверенной сверки, `cleanup(scope, callback)` учитывает результат удаления.
`RecordState` описывает историю записи, не состояние ОС. `Scope::PublicBlock`
нельзя очистить вызовом для `Scope::Session`. Один journal не перепривязывается к
другому владельцу/поколению: helper удерживает старую публичную защиту отдельным
журналом до подтверждённой передачи либо явного Unlock.

Начальные защитные пределы — 128 записей и 128 KiB сериализованного журнала;
превышение является ошибкой, а не усечением. Это лимит сетевых ресурсов одного
журнала, не числа серверов/профилей или правил внутри конфигурации ядра.

- [ ] Написать RED-тесты recovery/reopen, второго writer, symlink/hardlink,
  небезопасных прав, oversized/corrupt/unknown schema, generation/owner mismatch,
  незавершённого intent и оставшихся ресурсов после частичной очистки.
- [ ] Реализовать typed descriptors, owner/session/generation и отдельный scope
  сессионных ресурсов/публичной блокировки в `managed_network`.
- [ ] Открывать приватный каталог через directory FD, удерживать exclusive lock,
  не следовать ссылкам. Запись: ограниченная сериализация → exclusive temp →
  fsync файла → atomic rename → fsync каталога. Ошибка commit остаётся ошибкой
  durability, даже если rename уже виден.
- [ ] При reopen требовать explicit reconciliation; не создавать/удалять сеть.
  Corrupt/unknown journal сохраняется для разбора, не заменяется пустым.
- [ ] GREEN: реальные файловые тесты, MSRV fmt/clippy, review и атомарный commit.

Команды компонента: под BSD lock корня
`/var/tmp/codex-myrs/transient/yonik-network-journal`, с
`CARGO_TARGET_DIR=/var/tmp/codex-myrs/transient/yonik-network-journal/target`:
`cargo +1.85.1 test -p yonik-linux --test network_journal --locked` и
`cargo +1.85.1 clippy -p yonik-linux --all-targets --locked -- -D warnings`.
Ожидаются PASS и отсутствие предупреждений соответственно. Временную сборку
очистить после сохранения необходимых evidence вне TTL-каталога.

## 2. Авторизованный helper и одноразовый Prepare/Apply

- [ ] RED на private D-Bus: подмена UID, иной sender, revoke/исчезнувший peer,
  oversized DTO, повтор Apply, изменённый artifact/network stamp, stale generation.
- [ ] В `helper` ввести версии DTO и allowlist операций Capabilities, Prepare,
  Apply, Stop, Reconcile, Unlock и State. Возвращать структурированные ошибки;
  не принимать пути/команды. Лимиты задавать до крупных аллокаций.
- [ ] Prepare сам проверяет типизированный профиль/возможности, registry и
  immutable конфиг; polkit subject привязан к уникальному D-Bus sender.
  Handle нельзя повторно использовать или передать иному peer.
- [ ] Apply повторно проверяет авторизацию, окружение и digest прямо перед
  эффектами; callbacks старого поколения не изменяют новую сессию.
- [ ] GREEN: private-bus integration, отсутствие сетевых эффектов при любом
  отказе, отсутствие секретов в ответах/ошибках и независимое security review.

## 3. Реальные эффекты и компенсация

- [ ] RED в namespace: занятые TUN/table/rule IDs, ошибка каждого шага Apply,
  частично успешный cleanup, чужой маршрут/VPN/DNS меняется параллельно.
- [ ] Привязать journal intents к типизированным операциям TUN, Netlink,
  nftables и resolved. Подтверждать ownership по идентичности объекта в нужном
  namespace, а не только по имени из журнала.
- [ ] Подготовить защиту и исключения от зацикливания до захвата; запускать
  только registry engine через фиксированный systemd template. Идентичность
  service instance входит в учёт, PID без start identity недостаточен.
- [ ] Применять routes/DNS с подтверждением ACK/наблюдаемого состояния; applied
  generation выдаётся после готовности всех обязательных ресурсов. При отказе
  компенсировать подтверждённые шаги в обратном порядке, продолжая cleanup после
  единичной ошибки и сохраняя остатки.
- [ ] GREEN: обе закреплённые версии ядер, TCP/UDP IPv4/IPv6 через реальный TUN,
  исключение proxy endpoint, сохранение постороннего маршрута и DNS. Переиспользовать
  изоляцию `tools/engine_traffic.py`; не выдавать direct echo за проверку provider.

## 4. Потеря controller/helper и kill switch

- [ ] RED: SIGKILL на границах intent/effect/result; controller исчезает;
  helper возвращается; TUN уже занят другим владельцем; public protection остаётся.
- [ ] Reconcile сверяет каждый дескриптор с ОС. Неизвестное покрытие отображается
  явно; осиротевший engine останавливается только при подтверждённой идентичности.
- [ ] Разделить состояние сессии и applied-защиты: остановленный TUN при
  включённом kill switch не означает разблокированную сеть. Unlock авторизуется
  и коррелируется с владельцем/поколением, снимает только свои объекты.
- [ ] GREEN: traffic fixtures доказывают блокировку ON и обычную сеть OFF,
  сохранение LAN/private исключений, retry cleanup и отказ stale/foreign Unlock.

## 5. Controller/UI и поставка

- [ ] Подключить controller к helper state; не рисовать applied по одному
  значению настройки. Проверить отмену Connect/Apply, два Connect, Stop во время
  авторизации, закрытое окно и исчезновение helper.
- [ ] Подключить TUN только при пройденных совместимых сетевых/DNS capabilities.
  Незакрытые G-DNS/Fake-IP/private binding не скрываются за работающим helper.
- [ ] Проверить фиксированные service/polkit/registry и hardening обоих ядер
  в изолированной тестовой установке; обычный пользователь не подменяет binary/config.
- [ ] Обновить README RU/EN и evidence по фактически проверенным осям; review,
  зелёный CI, сборка и разрешённое слияние. G-LINUX/G-DNS/G-POWER и Linux v1
  закрываются собственными полными проверками, а не этим списком задач.
