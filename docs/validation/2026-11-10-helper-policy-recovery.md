# Durable policy и восстановление после сбоя

База `d51492c`, влитая PR #33 как `37401a4` после всех пяти зелёных jobs.
PR #32 влит как `dd82114`, также после зелёных push/PR workflows.
Это продолжение P12, не приёмка всего системного helper.

## Изменения

- Journal schema v2: resource intent и типизированный desired/previous/Unlock
  payload сохраняются одной атомарной записью. Revision монотонная, slot один,
  Removing содержит не более двух проверяемых кандидатов.
- Строгая миграция v1, валидируемый policy codec, ограничения размера/полей,
  связи phase/resource/state и redacted Debug. Legacy public records без payload
  не выдаются за отсутствие политики и не получают выдуманный previous.
- Durable Apply/Unlock связаны с реальным nft executor. Startup recovery
  только наблюдает; desired, previous, missing и pending Unlock различаются.
  Без свежей проверки окружения правила автоматически не воспроизводятся.
- Generic cleanup не обходит Removing; завершённое отсутствие атомарно удаляет
  slot и resource record. Неопределённые записи блокируют NoPolicy/NoOp shortcuts.
- Общий permit сериализует nft-команды разных executor в одном живом runtime
  и освобождается после kill/wait старого Child.

## Проверки Rust 1.85.1

- Workspace: **427 passed, 0 failed, 11 ignored**, 42 suites.
- Managed network unit suite: **33 passed, 4 ignored** в итоговом workspace.
- Workspace/all-targets Clippy `-D warnings`, fmt и diff check: PASS.
- Реально выполнены три ignored namespace-сценария: crash recovery, прежний
  TCP/UDP IPv4/IPv6 public block и прежний create/reopen/foreign/stale handle.
  Каждый прошёл; worker-fixture отдельно не считается завершённым тестом.
- Набор зависимостей не изменён; Cargo.lock нормализуется только после
  подтверждённой семантической эквивалентности перестановки package records.

Crash-сценарий запускает дочерний helper и делает настоящий SIGKILL после
подтверждённого durable/effect boundary, затем wait/reap и reopen журнала:

1. Install intent сохранён, таблицы нет: recovery сообщает Missing и не
   устанавливает firewall; исходная сеть остаётся доступной.
2. Install effect завершён, Stable ещё не записан: реальные правила сверяются
   с desired, после чего историческое состояние становится Stable.
3. Replace intent сохранён, старые правила действуют: PreviousRules не выдаётся
   за новый desired; явный Apply завершает переход и меняет LAN-доступ.
4. Replace effect завершён: recovery узнаёт новые правила до записи Stable.
5. Unlock intent сохранён, таблица ещё существует: recovery сообщает UnlockPending,
   не снимает блокировку автоматически.
6. Delete завершён, отсутствие ещё не записано: Missing сохраняет Unlock intent;
   явный Unlock завершает атомарную очистку. Повтор без managed records — NoPolicy.

Во всех шести точках родитель проверяет завершение именно сигналом 9.
Публичные IPv4/IPv6 и LAN-пробы отличают старую/новую политику и разблокировку.
Изоляция: отдельные user/net/mount/pid, proc/sysfs, `/tmp` и `/run`; host netns
отвергается. Пользовательские DNS, polkit, system bus, подписки и SQLite не менялись.

## Независимое review и дополнительные regressions

Review store/codec/migration/runtime не нашло новых ошибок phase-переходов.
Оно указало существенную границу: kill между завершёнными эффектами не проверяет
SIGKILL во время живого nft subprocess. Межпроцессный fencing остаётся задачей
systemd/cgroup сервиса; полной такой гарантии здесь нет.

Для одного живого runtime отдельный RED воспроизвёл обгон живой команды другим
executor. Общий permit уменьшен до одного; GREEN подтверждает Pending второго
executor до отмены и reap первого PID. Узкий review без findings.

Дополнительный RED нашёл возврат NoPolicy из poisoned journal с прежним пустым
snapshot. Оба shortcut теперь вызывают общий journal.check до чтения; новый
тест требует UncertainWrite для recovery и Unlock. Узкий review закрыл исправление.
Весь workspace после обоих исправлений прошёл.

## Остаток

Нужны service/IPC с авторизованными entrypoints, актуальные topology/engine
bindings, межпроцессный cgroup lifecycle и in-flight crash-тесты, TUN/DNS и
полное восстановление сессии. DesiredRules доказывает совпадение правил,
а не G-LINUX/G-DNS и не безопасность повторного использования ifindex.

Evidence: `local/validation/helper-durable-policy/`. Own build очищается после
сохранения логов; исходники и evidence остаются в workspace.
[Контракт](../specs/helper-policy-recovery.md).
