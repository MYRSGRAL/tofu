# Peer-bound подготовленные artifacts

База — `628d5c3` (PR #29, оба push/PR workflows SUCCESS). Компонент относится
к P12 и не подтверждает работу системного сервиса или применение сети.

## Реализация

`PreparedRequests<A>` использует один system-bus authorizer. Prepare выполняет
fresh polkit до factory, затем связывает проверенный artifact с UID/unique sender,
session/generation/EnvironmentStamp, актуальным SHA-256, размером и TTL.
Claim заново авторизуется; trusted reader контекста вызывается после polkit.
Выдача атомарно одноразовая. Чужой sender не уничтожает rightful handle, stale
rightful-попытка инвалидирует его. `CheckedConfig` реализует digest по своим
private bytes. Контекст LOCAL_ENV не подходит для управляемой сети.

Пределы: 16 pending/reserved, 4 на sender, 4 in-flight до auth/factory, 4 MiB
конфигурационных bytes, TTL60s, factory/liveness30s. Bytes не включают sealed
binary: общий бюджет/переиспользование snapshots ещё нужны до production service.

## Исправление авторизации

Регрессия воспроизвела отсутствие CancelCheckAuthorization после forged reply:
сначала MethodReturn, затем MethodError от постороннего peer. Оба RED завершились
Elapsed при ожидании отмены настоящего check. Исправление отправляет bounded
CancelCheckAuthorization закреплённому owner с тем же cancellation ID при ошибке
RPC либо неверном sender/oversized reply. Late allow остаётся отвергнутым.

## Проверки

- 8 новых тестов, включая настоящую private D-Bus/fake authority: два auth roundtrip,
  deny/challenge/revoke, два peer с одним UID, concurrent/replay, stale context/digest,
  TTL/caps/expiry, отказ/отмена factory, peer disappearance/forget и reservations.
- Helper-группа после исправлений: **27 passed**.
- Весь workspace Rust1.85.1: **380 passed, 0 failed, 7 ignored**.
- Workspace/all-targets Clippy `-D warnings`, fmt и diff check: PASS.
- Независимое security review нашло P2 для forged MethodError. Исправлен;
  узкий повторный review подтвердил закрытие, новых findings нет.

Тесты используют собственную шину и синтетические artifacts; system bus/polkit,
пользовательские ключи и сеть не изменялись. Динамический тест другого реального
UID не выполнялся; проверены разные unique sender при одном UID и логика binding.
Trusted dispatcher должен сериализовать claim/effects и изменения поколений.
Wire DTO, service, сетевые эффекты и recovery ещё не реализованы этим срезом.

Evidence: `local/validation/helper-prepared/` (RED/GREEN и полные журналы).
