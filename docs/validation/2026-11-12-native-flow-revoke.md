# Отзыв открытых native flows в Linux

База: a4f253257e1089352dba3806b7a435a05fc7188d, PR #126.
Pinned cores, wire ABI260, serialization и capabilities не изменены.

Xray `guardedSystemDial` теперь передаёт настоящий TCP/UDP socket существующей
Session через OwnSocket до публикации. Возвращённый Close вызывает Release
единственного handle; повторное закрытие после revoke идемпотентно. В sing-box
сохранён существующий socket tracking. Оба consumer проверяют закрытие Session
до нового authenticated lookup, в том числе при регистрации сокета после dial.
Новый слой tracking не добавлен. TUN остаётся в прежней custody Session.

Приватный fixture проходит authenticated HELLO/FEATURES/ATTACH/ACK, затем выдаёт
Known wire replies по LOOKUP. Он существует только в Go test binary; production
issuer W1 не подключён и остаётся fail-closed. Config и environment не создают
trusted context. Environment выбирает только изолированный тестовый launcher.

Приёмка обоих actual cores: четыре одновременно открытых TCP либо UDP loopback
flow передают и получают данные. Отдельные controls cancel, EOF, REVOKE и deadline
закрывают sockets, очищают tracking, запрещают следующий dial/route/DNS guard.
Повторный revoke и Close безопасны; после revoke новые wire LOOKUP запрещены.
Существующие проверки single-FD ATTACH custody и TUN сохраняются.

RED: при удалении Xray socket registration тот же тест получает успешный Write
после cancel (`revoked socket writes`). GREEN использует реальные sockets,
actual dial callsites, authenticated broker и ожидание cleanup без skip.

Evidence вне TTL: `/home/myrs/projects/yonik/local/evidence/revoke-e5a27`.
`accept.sh`, `check.sh`, `final-fixture.sh` содержат команды; `*-red.log`,
`*-green.log`, `*-race.log`, `*-vet.log`, `*-build.log`, `*-final.log`,
`builder.log`, `clippy.log`, `fmt.log`, `ruff.log` — результаты.
Manifest adapter содержит package/callsite/patch SHA256, отдельный acceptance
manifest — hashes binaries и toolchain. Patches применяются к чистым pinned
archives с проверкой каждого manifest hash.

Полная production campaign, Android и физические G-UI/G-POWER отложены.
Fixture доказывает Linux private acceptance, а не production issuer integration.
