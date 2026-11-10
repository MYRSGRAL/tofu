# Проверка helper service — 10 ноября 2026

Кандидат: ветка `feat/helper-service`, база `cef982a` (влитый PR #34).
Назначение — ограниченный IPC v1 и actual public-protection backend, без установки
системного unit и без подключения VPN через GUI.

## Результаты

- `cargo +1.85.1 test --workspace`: **442 passed, 0 failed, 12 ignored**, 43 suites.
- `clippy --workspace --all-targets -- -D warnings`: PASS; rustfmt/diff: PASS.
- Python: **100 tests**, PASS; Ruff 0.17.4 format/check всех tools: PASS.
- Private D-Bus проверяет Prepare/Apply, peer binding, replay, deny/challenge,
  stale context, неверные version/signature/поля и раздельные polkit actions.
- При paused polkit для Apply/Stop/Unlock State доступен за 500 ms;
  до успешной авторизации эффектов нет.
- Отдельный ignored сценарий с реальным host и nft: `nft -c` в Prepare не создаёт
  таблиц; Apply блокирует TCP/UDP IPv4/IPv6; Stop сохраняет блокировку; новый
  backend читает журнал и сообщает RecoveryRequired; свежий Unlock восстанавливает
  все четыре traffic-пробы. PASS в отдельных user/net/mount/pid namespaces с
  приватными proc/sysfs, /tmp, /run и отказом при совпадении с host netns.
- Journal: 23 integration tests, включая 5 новых сценариев owner recovery,
  durable передачи пустого журнала, отказа при ресурсах/политике, poisoned write и
  небезопасном/повреждённом storage без перезаписи.
- Actual `yonik-helper --check-worker-fence`: положительная и отрицательная
  проверки в собственных transient user units, PASS. Чужой живой процесс даёт
  отказ. Все созданные units неактивны после штатной очистки.
- Inert systemd lifecycle: SIGKILL leader при TERM-игнорирующем child;
  новый marker создан лишь после исчезновения прежних PID/starttime, PASS.
  Unit остановлен, записанные identities исчезли. Сетевых эффектов нет.
- XML packaging parse PASS. `systemd-analyze verify` исходного unit сообщает
  отсутствие неустановленного `/usr/lib/yonik/yonik-helper`; временная копия с
  единственной заменой executable на собранный кандидат проходит с exit 0.
  Это статическая проверка, не доказательство установки root service.

## Независимое ревью

Sol review выявило P1 stale LAN allow после topology change и P2 mutex во время
ожидания polkit. Оба исправлены: LAN явно unsupported до reconciler;
авторизация предшествует host lock, Apply держит guard от context до эффекта.
Узкое повторное review не нашло новых ошибок в исправлениях.

Локальные полные логи: `local/validation/helper-service/` (не включаются в git).
Использовалась только синтетическая сеть: подписки, HWID и ключи не запрашивались.

## Ограничения

Нет system TUN, DNS, GUI wiring, LAN и фонового reconcile. Startup fence подтверждает
отсутствие живых старых workers, не завершение всех операций ядра. System unit не
устанавливался. P12 остаётся частичным, итоговый процент плана не меняется.
