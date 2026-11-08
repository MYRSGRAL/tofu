# Исполняемый Linux host локального SOCKS

Основание: EngineHost, PreparedPlan, FR-03/04/10. Пользователь разрешил
автономную реализацию существующей спецификации, проверки и merge после review.

1. tofu-linux — отдельный Linux-only host; portable crates сохраняют Android
   compile-check. Tokio runtime принадлежит оболочке. Reqwest/rustls выполняет
   HTTPS через явный SOCKS5h, без env proxy, redirects и direct fallback.
2. Пользователь указывает путь ELF ядра и ожидаемый SHA-256. Открыть regular file
   с O_NONBLOCK, ограничить поток 128 MiB, скопировать/хешировать в sealed executable
   memfd. Версию, config-check и start выполнять из этого immutable snapshot;
   исходный pathname после чтения не используется. Linux >=6.3 с executable memfd.
3. Скомпилированные байты конфигурации закрыты и живут в памяти. Config-check
   получает stdin, затем Session.prepare/apply сверяют SHA-256 тех же байтов.
   Системная сеть не меняется, поколения network/applications/authorization
   для этого ограниченного режима не представляют TUN/VPN snapshot.
4. Все subprocess операции имеют timeout и явную отмену с kill+wait. Stdio ядра
   не экспортируется, errors — статические коды. Дочерний процесс принадлежит
   host, Stop идемпотентен; SIGINT/SIGTERM работает и во время подготовки.
5. Перед NetworkApplied убедиться, что TCP LISTEN 127.0.0.1:port принадлежит
   PID ядра: пересечение inode из /proc/PID/fd с /proc/PID/net/tcp. Перед/после
   HTTPS перепроверить владельца и состояние процесса. Connected только после
   HTTPS 2xx; timeout/HTTP отказ — Limited, процесс остаётся для использования.
6. CLI tofu-local читает URI/Base64 импорт из stdin, выбирает сервер,
   по умолчанию отклоняет частичный импорт. --accept-partial вместе с явным
   --server разрешает только выбранную распознанную запись и сообщает число ошибок;
   показывает только безопасные NDJSON состояния. --once проверяет и останавливает;
   обычный режим обслуживает SOCKS до Ctrl+C/SIGTERM или выхода ядра. HTTPS URL
   настраивается, только https, без credentials/fragment; по умолчанию generate_204.
7. Сначала тесты snapshot/hash/seals/version/timeout/cancel/URL, затем реализация.
   Настоящие ядра: запуск/остановка, чужой занятый порт, SIGTERM при подготовке,
   unexpected exit, local HTTPS через VLESS и отрицательные TLS/status проверки.
   После fmt/clippy/tests/MSRV/release/Android и README RU/EN — review, fixes, PR.

Выбран прямой async subprocess host с отдельным HTTPS клиентом. Shell/curl
оставлены только диагностике: продукт не зависит от shell и не передаёт
credentials в argv. Native libraries Android и daemon/helper сейчас не нужны.
Этот этап не закрывает TUN/DNS, storage/fetch, policy compiler, GUI/tray,
автоматическое recovery или полную классификацию отказов удалённого туннеля.
