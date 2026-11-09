# Проверка загрузки и snapshot подписок

Среда: CachyOS x86_64, Rust 1.85.1, системный SQLite 3.53.4, KDE Secret Service.
Изменения продолжают контракт `data-and-contracts.md`, разделы 1–4.

## Реализованный тракт

`SubscriptionService::refresh` читает зашифрованный источник → Direct FetchHost
проверяет каждый DNS-набор/peer/редирект → bounded importer строит неизменяемый
отчёт → чистый candidate либо явное согласие на конкретный отчёт → SQLite CAS
по source generation и snapshot revision → атомарная публикация сохранённого
snapshot. Активная engine-сессия этим API не изменяется.

HTTP 206 и Content-Range без запроса Range отклоняются: корректный URI-префикс
частичного ответа не должен заменять полный список. Разбор обновления и offline
snapshot выполняется на blocking pool с общим лимитом двух CPU-heavy задач;
permit остаётся у запущенной задачи до её завершения даже при отмене ожидания.

Новое обновление в том же сервисе отменяет старую загрузку и инвалидирует старое
согласие; отдельные экземпляры защищены транзакционным CAS. `cancel(id)` действует
и на загрузку, и на подготовленный candidate. Переданный watch receiver отменяет
сетевую фазу; после получения Pending UI использует `cancel(id)` либо удаляет
candidate. После отправки SQL commit операция может завершиться даже при удалении
awaiting future: результат сверяется повторным чтением, а не слепым retry.

SQLite хранит ciphertext URL/имени, тела, метаданных и validators. XChaCha20Poly1305
использует случайный nonce 24 байта; AAD связывает database ID, тип записи,
source ID, поколения и revision. Sentinel проверяет ключ даже в пустой базе.
Чужая/будущая схема, неверный ключ и повреждение не вызывают сброс данных.
Ключ предоставляется Secret Service через отдельный DH-сеанс; в БД его нет.
Создание ключа сериализуется lock собственного runtime-каталога. Для существующей
БД создание нового ключа запрещает вызывающий host через `allow_create = false`.

## Доказательства

- `cargo test --workspace --locked`: 121 Rust-тест и 4 doctest прошли.
- 5 тестов переносимого staged-controller: ошибки/пустота, точное согласие,
  чужие tickets, отмена/смена источника и opaque native JSON.
- 8 hermetic FetchHost тестов: public/private/transition IP, URL/validator
  validation, отсутствие proxy fallback, body/redirect limits, conditional 304,
  отсутствие validators на другом URL, отмена и timeout до заголовков.
- 5 тестов Repository: переоткрытие, отсутствие plaintext markers в DB/WAL,
  CAS двух соединений, сохранение старого snapshot при смене источника,
  неверный ключ, tamper AAD/sentinel, будущая схема без изменения файла,
  права/симлинки и лимиты данных.
- 11 интеграционных тестов сервиса: весь тракт через локальный HTTP fixture,
  offline после остановки сервера/переоткрытия БД, review частичного импорта,
  новые запросы/отмена/смена источника, чужое согласие, CAS независимых сервисов,
  validators после 304, HTTP/empty failures и отсутствие proxy fallback.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`, fmt,
  release build и `cargo check --workspace --target aarch64-linux-android --locked`
  прошли на Rust 1.85.1. Android проверяет переносимые части; Linux адаптеры
  исключены по target cfg, это не Android SQLite/keystore реализация.
- Отдельно запущен ignored `credentials::tests::secret_service_readonly`:
  успешное DH-подключение и поиск ключа, без создания/изменения записей пользователя.
  Запись нового production-ключа этим тестом не проверяется.
- Отдельно запущен ignored `fetch::tests::public_https_direct`: реальный
  `https://www.gstatic.com/generate_204` через Direct с DNS pinning, TLS и peer
  membership check вернул успешный пустой ответ. Оба environment-dependent теста
  исключены из обычного hermetic прогона и пройдены отдельно на рабочей машине.

Обычные тесты используют вымышленные секреты, отдельные каталоги 0700 и только
loopback HTTP. Настоящая тестовая подписка в файлы и аргументы процессов не попадала
и в этом этапе не загружалась. Системные маршруты/proxy/DNS не менялись.

## Ограничения и следующие требования

Это библиотечная интеграция: команды существующего CLI ещё читают stdin, GUI,
таймеры/питание, server fingerprints и избранное, миграции после схемы v1 и
применение нового snapshot к активной сессии остаются впереди. Последний handle
SQLite в GUI должен закрываться через `close().await`.

Direct требует HTTPS и публичные адреса на каждом переходе; HTTP/private
разрешаются только явным исключением локального источника. IPv6 special-purpose
диапазоны консервативно отклоняются. ОС DNS lookup может завершаться в blocking
pool после отмены future; HTTP после отменённого lookup не запускается.

Proxy FetchRoute возвращает `UnsupportedPeerVerification` до сетевого I/O:
reqwest SOCKS раскрывает TCP peer прокси, а не конечного сервера. Для выполнения
полного контракта нужен доверенный engine-side upstream peer interface; сравнение
loopback proxy с allowlist конечного сервера не является доказательством.
Автоматический Direct fallback отсутствует. Это незакрытое требование, а не
признание proxy-загрузки готовой.

Native JSON сохраняется только после явного review; opaque snapshot не разрешает
запуск произвольного конфига. Public DNS rebinding/redirect проверяются кодом и
unit/fixture сценариями; отдельный управляемый публичный TLS/DNS стенд ещё нужен
для полной runtime-проверки SSRF на смене DNS. G-LINUX/G-DNS/G-UI не закрыты.
