# Ограниченный offline-импорт подписок

## Реализация

Crate/CLI tofu-import разбирает URI и один Base64 envelope. Поддержаны core-поля
VLESS, VMess Base64-JSON, Trojan, Shadowsocks SIP002/legacy, HTTP CONNECT и SOCKS5:
endpoint (IDNA/IPv4/IPv6/port), credentials, transport/security kind.
Исходник и параметры сохраняются без тихого удаления расширений. Неизвестные,
неподдерживаемые и небезопасные параметры получают review flags. Все записи
остаются ParsedServer, а не разрешёнными к активации конфигурациями.

Базовые профили VLESS/VMess требуют UUID-форму id. Детальная проверка REALITY,
транспортов, cipher key sizes и возможностей закреплённых ядер остаётся adapter
этапом. SS AEAD-2022 userinfo не принимается в Base64, как требует SIP002.
Дубли query-параметров и верхних VMess JSON-полей отклоняются. Неверный тип net/tls
не заменяется молча значением по умолчанию.

Пределы: 8 MiB входа, 32 MiB decoded, 20 000 непустых URI-записей,
64 KiB на URI, JSON nesting 64. Чтение ограничено MAX_INPUT+1; превышение
не заставляет дочитывать бесконечный источник. Native JSON проверяется RawValue
без построения полного дерева и не исполняется, не открывает файлы и не скачивает
включения. JSON пока opaque candidate, не готовый native/managed engine plan.

Debug скрывает исходник, адреса, credentials и параметры. CLI получает тело
через stdin и выводит только aggregate counters/static codes/physical line indices.
Для Base64 индексы относятся к декодированному тексту. Exit 0 означает готовность
аналитического отчёта, не успешное сохранение или подключение.

Фатальные ошибки сохраняют исходные байты через ImportFailure::input(), включая
невалидный UTF-8. После IO error или превышения потокового лимита сохраняется
только полученный префикс с input_is_complete=false. Debug/Display не раскрывают
эти байты. Хранение не шифруется и не zeroize-ится этим crate; защищённое
постоянное хранилище и управление временем жизни секретов принадлежат host.

## Проверки

23 новых integration/CLI tests: шесть протоколов, partial import/line indices,
однослойный Base64 с padding/URL-safe/BOM, opaque JSON/depth, IDNA/IPv6,
SS legacy и percent-escaped password, duplicate/mistyped VMess, безопасная
диагностика canary-секретов, exact limits/overflow/unbounded reader и сохранение
полного/неполного исходника при ошибках.

Вместе с предыдущими crates: 45 тестов + 4 doctest. Rust 1.97 и MSRV 1.85.1,
fmt/clippy, MSRV release и Android compile-check проверяются локально.
Cargo.lock фиксирует совместимые с Rust 1.85 версии; URL/IDNA транзитивные ICU
зависимости выбраны resolver из совместимого диапазона.

## Приватная подписка

Через release CLI проверен предоставленный пользователем источник: HTTP 200,
1116 байт, Base64 envelope, parsed=2 (VLESS), rejected=1
(UnsupportedProtocol, decoded line 3), activation=not_checked.
URL передан downloader через stdin-конфигурацию, тело — через stdin импортера;
секреты не попадали в argv, файлы, Git или диагностический вывод.
Redirect не следовал, TLS validation включена, размер ограничен.
Это проверка импорта, не работоспособности серверов и не production FetchHost.

## Осталось

Нет загрузки/таймеров подписок, транзакционного snapshot replacement, подтверждения
частичного сохранения, fingerprint/избранного, encrypted storage, managed JSON
extraction, config compilers и активации. Hysteria2/TUIC и другие URI-диалекты
отклоняются с сохранением исходника. Полные FR-06/07 ещё не закрыты.

Источники форматов:
[SIP002](https://shadowsocks.org/doc/sip002.html),
[VMess share JSON](https://github.com/2dust/v2rayN/wiki/Description-of-VMess-share-link).

## Review и исправления

Независимое ревью diff `2c96860..be439e3` нашло три P2: bare IPv6 в VMess JSON
не проходил URI-oriented Host::parse; query security мог отключить TLS для HTTPS
proxy; неверные типы/диапазон aid не диагностировались. Все три воспроизведены
регрессионными тестами до исправления.

IPv6 теперь сначала нормализуется как IpAddr; конфликт security с HTTP(S) scheme
отклоняется; aid допускает только unsigned number/decimal string в диапазоне
0…65535, ненулевое значение получает LegacyOption. Диапазон соответствует
[описанию alterId](https://www.v2fly.org/config/protocols/vmess.html#userobject).
Review не включал отложенные FetchHost/storage/managed JSON/engine adapters.
