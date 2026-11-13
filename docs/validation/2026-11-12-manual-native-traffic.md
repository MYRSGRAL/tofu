# Ручная форма: actual traffic Linux

Leaf от `7ff75c5405ac669f1bd56fa9ac5e657bc486e154`, base PR #135.
Production Rust/QML, parser, backend, pins, capabilities и ABI не меняются.

`tests/manual_traffic_qml/tst_fixtures.qml` заполняет настоящую форму
`ManualEntry.qml`, вызывает submit и получает JSON из сигнала `submitted`.
Это не положительная инъекция в backend ManualEntry. Синтетические credentials
публичны. `tests/manual_native_traffic.py` передаёт неизменённый JSON сохранённому
`manual-compile`: production import → ServerIntent → compile.
Native-конфигурация проверяется отдельным oracle с константами endpoint,
credentials и transport. Controlled sing-box peer строится независимо от
скомпилированной конфигурации клиента; оба pinned клиента подключаются к нему.

Матрица для каждого из Xray 26.10.31 и sing-box 1.14.4:

| Протокол | Транспорт | TLS |
| --- | --- | --- |
| VLESS | WS, gRPC | да |
| VMess | TCP; WS, gRPC | нет; да |
| Trojan | TCP, WS, gRPC | да |
| Shadowsocks | TCP (AES-128-GCM) | нет |
| SOCKS5 | TCP с username/password | нет |
| HTTP | CONNECT с Basic credentials | да |

Итого 11 положительных TCP echo на ядро. Каждый запрос использует случайную
нагрузку и точное сравнение ответа. Для каждого случая peer повторно запускается
с неверными credentials: echo обязан отказать. Echo server остаётся доступен,
поэтому direct bypass дал бы ложный успешный ответ и провалил проверку.
Для восьми TLS случаев на ядро дополнительно проверяются неизвестный CA и
неверный SNI. Проверка сертификата никогда не отключается. Общий результат:
22 положительных и 54 отрицательных проверки.

Новый `--controlled-transports` в Qt smoke сохраняет через настоящий ввод,
review, verified backup и approve две записи: VMess WS и Trojan gRPC. Endpoint,
UUID/password, SNI, WS path/Host и gRPC service совпадают с traffic fixtures;
ALPN пустой. Maintenance backup выполняет close/reopen. Traffic получает JSON
сигнала формы, а не расшифрованную запись этого smoke. Предыдущие 12 encrypted
roundtrip и 24 config-check остаются отдельным checkpoint, повторно не выполнялись.

Команды (из worktree, под flock собственного transient root):

```sh
python crates/yonik-desktop/tests/manual_native_traffic.py \
  /home/myrs/projects/yonik/local/evidence/fr07-manual/manual-compile \
  /home/myrs/projects/yonik/local/engines "$task_root"
unshare --user --map-root-user --net dbus-run-session -- \
  python crates/yonik-desktop/tests/manual_entry_smoke.py \
  /home/myrs/projects/yonik/local/evidence/fr07-manual/yonik-desktop \
  "$evidence/qt-save" --controlled-transports
```

Harness сам создаёт отдельный user/network namespace, поднимает только lo;
ключи TLS и конфигурации находятся в собственном transient root.
Qt save запускается в отдельном namespace и DBus session; keyring не используется.
Постоянное evidence: `/home/myrs/projects/yonik/local/evidence/manual-entry-native-traffic`.
Retained binaries и исходные MSRV 1.85.1 all-targets clippy/fmt evidence:
`/home/myrs/projects/yonik/local/evidence/fr07-manual`. Rust source неизменён,
повторные Cargo build/clippy не нужны для harness-only leaf.

Матрица покрывает все шесть протоколов и обе доступные transport-ветви каждой
из трёх форм с transport. Она не заявляет полный cross-product ciphers, ALPN,
HTTP plaintext/no-auth или UDP. VLESS TCP/TLS уже принят parent и здесь не повторяется.
WS query и gRPC multi_mode не добавлены и не объявляются поддержанными.
Android и физические G-UI/G-POWER отложены.
