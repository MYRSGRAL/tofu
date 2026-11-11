# P04: native sing-box JSON normalization

Импорт извлекает vless, vmess (alter_id=0), trojan, shadowsocks, http и socks
(version=5). Подмножество transport: отсутствие transport (TCP), ws и grpc.
TLS: enabled, server_name, alpn и insecure=false. Неизвестные поля, настройки
другого протокола, нестандартные transports, insecure и непредставимые опции
блокируют ServerIntent через review flags. Другие секции полного конфига также
требуют review: импорт не воспроизводит routing, DNS или inbounds.

Исходный документ сохраняется побайтно в original/native_json; source каждого
профиля содержит исходный outbound. Лимиты импорта и capabilities не изменены.
Новых зависимостей нет.

Fixture: crates/yonik-import/tests/corpus/singbox/six-protocols.json.
Проверки: Rust 1.85.1, cargo test -p yonik-import, cargo clippy -p yonik-import
--all-targets -- -D warnings, cargo fmt --all -- --check.

Приёмка использует библиотечный путь import_bytes → ServerIntent → JSON round-trip
→ into_server → yonik_engine::compile для каждого из шести fixture outbounds и
обоих cores, затем xray run -test -config и sing-box check -c.
Существующий CLI yonik-compile отдельно запрещает native_json; его контракт
не изменён в этом PR. Сетевые подключения к синтетическим endpoints не выполняются.
Android и физические G-UI/G-POWER вне этой проверки.
