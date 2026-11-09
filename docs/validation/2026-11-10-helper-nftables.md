# Nftables: собственные таблицы и восстановление cleanup

База: `ed770be`, впоследствии влитая PR #31 как `acff6c5`. Это продолжение
P12, не завершение системного VPN или kill switch.

## Реализовано

- Типизированный NftExecutor: exclusive create, наблюдение полного ownership,
  delete с family и ненулевым kernel handle без name.
- Закреплённые namespace и executable FD; строгая проверка системного nft,
  закрытые argv/stdin, env_clear, cwd `/`.
- До четырёх дочерних процессов глобально, один на executor; вывод каждого
  потока до 256 КиБ, выполнение команды до 5 секунд. Отмена сохраняет владельца
  Child до kill/wait и удерживает permits до reap.
- Durable intent/reconcile и новый Journal::mark_cleanup для асинхронного
  удаления. Чужие и неоднозначные объекты не удаляются по имени.

## Проверки

Rust 1.85.1, существующие зависимости без изменения состава/версий:

- Workspace: **403 passed, 0 failed, 8 ignored**, 42 suites.
- Journal: **17 passed**, новый переход прошёл RED/GREEN.
- Nft unit suite: **9 passed, 1 ignored**. Проверены parser/identity, ограничения
  вывода, deadline, отмена/reap и отказ cleanup replay до запуска команды.
- Workspace/all-targets Clippy с `-D warnings`, fmt и diff check: PASS.
- Отдельно выполнен ignored namespace-тест: **1 passed**. Реальные nftables
  эффекты: create/reopen/reconcile/remove, неправильный scope, чужая таблица,
  exclusive create, старый handle после замены таблицы и crash cleanup replay.
- Дополнительная прямая kernel-проба подтвердила новый handle при замене имени
  и сохранение чужой таблицы при удалении по старому handle. Локальный nft 1.1.7.

Все сетевые эффекты выполнены через unshare user/map-root/net/mount/pid.
Внутри смонтированы собственный маленький tmpfs `/tmp`, proc и sysfs. Guard
требует другой netns по сравнению с переданным host identity и только loopback.
Первый запуск guard корректно отверг изоляцию со старым sysfs mount, который
показывал интерфейсы host; до сетевых эффектов не дошёл. Повтор с собственным
sysfs прошёл. Сеть, DNS, system bus и polkit пользователя не менялись.

## Независимое review

Найден P2: после durable CleanupRemaining, успешного delete и crash до записи
Absent повтор ensure того же intent создавал таблицу заново. Исправлен отказ
до наблюдения/команды для CleanupRemaining. Namespace regression сначала
воспроизвёл FAIL с `Ok(())`, после исправления прошёл; обычный unit проверяет
границу до запуска процесса до и после reopen. Узкий повторный review закрыл
P2 без новых непосредственно связанных findings.

## Границы результата

Создаётся пустая таблица, а не блокирующие правила. Нет системного service/IPC,
TUN, авторизованного внешнего Unlock/Apply и полного восстановления сессии.
Namespace test использует только закрытый cfg(test) fixture executable: в
userns host-root файл отображается как nobody. Production-проверка UID=0
не ослаблялась; отдельно проверена read-only на установленном системном nft.
Поведение с враждебным CAP_NET_ADMIN/root вне заявленной границы доверия.

Evidence: `local/validation/helper-nft/`. Восстановимые build-артефакты очищаются
после проверки; для повторения требуется пересобрать test binary.
[Контракт и первоисточники](../specs/helper-nftables-executor.md).
