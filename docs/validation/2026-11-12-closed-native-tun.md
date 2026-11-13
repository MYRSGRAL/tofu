# Закрытая передача TUN в pinned native cores

Scope: Linux, Xray `a4a3dff5b20c2faae567b742c172c7b95e75c083` и
sing-box `c005f35300ce76427dc285481ec5695f5b179cd2`.
Rust, frozen wire ABI, registry pins и capabilities не изменены.

`closedAttach` — приватный consumer закрытого worker handoff. Он принимает
exact192B BOOTSTRAP и два inherited FD, проверяет live pidfd/starttime и
SCM_CREDENTIALS, связывает HELLO с session/generation/invocation, отправляет
FEATURES, принимает exact-one-FD ATTACH и отправляет ACK после проверки TUN.
Config, JSON и environment не могут создать его private context key.
Production issuer W1 ещё не подключён: обычный запуск остаётся fail-closed.

Проверка custody использует TUNGETIFF с exact IFF_TUN|IFF_NO_PI, имя, ifindex,
MTU, CLOEXEC и O_NONBLOCK. Последний обязателен для прерываемого native I/O;
consumer не меняет status flags общего с helper open file description.
Полученный SCM_RIGHTS duplicate удерживается сессией. Перед constructor создаётся
CLOEXEC duplicate, исходный received FD закрывается, а native handle атомарно
регистрируется для revoke. Helper сохраняет свой original. Любая ошибка
constructor, EOF, unsolicited сообщение (включая REVOKE), смерть server anchor
или отмена context закрывает session custody. Повторная инъекция запрещена.

В sing-box actual Start вызывает `NewOwnedInterface`, который устанавливает
`sing-tun.Options.FileDescriptor > 0` и `GSO=false`. В закреплённом sing-tun
`v0.9.7-0.20261023115432-040db71fcd33` поля
`PlatformInterfaceFileDescriptor` нет. ExternalConfiguration обязателен:
он запрещает изменение адресов/routes/DNS при native Close. Проверки существующей
external configuration сохраняются. Native fallback в Start запрещён.

Xray Handler.Start передаёт свой context через Linux `newWorkerTun` в actual
`NewTun`. `CarryClosedSession` сохраняет private key при upstream detach context;
положительный constructor test проходит через тот же перенос. Конструктор больше не использует `openFromEnv` как authority и не
создаёт TUN при отсутствии closed context. LinuxTun получает `ownsTun=false`;
обёртка ограничивает Close session custody и не предоставляет интерфейс
ConfigureSystemDNS. Не-Linux пути оставлены исходными.

Дополнительно исправлен early deny RoutePacketConnection: он закрывает
принадлежащий этому entry point PacketConn. Остальные entry points сохраняют
прежнюю модель владения. Ненулевой PacketConn со счётчиком Close воспроизводит
RED `ErrDenied/closes=0`; GREEN требует ровно одно закрытие.

## Воспроизведение

Применить обычную цепочку adapter patches и native-broker-codec.patch к pinned
исходникам; module digests должны совпасть с source-lock.json. Production builder
проверяет hashes патча, package и callsite файлов и запускает обычные Go tests.
Для положительных тестов требуются доступные unprivileged user/net namespaces
и `/dev/net/tun`. Из каталога каждого patched core:

```sh
export YONIK_HOST_NETNS="$(readlink /proc/self/ns/net)"
export YONIK_ISOLATED_TUN_TEST=1
/home/myrs/projects/yonik/local/orchestration/with-rust-slot \
  unshare --user --map-root-user --net \
  env GOMAXPROCS=1 go test -p=1 -mod=readonly -timeout=60s \
  ./common/yonikbroker -count=1 -v
```

Fixture проверяет отдельную netns, создаёт только собственный bound TUN,
использует frozen wire issuer и передаёт SCM_RIGHTS duplicate. Проверяются
cancel/EOF/REVOKE, неверные metadata, не-TUN FD, отсутствие handoff,
одноразовость передачи, сохранность helper original и actual native constructors.
Тесты без isolated launcher явно пропускают положительные cases.

Evidence этой итерации: `/home/myrs/projects/yonik/local/evidence/tun-e5a27`.
Там сохраняются логи, source/patch manifest, бинарники и launcher вне TTL.
Это private fixture acceptance, не production/helper end-to-end acceptance.
After-DNS, before-dial и full-flow revoke остаются следующими обязательными
задачами; routing-entry guard не заменяет эти стадии. Android, G-UI/G-POWER и
физическая приёмка не входят в эту проверку.
