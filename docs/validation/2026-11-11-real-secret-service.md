# Настоящий Secret Service: VM-проверка credential lifecycle

База: `6cf65a4cf9760a7df68b8eee10f012770b38604d`.
Это частичная проверка P06/credentiallife, не закрытие Linux gap.

Стенд: собственная одноразовая QEMU VM, пользователь fixture, собственные
XDG storage/runtime и control socket. Private dbus-daemon не содержит service
каталогов; до запуска daemon имя org.freedesktop.secrets отсутствует.
Production DesktopHost использует обычный HostConfig, без with_ephemeral_key.
Пароль создаётся в VM в файле 0600 и передаётся daemon через stdin.
В stdout попадают только allowlist статусы и enum CredentialError; ключ не
печатается и не хешируется.

База VM — фиксированный официальный Arch cloud image 20261101.620452.
Runner проверяет SHA256 образа, ключа и подписи по существующему provenance,
VALIDSIG с primary fingerprint 1B9A16984A4E8CB448712D2AE0B78BF4326C6F8F.
Cache удерживается shared BSD lock и не изменяется. Guest-only установка
требует полной pacman -Syu транзакции: частичная -Sy конфликтует с переносом
OpenSSL в openssl-3. На хосте пакеты не устанавливались.

## Наблюдения

Первый свежий запуск: create → shutdown ACK → reopen одной БД через настоящий
Secret Service прошёл (118 и 41 мс). Последующий запуск сохранённого keyring
дал CredentialError::Locked. Отдельные negative gates при недоступном сервисе
и locked коллекции прошли: 18–41 мс, shutdown ACK не более 5 секунд,
SHA256 файла subscriptions.sqlite до и после совпадает.
Это hash файла БД, не ключа. Последний диагностический artifact отдельно
содержит expected/passed для каждого шага; неуспешные positive gates не скрыты.

Стенд требует Locked=false до positive gate, поскольку владение именем
D-Bus само по себе не подтверждает завершение unlock. Дополнительно проверяется
явный unlock через собственный control socket после регистрации сервиса.

Evidence вне TTL: `/home/myrs/projects/yonik/local/validation/credentiallife/`.
`manifest.json` связывает базу, исходники и бинарник до запуска VM;
`actual-service.json` содержит завершённые gates, `partial-service.json` —
последний прерванный прогон. Старые файлы evidence не являются результатом
последнего прогона: сверять имя соответствующего vm-*.log и manifest.

## Проверки и оставшиеся границы

Rust 1.85.1 --locked: example build, scoped clippy -D warnings, fmt check;
private-bus credentials_bus: 4 PASS. Python: py_compile PASS.
Ruff 0.16.6 из существующего uv cache: format/check PASS; host-install не выполнялся.
CI использует Ruff 0.17.4; равенство версий не заявляется.
Тяжёлые Rust-команды используют with-rust-slot и один transient target.
Production credentials/backend не изменялись.

Не подтверждены: сохранённый зашифрованный профиль при reopen, успешный unlock
после service/session restart, замена owner во время активного запроса,
creation race без дублирования, аппаратный login/PAM и KDE Wallet.
Наличие пустой открытой БД само по себе не доказывает сохранение master key:
для следующего positive gate требуется сохранённый зашифрованный fixture record.
Нельзя объявлять эти пункты пройденными по данному стенду.
