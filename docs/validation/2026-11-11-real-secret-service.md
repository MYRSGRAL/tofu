# Настоящий Secret Service: подтверждённый VM credential lifecycle

База реализации: `6cf65a4cf9760a7df68b8eee10f012770b38604d`, PR 118.
Проверен настоящий gnome-keyring `1:51.1-1`, dbus `1.16.2-2`, production
DesktopHost с обычным HostConfig и production subscription_master_key.
`with_ephemeral_key` не используется. Production credentials/backend не менялись.

## Изоляция и происхождение

Одноразовая QEMU VM, собственный пользователь fixture, runtime/control socket,
XDG data/config и SQLite. Private dbus-daemon не содержит service directories;
до запуска daemon нет org.freedesktop.secrets. Текущий owner и его Unix PID
сверяются с PID единственного управляемого foreground daemon; после остановки
проверяется отсутствие имени. Адрес bus задаётся только guest-процессам.
Host keyring, host HOME, host packages и сетевые настройки не менялись.

Фиксированный официальный Arch cloud image `20261101.620452` проверен по
существующему provenance: SHA256 образа, ключа и подписи, VALIDSIG с primary
fingerprint `1B9A16984A4E8CB448712D2AE0B78BF4326C6F8F`. Cache read-only под
shared BSD lock; собственный overlay под root-directory BSD lock. Guest-only
pacman -Syu нужен из-за миграции OpenSSL в openssl-3. Rust build выполняется на
хосте через with-rust-slot, с одним target вне checkout; VM/bootstrap слот не держит.

Пароль генерируется только в VM в файле 0600 и подаётся startup daemon через
stdin. Live unlock использует собственный control socket; проверяются тип/UID
сокета, SO_PEERCRED PID/UID, deadline и фиксированный статус ответа. Пароль,
ключ, ciphertext и base64 payload не выводятся. Диагностический DH probe
выводит только PID и counts unlocked/locked/readable/refused. Plain Secret
Service sessions не используются. Hash файла SQLite проверяет неизменность
зашифрованных test data, ключ не хешируется.

## Evidence и результаты

Evidence вне TTL:
`/home/myrs/projects/yonik/local/validation/credentiallife/`.
Исторические failed runs сохранены в корне и `candidate2/`.
`candidate3/manifest.json` и `candidate4/manifest.json` созданы до VM effects:
содержат hashes исходников, probe/host binaries и provenance базы.
`actual-service.json` в каждом candidate содержит публичные результаты.
Candidate3 начинается со свежего keyring/БД. Candidate4 — отдельная узкая
проверка live unlock на сохранённом overlay candidate3, с новыми bus/процессами;
`database_existed_before_run=true` явно отличает её от first create.

| Gate | Реальное наблюдение |
| --- | --- |
| Первый create → shutdown ACK → reopen | candidate3: ready 110/58 мс, один key item |
| Зашифрованный профиль | SubscriptionService импортирует локальную test subscription; offline original совпадает после close/reopen |
| Service restart | новый daemon PID/owner, fresh host ready 67 мс; профиль расшифрован |
| Locked default collection | default и login указывают на `/org/freedesktop/secrets/collection/login`; Locked=true, locked items=1, readable=0; host отказал за 33 мс |
| Unavailable | после остановки имя отсутствует; host отказал за 21 мс; SQLite SHA256 неизменён |
| Live collection unlock | candidate4: owner `:1.26`, PID 553 сохранены; Locked true→false; свежий host PID 603 ready за 37 мс, профиль расшифрован |
| Stale DH session/item после owner change | тот же probe PID 5295: readable=0, refused=1; отдельный fresh production host ready 40 мс |
| Pending production request + owner loss | daemon остановлен SIGSTOP до запроса, затем завершён; host отказал за 2 мс после потери owner, bounded shutdown ACK, SQLite неизменён |
| Session restart | новая private bus, новые daemon/host connections, те же keyring/БД; ready 43 мс, профиль расшифрован |
| Existing DB + missing key | отдельный пустой keyring: keyring_missing за 43 мс, БД неизменна; новый ключ для существующей БД не создаётся |
| Create race | два production credential процесса: Busy/available, после гонки ровно один readable key item |

Все host probes ожидают shutdown ACK не более пяти секунд; ожидание startup
ограничено 22 секундами. Отказы не запускают повтор RPC или ephemeral fallback.
Удерживаемый stale item — диагностический объект зависимости secret-service;
production subscription_master_key создаёт connection/session заново на каждом
вызове. Это различие проверено отдельно от fresh production host.

## Исправленный дефект стенда

В прежнем runner второй `gnome-keyring-daemon --unlock` ошибочно считался
unlock-клиентом существующего daemon. Он мог запустить дополнительный daemon:
в candidate2 после завершения управляемого процесса Secret Service продолжал
отвечать и возвращал ноль items. CLI exit 0 не доказывал unlock. Candidate2 RED:
unavailable и service_restart failed; encrypted profile перестал читаться.

В candidate3 GREEN удалён второй daemon, добавлены owner PID equality и
проверка исчезновения имени; persisted profile/restart/race gates прошли.
В candidate4 live unlock выполнен по control protocol при том же owner,
а не через CLI exit status. Формат protocol сверялся с официальными
[gkd-control client](https://github.com/GNOME/gnome-keyring/blob/main/daemon/control/gkd-control-client.c)
и [control codes](https://github.com/GNOME/gnome-keyring/blob/main/daemon/control/gkd-control-codes.h).
Оснований менять production locked checks, crypto или owner handling не найдено.

## Проверки и границы

Rust 1.85.1 --locked: оба examples build PASS; clippy -p yonik-linux
-p yonik-desktop --all-targets -- -D warnings PASS; fmt check PASS.
credentials_bus: 4 PASS, включая private-bus регрессии. Python tools: 196 PASS;
translation tests/check PASS. Ruff 0.16.6 из существующего cache: format/check
PASS; CI использует 0.17.4. Host install не выполнялся.

Подтверждён VM/private-bus критерий P06/credentiallife. Аппаратный login/PAM,
GUI unlock/cancel prompt, KDE Wallet и интеграция с настоящей desktop login
session этим стендом не проверены. Перезапуск private bus является session
restart стенда, не аппаратным logout/login. Сигнализация потери owner проверена
при pending OpenSession; каждая возможная точка прерывания CreateItem/GetSecret
не инъецировалась. Собственные overlay, cloud-init/SSH keys и target очищаются
после сохранения binaries/manifests/public evidence вне TTL.

## Автоматический gate stale owner

Independent review обнаружил, что owner_changed и stale probe только
записывались. Root добавил обязательный gate для полного режима: реальный
owner должен измениться, PID ранее открытого probe должен сохраниться,
GetSecret должен быть отклонён ровно для одного item и не вернуть читаемый секрет.
Отсутствующие результаты и подмена PID отклоняются. Focus unlock остаётся
отдельным узким режимом и не выдаётся за полную stale-owner матрицу.

Regression RED → GREEN (включая false owner, readable stale item, другой PID
и отсутствующий результат), Ruff PASS. Окончательный verifier offline повторно
применён к сохранённым candidate3/all и candidate4/unlock: PASS. Guest/Rust
не изменены и VM заново не запускалась; это не доказательство запуска всего
итогового fixture head. Исторические различия hashes runner сохранены.
Evidence: `local/validation/credentiallife/stale-gate-{red,green}.log` и
`stale-gate-manifest.json`.
