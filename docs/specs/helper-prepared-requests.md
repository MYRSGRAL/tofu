# Одноразовые подготовленные запросы network helper

Компонент `PreparedRequests<A>` связывает проверенный artifact с D-Bus peer,
контекстом сессии и новой авторизацией перед выдачей для Apply. Это часть P12;
выдача artifact не является подтверждением изменения сети.

## Граница доверия

Production constructor открывает канонический system bus через `PeerAuthorizer`.
`connection()` возвращает то же соединение для будущего dispatcher. Входящий
`Message` должен быть исходным сообщением этого доверенного transport; UID не
берётся из body. Разбор версии/allowlist wire DTO и системный service — следующий
слой. Проверка 64 KiB в authorizer происходит после получения Message библиотекой;
это не защита от всех транспортных аллокаций D-Bus.

Factory и reader контекста выполняются доверенным host. Они не являются полями
клиентского payload. Factory должен сам выбрать ядро из [реестра](helper-trusted-registry.md),
скомпилировать типизированное намерение и выполнить version/config-check.
`PreparedArtifact` реализован на настоящем `CheckedConfig`; текущий SHA-256
вычисляется по его private bytes. Один valid trait-объект сам по себе не повышает
доверие пользовательского engine-dir до системного.

## Prepare

1. Проверяются формат Message и доступность bounded слотов.
2. Выполняется свежая polkit-проверка ManageNetwork.
3. Только после allow вызывается ограниченный по времени factory.
4. Проверяются размер artifact и живой peer после длительного config-check.
5. Сохраняются artifact, UID/unique sender, session/generation, EnvironmentStamp,
   фактический digest, размер и TTL; клиент получает случайный 32-байтный handle.

`PreparedContext` требует ненулевые session/generation и поколения network,
applications, authorization, engine с `authorized=true`. LOCAL_ENV локального
SOCKS не является подходящим контекстом управляемой сети.

## Claim перед Apply

`claim_for_apply` возвращает `ClaimedArtifact<A>`, удерживающий память вместе с данными,
и заново проверяет ManageNetwork. Reader актуального контекста
вызывается после polkit, чтобы время диалога не закрепляло старые поколения.
Затем проверяются TTL, UID и unique sender, session/generation/environment,
размер и SHA-256 текущих bytes. Artifact выдаётся только один раз под lock.
Чужой peer не уничтожает правильный handle; rightful stale-попытка его инвалидирует.
Повторный или истёкший handle не выдаёт artifact.

Trusted caller обязан сериализовать claim, эффекты и изменение поколений. Между
выдачей и эффектами нельзя допускать подмену проверенного окружения. Модуль не
заменяет reducer/PreparedPlan, journal, G-DNS, отмену effect или rollback.

## Ограничения ресурсов и lifecycle

- До 16 pending/reserved запросов суммарно, до 4 на unique sender.
- До 4 операций in-flight; свободный слот проверяется до auth и factory.
- Конфигурационный payload до 4 MiB, TTL 60 секунд; factory и дополнительная liveness-проверка
  ограничены 30 секундами. Polkit имеет собственные отдельные deadlines.
- Expiry освобождается при следующей операции без idle polling.
- Reservation освобождается при отказе/отмене/Drop. Trusted NameOwnerChanged
  handler вызывает `forget_peer`; этот hook не является клиентским RPC.
- Handle, контекст и artifacts скрыты в Debug. Ошибки не включают body, raw
  D-Bus errors, пути, URL и credentials.

При чужом/oversized reply авторизации helper также отменяет настоящий pending
CheckAuthorization у того же pinned authority. Поздний allow не становится
разрешением; удалённая отмена остаётся best effort с отдельным deadline 250 мс.

Payload и executable images имеют [общий процессный бюджет](helper-memory-budget.md).
Перед factory резервируется retained ёмкость, claim передаёт RAII-владельца,
а weak-cache реестра разделяет неизменяемый image. Учёт не является измерением RSS
или подтверждением G-POWER.

## Приёмка

Используется отдельный dbus-daemon с fake authority и два настоящих клиента.
Нужны два roundtrip авторизации на Prepare/claim; deny/challenge/revoke,
другой unique sender при том же UID и replay, concurrent claim, stale контекст/digest, TTL/caps,
отмена factory/auth и peer disappearance. Тесты clocks/constructors закрыты cfg(test).
Host system bus, настоящий polkit и пользовательская сеть не изменяются.
