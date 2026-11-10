# Явный SOCKS endpoint загрузки подписки — 2026-11-10

База fetch-core: dd6962e7618d688660927bc58e2694940eba5598; независимый fetch-core commit: bf98e984b691da9290245299c9e4f52bb6a82c94. Итоговая область PR: FetchHost, SOCKS fixture и desktop manual/scheduled wiring подтверждённой сессии. Полный Pxx PASS этим срезом не заявляется.

`FetchHost::new(timeout)?.with_socks_endpoint(SocketAddr)?` выбирает loopback SOCKS5 listener явно. Нулевой порт и непетлевые адреса отвергаются. Без builder Proxy возвращает прежний UnsupportedPeerVerification. Direct сохраняет прежнюю проверку peer. System proxy/env не используются, fallback отсутствует.

В reqwest 0.12.28 проверены connect.rs: connect_socks выбирает DnsResolve::Local для socks5; socks::connect вызывает переданный resolver.http_resolve и передаёт выбранный IP в SOCKS CONNECT. async_impl/client.rs передаёт resolver с resolve_to_addrs в set_socks_resolver. Поэтому проверенный набор DNS закрепляется для каждого hop; socks5h не применяется. DNS выполняется локально через существующий системный lookup, это не обещает скрывать DNS от локального resolver. Имя назначения не отдаётся SOCKS для повторного разрешения; HTTP Host/TLS SNI сохраняют имя исходного ресурса.

Для Proxy TCP peer проверяется против выбранного loopback endpoint. Удалённый peer нельзя независимо наблюдать через SOCKS: исполнение CONNECT доверяется активной локальной сессии, а HTTPS проверяет сертификат исходного имени. Публичные адреса валидируются до CONNECT, на каждом redirect; private destination требует Source::local. HWID остаётся в исходном origin, validators привязаны к точному URL и HWID.

Fixture реализует SOCKS5 negotiation и CONNECT, проверяет IP/порт, возвращает HTTP body, проверяет отсутствие Direct accept. Отдельно покрыты auth-required, закрытый порт, timeout, cancel, HWID cross-origin redirect и public/private policy. Все операции локальны; пользовательские сеть и keyring не меняются.

Evidence: /home/myrs/projects/yonik/local/validation/subscription-proxy. RED: без передачи endpoint загрузка завершилась UnsupportedPeerVerification; GREEN включает явный endpoint.

Проверки через with-rust-slot, task-root flock, CARGO_TARGET_DIR и jobs=1:

- `cargo test -p yonik-linux --test subscription_proxy`: 5 passed.
- `cargo test -p yonik-linux --lib --tests`: 440 passed, 27 ignored; 0 failed.
- `rustfmt --edition 2024 --check` для обоих изменённых Rust файлов и `git diff --check`: PASS.
- `cargo clippy -p yonik-linux --all-targets -- -D warnings`: BLOCKED существующим clippy::collapsible_match в crates/yonik-engine/src/lib.rs:165 на toolchain 1.97. Этот файл вне ownership и не изменялся.
- `cargo clippy -p yonik-linux --all-targets --no-deps -- -D warnings`: PASS. Проверены все targets изменяемого пакета без lint зависимостей.

## Сквозная desktop интеграция

Desktop backend принимает и сохраняет `route=Proxy`. Endpoint берётся из immutable CompiledConfig запущенной сессии и допускается только после Connected от LocalSession::check. Выбранный UI порт и Preparing/Checking/Limited не подтверждают готовность. Каждому запуску назначается поколение; stop, выход процесса и потеря Connected отзывают lease. Переполнение поколения запрещает новый запуск.

DesktopFetch фиксирует lease на manual operation или scheduler dispatch. Direct сохраняет прежний путь. Proxy проверяет lease перед подключением, отменяется при изменении watch и повторно проверяет поколение после fetch. Backend отменяет старую manual operation с существующим acknowledgement, удаляет устаревший review; scheduler при отзыве использованного proxy lease завершает shutdown с ACK и восстанавливает schedule из repository. HTTP/DNS/TLS защита FetchHost не менялась в desktop delta. SubscriptionService, его CAS и review/approval контракт не изменялись; новый bridge в subscriptions.rs не требуется.

Проверки desktop delta:

- `cargo test -p yonik-desktop --lib --test backend`: 19 unit + 33 integration passed, 1 native integration ignored по умолчанию.
- `tests/proxy_native.py <backend-test-binary> /home/myrs/projects/yonik/local/engines`: native integration выполнен отдельно для sing-box 1.14.4 и Xray 26.10.31, оба PASS. Настоящий DesktopHost работает с private D-Bus, настоящими pinned cores, VLESS loopback peer и TLS endpoint с отдельным CA через SSL_CERT_FILE только дочернего процесса. Fixture повторно использует tools/engine_host.py::peer/https_server (provenance: configurable-health validation 2026-11-09). Ни host trust store, ни пользовательский keyring, ни маршруты/DNS хоста не меняются.
- Native сценарий: bootstrap Direct → Connected → manual Proxy импорт; stop → Proxy отказ без новой HTTP попытки; новый host/новая подтверждённая сессия → scheduled Proxy review → discard; удержанный manual Proxy HTTP запрос → stop → Disconnected, busy=false, отсутствует stale pending.
- Unit regression: смена поколения при том же endpoint отклоняется до accept; stop отменяет ожидание SOCKS handshake.
- `cargo fmt --all --check`, Python ruff format/check и diff-check: PASS.

Desktop evidence и SHA256 обоих ядер: /home/myrs/projects/yonik/local/validation/subscription-proxy/desktop. Предыдущие 440 Linux regression tests относятся к независимому fetch-core commit bf98e984b691da9290245299c9e4f52bb6a82c94; desktop delta его не меняет. Этот срез не объявляет полный Pxx PASS.
- `cargo clippy -p yonik-desktop --all-targets --no-deps -- -D warnings`: PASS на итоговом desktop delta.
