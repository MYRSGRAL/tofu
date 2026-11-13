# FR07: структурированный ручной ввод Linux

`ManualEntry.qml` встроен в окно добавления источника. Шесть протоколов:
VLESS, VMess, Trojan, Shadowsocks, SOCKS5, HTTP. Имя, endpoint, порт,
UUID/credentials и шифр вводятся отдельными полями. Для VLESS/VMess/Trojan
доступны TCP, WS (path, Host), gRPC (service_name); TLS предоставляет SNI и
ALPN. HTTP допускает TLS. Trojan требует TLS; VLESS к DNS-имени или публичному
адресу требует TLS согласно ограничениям Xray. Plaintext VLESS форма допускает
на private/link-local IPv4, IPv4 loopback и ::1. Shadowsocks предлагает AEAD AES-128/256-GCM и
ChaCha20-IETF-Poly1305. REALITY, дополнительные options и AEAD-2022 форма
не предлагает.

Форма создаёт один sing-box outbound через JSON.stringify с закрытым набором
полей. Callback `submitted(name,text)` вызывает существующий `import_text`.
Далее работают parent local review, явный approve, verified format4 backup,
encrypted repository и ServerIntent. Новых backend-команд нет.

Порт ограничен 1–65535, UUID имеет канонический формат, имя ограничено
128 символами, endpoint и SNI исключают URL delimiters и whitespace,
остальные строки ограничены и исключают управляющие символы. Скрытый ввод
UUID/credentials исключает predictive input. Закрытие/submit очищает поля.
Ошибки parser и unknown options остаются в существующем fail-closed пути.

## Воспроизводимые проверки

- `tests/manual_qml/tst_manual.qml`: независимые точные assertions полей всех
  протоколов, TLS/WS/gRPC, callback/clear, invalid UUID/port, RU/EN.
- `tests/manual_entry_smoke.py BINARY OUTPUT`: настоящие Qt input/click,
  12 случаев (протоколы и TLS/WS/gRPC) → preview counts → backup/approve → повторное открытие;
  проверка redaction trace/stderr.
- `tests/manual_native.rs`: 12 native случаев → verified backup → encrypted
  save → close/reopen → ServerIntent → оба pinned core config checks.
  Запуск: `YONIK_LOCAL_CORE_ROOT=... cargo test -p yonik-desktop --test
  manual_native -- --ignored` через общий Rust slot.

Постоянное evidence: `/home/myrs/projects/yonik/local/evidence/fr07-manual`.
Android и физические G-UI/G-POWER исключены. Config check не является
доказательством actual traffic; результаты traffic-приёмки учитываются отдельно.

Traffic harness `tests/manual_traffic.py` использует ту же закрытую native JSON
grammar с controlled VLESS TCP и TLS peers для обоих cores. Проверяются TCP/UDP
IPv4/IPv6, отсутствие обхода при остановленном peer, trusted certificate,
отказы при неверном SNI и неизвестном CA. Остальные протоколы и WS/gRPC
покрываются config check, но не actual traffic этим harness.

## Результаты приёмки

- QtTest: 8 passed; протоколы, keyboard selection, точные TLS/transport поля,
  TLS policy, callback, clear, RU/EN.
- Настоящий Qt smoke: 12 случаев с выбором popup mouse controls, вводом полей,
  accepted=1/rejected=0 для каждого preview, явным approve и 12 сохранёнными
  источниками. Первый approve требует verified backup3. После последнего
  импорта maintenance backup закрывает и заново открывает repository; ready
  и выбранный профиль восстановлены. Это reopen внутри процесса: smoke binary
  намеренно генерирует новый случайный ephemeral key при каждом запуске.
- Native matrix: 12 encrypted save/close/reopen и 24 pinned config checks.
  Отдельные assertions сравнивают endpoint, UUID/credentials, cipher, SNI,
  ALPN, WS Host/path и gRPC service_name с константами; roundtrip не служит
  единственным oracle.
- Controlled traffic: Xray и sing-box — TCP/UDP IPv4/IPv6 через VLESS,
  отказ без peer; TLS проходит с доверенным CA и правильным SNI, отказывает
  при неверном SNI и неизвестном CA. Проверка TLS не отключается.
- Backend local: 8 passed, 1 ignored (отдельный old-reader control parent).
- MSRV 1.85.1 GUI/test-support all-targets clippy `-D warnings`, fmt,
  pinned Ruff 0.17.4 check/format-check и RU/EN inventory проходят.
  Системные Qt/GCC headers дают C++ warnings; Rust clippy чистый.

Production CLI не принимает native JSON: traffic использует отдельный
`examples/manual_compile.rs` с production import/ServerIntent/compile API.
Production CLI и backend-команды не изменены. Parent old-reader compatibility
и first-format4 API переиспользуются без изменений; Android/physical acceptance
остаются вне scope. WS/gRPC и остальные протоколы имеют config-check proof,
но actual traffic ограничен указанными VLESS TCP/TLS случаями.
