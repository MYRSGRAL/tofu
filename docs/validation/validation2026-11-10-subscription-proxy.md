# Явный SOCKS endpoint загрузки подписки — 2026-11-10

База: dd6962e7618d688660927bc58e2694940eba5598. Область: FetchHost и отдельный subscription_proxy fixture. Backend ещё должен передавать endpoint активной сессии; полный Pxx PASS этим срезом не заявляется.

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
