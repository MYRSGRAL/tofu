# ResolverHost: факты и поток DNS-конфигурации

Цель — фактическая глобальная/per-link конфигурация resolved, а не предположение
по наличию NetworkManager или D-Bus-службы. На целевом хосте DumpDNSConfiguration
доступен, mode=foreign, SubscribeDNSConfiguration без интерактивной авторизации
возвращает InteractiveAuthenticationRequired. DNS хоста не перенастраивается.

1. Typed Varlink configuration: servers/fallback, search/route-only domains,
   DefaultRoute, DNSSEC/DoT, scopes и resolvConfMode. Unknown/nullable поля не
   превращаются в выдуманный успешный split DNS; Debug скрывает адреса/зоны.
2. Ограниченный NUL-framed transport для DumpDNSConfiguration и SubscribeDNSConfiguration.
   Только фиксированные методы, 1 MiB/frame, общий startup deadline/cancel,
   обработка multiple frames, EOF, неизвестного метода и отказа авторизации.
3. Endpoint привязан к защищённому /run/systemd/resolve, проверяются inode и
   SO_PEERCRED; origin namespace неизвестен, если его нельзя подтвердить.
   Подписка не использует idle polling; после ошибки сокет закрывается.
4. Синтетический Unix/Varlink fixture, read-only host сверка без сохранения DNS
   адресов/зон, version/permission gates; MSRV/Clippy/Android и независимое ревью.

DNS configuration stream требует systemd API v258+, Dump — v259+. Нет fallback
к PropertiesChanged: часть нужных DNS-полей не обязана публиковать такие сигналы.
Снимок и первый frame не являются lease Apply, а отказ подписки не считается
полным наблюдением. ResolverHost Apply/targeted cache invalidation, TUN/G-DNS,
helper и ранее открытые архитектурные решения остаются следующими этапами.
