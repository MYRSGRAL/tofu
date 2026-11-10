# Управляемая экстракция VMess и Trojan из Xray JSON

Дата: 2026-11-10. Срез Linux/general parser; это не приёмка всего P04.

## Основание и границы

Фактический base worktree: `e33fa1908dfb3deb5434506341674de0b3a827b4`.
Ветка: `t3/xray-vmess-trojan-parser`. Head реализации: `5539cdfbcfd4004161ee7cc556134cf51cc768ab`.
Финальный head включает следующий документирующий commit с этим отчётом.

Изменены только `crates/yonik-import/src/xray.rs`, новый
`crates/yonik-import/tests/xray_protocols.rs` и этот отчёт.
Модель ParsedServer, URI parser, identity, ServerIntent и engine не менялись.
Зависимости не менялись. CONTRIBUTING.md и дополнительные AGENTS.md в worktree
не найдены; применены инструкции задания и предоставленный AGENTS.md.

Локальные источники формата: `uri.rs::vmess`, `intent.rs::read_vmess`,
`yonik-engine/src/lib.rs::xray_outbound` и `yonik-engine/src/options.rs`.
Требования: `docs/specs/data-and-contracts.md` (входы, нормализация, native JSON,
секреты), `requirements.md` FR-07, `validation-and-roadmap.md` (матрица импорта).
Внешних предположений о расширении формата не добавлено.

## Поведение

- VMess `settings.vnext[].users[]` преобразуется в существующий VMess envelope:
  UUID, security → scy (по умолчанию auto), alterId → aid, endpoint, TLS и transport.
- Trojan `settings.servers[]` использует существующую URI-нормализацию пароля,
  endpoint, security и transport; специальные символы credentials экранируются.
- Для новых протоколов требуется ровно один endpoint и один VMess user.
  Пустой/неоднозначный массив даёт явную ошибку записи; первый элемент не выбирается.
  Существующее поведение VLESS и Shadowsocks не изменено.
- Полный native JSON сохраняется побайтно существующим import_bytes, включая
  routing/listeners и неизвестные опции. Он не исполняется.
- Неизвестные поля на уровнях outbound/settings/endpoint/user/stream отмечаются
  UnsupportedOption. Непредставимые опции VMess envelope (например Reality publicKey,
  grpc authority и allowInsecure) также блокируются через UnsupportedOption.
  Legacy alterId блокируется существующим LegacyOption.
- До извлечения действуют существующие лимиты размера, глубины и уникальности
  декодированных ключей. Endpoint проходит существующую проверку IP/IDNA/порта.
  Debug скрывает source, native JSON и credentials.

## Проверки

Все Cargo-команды выполнялись с Rust 1.85.1, CARGO_BUILD_JOBS=2,
CARGO_TARGET_DIR=/var/tmp/codex-myrs/transient/yonik-xray-protocols/target и
`flock -s /var/tmp/codex-myrs/transient/yonik-xray-protocols`.

Первый тест до реализации: `cargo +1.85.1 test -p yonik-import --test xray_protocols`
завершился exit 101: ожидался один VMess, получено ноль. После реализации
тот же тест прошёл. Дополнительные негативные сценарии проверены вместе с ним.

Новые тесты проверяют нормализацию VMess/Trojan и IDNA, побайтовый native source,
стабильность identity и ServerIntent roundtrip, секреты в Debug, пустые/множественные
endpoints/users, неизвестные и небезопасные поля, malformed credentials/ports/host,
escaped duplicate keys, oversized outbound, depth, IPv6/gRPC и legacy alterId.

Результаты на head реализации:

- `cargo +1.85.1 test --workspace --locked` — exit 0: 50 test/doc-test suites,
  532 passed, 0 failed, 24 ignored (существующие opt-in системные проверки).
- `cargo +1.85.1 test -p yonik-import --test xray_protocols --locked` — exit 0,
  7 passed, 0 failed.
- `cargo +1.85.1 clippy --workspace --all-targets --locked -- -D warnings` — exit 0.
- `cargo +1.85.1 fmt --all -- --check`, `git diff --check` — exit 0.

Первый полный прогон дополнительно выявил ошибку Reality fixture: одновременно
присутствовали tlsSettings и realitySettings с одинаковым SNI. URI parser отклонил
повтор параметра. Fixture исправлен: неактивный TLS-блок убран, добавлено явное
неизвестное Reality-поле; повторный полный прогон прошёл. Production-код для обхода
этого отказа не менялся.

Краткое evidence сохранено в этом отчёте вне TTL build-каталога. Полные локальные
логи: `local/validation/xray-json-protocols-original/workspace-test-final.log`, `protocols.log`,
`clippy-final.log`. Бинарники для передачи не требуются. Перед завершением активных
сборок этого task не осталось; собственный Cargo target очищен штатным cargo clean.

## Остаток и ограничения

Независимое review выявило P2: один native ALPN-токен с запятой превращался
после URI-преобразования в два протокола. Регрессия на VMess/Trojan, один/два
элемента и обычный контрольный список сначала упала на прежнем коде.
Теперь непредставимый ALPN отклоняет managed extraction до склейки; полный
native JSON остаётся сохранённым. Все тесты yonik-import после исправления
прошли. Логи red/green и clippy сохранены в основном workspace:
`local/validation/review51-{alpn-red,alpn-green,clippy}.log`.

Отдельный PR требует независимого review root перед merge. Вложенных агентов нет.
Приёмка всего P04, Android, native config execution, helper, routing и listeners
за рамками среза. Qt GUI не изменялся и feature gui для parser-проверок не включался;
пользовательские данные, keyring, сеть и системные helpers не изменялись.
Real Xray execution/validator и живые удалённые серверы в этом срезе не запускались.
