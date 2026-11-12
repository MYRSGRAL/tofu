# Отмена настоящего health probe

Узкий scope FR10/P05: LocalSession::check при запущенном pinned Xray 26.10.31
и sing-box 1.14.4. Dependency PR129 включён обычным merge; production backend,
helper, domain, ABI и pins не изменены.

Новый example health_probe_cancel вызывает настоящий check. Контролируемый
upstream получает SOCKS greeting от ядра, удерживает соединение без ответа и
только затем подаёт Cancellation. Проверяется exact HostError::Cancelled,
прежний PID и состояние Checking согласно actual contract. Затем явный Stop,
Disconnected и исчезновение PID/starttime. Future check не выбрасывается владельцем
для имитации отмены.

Контроли для каждого core: настоящий HTTPS 204 через core и SOCKS peer;
отказ upstream с exact Transport и Limited. TLS fixture использует однодневный
локальный CA и подписанный им leaf-сертификат с SAN IP 198.18.0.1. SSL_CERT_FILE/SSL_CERT_DIR
задаются только окружению стенда, host trust store не изменяется. Peer перенаправляет
соединение к локальному TLS fixture, внешней сети нет.

Дополнительно DesktopHost через публичные start/dispatch/callback выполняет
add_source, refresh, select_profile, set_probe_target, Connect. После реального
health greeting и snapshot Checking подаётся Stop, ожидается Disconnected,
затем shutdown/wait_closed. Сохраняется фактический snapshot. В публичном контракте
Stop отсутствует request_id ACK: привязанный к запросу ACK и exact Cancelled через
DesktopHost остаются gap. Стенд не выдаёт callback snapshot за такой ACK.
Exact cancellation и сохранность active session доказуются компонентной осью;
DesktopHost Stop по контракту завершает session.

tools/health_probe_cancel.py использует исправленный zombie-aware oracle PR129.
Каждая ось выполняется в private user/net/PID/proc/D-Bus, только loopback,
не затрагивает host routes/DNS/keys. По всем новым процессам сохраняются FD,
threads, PSS, private dirty и PID/starttime. Terminal owned group обязана быть пуста.
Контроль удержания собственного процесса проверяет память >=8 MiB и открытый FD,
после terminate/wait группа снова пуста. Это bounded terminal cleanup proof;
долговременный memory plateau и GUI/physical/Android не проверяются.

Воспроизведение: Rust 1.85.1, with-rust-slot, jobs=1, внешний CARGO_TARGET_DIR
под flock собственного transient root; собрать -p yonik-desktop --example
health_probe_cancel. Затем python tools/health_probe_cancel.py ABS_BINARY
ABS_CORE_DIR ABS_NEW_EVIDENCE_DIR. Binary/source/core/hash и raw evidence сохраняются
в local/validation/health-cancel данного worktree вне TTL. Временный TLS private key
создаётся fixture и не является ключом пользователя; не добавляется в git.


Фактическая приёмка attempt-2: **8/8 PASS**; оба exact Cancelled, оба HTTPS
204, оба Transport при отказе peer и оба DesktopHost Stop → Disconnected.
Все terminal_owned_group пусты; итог owned_resources_zero=true.
Negative control: 4 FD, private dirty 11739136 bytes; после reaping группа пуста.
Первый attempt отклонён TLS: CA использовался как leaf; fixture исправлен,
production TLS validation не отключалась. Raw failed evidence сохранён.

Rust MSRV 1.85.1: host unit tests — 38 passed, 3 ignored; clippy
-p yonik-desktop --all-targets -- -D warnings и fmt --all -- --check — PASS.
Три Python regression tests zombie/identity/retainer — PASS; Ruff check/format — PASS.
Clippy первоначально отклонил неучтённую длину read в HTTP fixture; исправлено
ограниченным чтением заголовков перед финальной сборкой.
