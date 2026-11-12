# Ручной профиль через encrypted repository и Connect

Leaf от `5a22ecf8a63a99828d31a66cce52354f9ea3722a`, первоначальная база PR —
`t3/manual-entry-native-traffic` (#139); root переносит базу после parent merge.

`manual_encrypted_connect.py` повторно использует настоящий ввод полей
`manual_entry_smoke.py`. Для каждого отдельного профиля выполняются submit,
review, verified backup, approve и завершение GUI. Новый процесс открывает ту же
БД, выбирает профиль через `profileSelect_1` и нажимает `serversConnectButton`.
JSON сигнала формы и manual-compile не используются в Connect.

Существующий executable `tray_cleanup_negative --editor-persistence` запускает
production GUI/DesktopHost с fixture key `[43; 32]`. Это изолированный синтетический
ключ для encrypted repository; production custody через Secret Service здесь
не проверяется. Production contracts, parser, backend, manual UI, pins и ABI
не меняются. Retained обычный GUI создаёт случайный ключ на каждый smoke запуск,
поэтому для двух процессов необходим этот существующий test-support executable.

Scope: VMess WS/TLS и Trojan gRPC/TLS на pinned Xray 26.10.31 и sing-box 1.14.4.
Для каждого — actual TCP echo и peer с неверными credentials при доступном echo
endpoint. Пассивный LD_PRELOAD observer сохраняет actual JSON write в pipe после успешной
исходной write, не меняя байты или результат syscall. Oracle проверяет эти
одинаковые check/run configs собственного DesktopHost и hash
исполняемого memfd ядра, endpoint/protocol/credentials/transport/TLS. TLS доверяет
только локальному синтетическому сертификату; skip verify не используется.
После GUI Stop требуется Disconnected/can_stop=false, terminal closed, отсутствие
PID ядра и повторный bind SOCKS-порта. Peer завершается с wait, echo threads joined.

Evidence сохраняется вне TTL:
`/home/myrs/projects/yonik/local/evidence/manual-encrypted-connect`.
Сырые конфигурации и приватный TLS key остаются только в собственном transient root.

Команда запуска под flock task-root:

```sh
unshare --user --map-root-user --net dbus-run-session -- \
  python crates/yonik-desktop/tests/manual_encrypted_connect.py \
  "$evidence/tray_cleanup_negative" /home/myrs/projects/yonik/local/engines \
  "$task_root" "$evidence"
```

22-case traffic matrix #139 здесь не повторяется. UDP, Android и физические
G-UI/G-POWER в scope не входят. Этот checkpoint не заменяет production key custody
acceptance или полную transport/cipher/ALPN матрицу.

Результат: 4 положительных echo + 4 peer-sensitive отказа, все 8 запусков
завершились closed/reaped с освобождением SOCKS listener. `result.json` содержит
PID и SHA-256 actual writes, ciphertext DB и core для каждого запуска.
Pinned Ruff 0.17.4 check и format --check прошли; observer собран с
`-Wall -Wextra -Werror`. MSRV 1.85.1 all-targets clippy `-D warnings` и fmt
переиспользованы из source-identical #135 (`retained-*.log`); новый Cargo запуск
потребовался только для существующего persistence executable. Обычный retained
GUI и ядра не пересобирались. Persistence fixture не поддерживает screenshot path:
в этой приёмке сохраняются trace, а capture-маркеры общего helper исключены.
Перед штатным cargo clean external refs отсутствовали; собственный TTL root удалён.
