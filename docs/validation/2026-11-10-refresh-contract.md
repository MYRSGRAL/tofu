# Адресная проверка refresh-контракта Linux

База: `f8975948406c920c4d78439348001394647d17dd`.

Новый integration target `subscription_refresh_contract` использует production
SubscriptionService/FetchHost, зашифрованный SQLite в отдельном каталоге и HTTP
listener на `127.0.0.1:0`. Каждый сценарий обязан завершить fixture и проверить
фактическое количество и HTTP request target. Внешние провайдеры, keyring и
настоящие credentials не используются.

Проверяются дополнительные условия к существующему `tests/subscriptions.rs`:

- Оборванный Content-Length после валидного URI-префикса.
- 206 и отдельно 200 с Content-Range.
- 401, 407 и redirect на запрещённую схему file.
- Каждый отказ сохраняет тело, metadata, validators, revision и поколение
  последнего snapshot; последующие запросы продолжают отправлять его ETag.
- Ротация URL-токена и имени профиля сохраняет ID и избранное; validators старого
  ресурса не передаются новому URL.

## Границы результата

Это частичное evidence P04/P05, не полный PASS. Proxy в текущем FetchHost
безусловно возвращает UnsupportedPeerVerification до сетевого запроса. API не
принимает адрес HTTP/SOCKS-прокси. Поэтому HTTP 407 в этой проверке является
ответом origin fixture и не доказывает proxy authentication. Фактический
HTTP/SOCKS Proxy runtime остаётся root-задачей согласования транспортного API.
Скрытый DIRECT fallback уже проверяется существующим integration test.

Identity evidence: `crates/yonik-import/tests/identity.rs` явно включает серверные
credentials в identity material. Ротация серверного пароля должна менять
fingerprint; здесь сохраняется identity при ротации токена источника.

Существующие partial/empty import, CAS и cancel до HTTP headers не копируются.
Отдельный тест удерживает последний байт HTTP-тела до завершения cancel, затем
отправляет поздний ответ и проверяет прежний snapshot и revision. CAS остаётся
покрыт существующими тестами; новая матрица его не дублирует.

Команды выполняются Rust 1.85.1, jobs=1, через with-rust-slot под BSD flock
собственного transient root. Логи вне TTL:
`/home/myrs/projects/yonik/local/validation/refresh-contract/`.

## Результат

`cargo +1.85.1 test --locked -p yonik-linux --test subscription_refresh_contract`:
3 passed, 0 failed, 0 ignored. `cargo +1.85.1 fmt --all -- --check`: PASS.
`cargo +1.85.1 clippy --locked -p yonik-linux --test subscription_refresh_contract -- -D warnings`:
PASS. Это адресные проверки; workspace/внешняя сеть не запускались.
Подтверждённого production-дефекта нет, исправления fetch не потребовались.
Первый компиляционный прогон выявил отсутствие Debug у RefreshResult в тестовом
expect_err; тест исправлен явным match. Это не RED production-регрессии.
