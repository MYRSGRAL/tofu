# N4 component consumer witness

Только same-UID fixture issuer в собственных user/net/mount namespaces.
Rust test `real_journal_actual_consumer` создаёт persistent Plain Journal-owned
BoundTun, удерживает original Arc и передаёт реальный ATTACH через Session. Go
получает 192-byte fixture bootstrap, SEQPACKET endpoint и live pidfd; существующий
private closedAttach проверяет credentials/binding/metadata/single CLOEXEC FD/ACK.
READY синхронизирует установку SO_PASSCRED до первого HELLO.

Go overlays добавляют только `_test.go` в common/yonikbroker каждого pinned core.
Они используют существующие actual pipelines; имя и MTU берутся из проверенного
ATTACH context. Production patches, source-lock и package metadata не меняются.
Готовые production ELF167 не являются этими test ELF.

Восемь случаев: cancel/revoke/stop/wrongmetadata для Xray и sing-box. Positive
случаи проверяют TCP/UDP echo, удержанный 32-MiB outbound writer и прекращение
traffic после terminal. Revoke приходит helper→consumer по настоящему wire;
cancel прекращает helper Session. Stop закрывает actual core, затем fixture
cleanup отзывает Session. Wrongmetadata меняет live link MTU после binding и
проверяет отказ closedAttach до constructor. Caller TUN FD не принимается.

Go проверяет возврат FD к baseline после cleanup; existing pipeline проверяет
отсутствие активных core I/O goroutines. Rust ждёт exit child, проверяет original
owner, FD baseline, затем отпускает последний Arc и удаляет Journal TUN. TTL
не принимается вместо terminal. Каждый случай использует отдельный child:
это scoped lifecycle acceptance, не длительный plateau test.

## Воспроизведение

Создать собственный transient root и удерживать flock его корня во время всей
работы. `build.py --root <root> --output <evidence вне TTL>` клонирует pinned
sources, применяет accepted patches, проверяет hashes и строит отдельные Go test
ELF с `-overlay`. Сохранить source/overlay manifests и binary hashes.

Rust test ELF собрать `cargo test --locked -p yonik-linux --lib --no-run -j1` с
CARGO_TARGET_DIR внутри того же task root, через общий with-rust-slot. Скопировать
ELF в evidence до очистки. Запуск:

```sh
flock -s "$task_root" tests/isolatedfixture/n4/run.sh \
  "$evidence/linux-consumer.test" \
  "$evidence/xray-consumer.test" \
  "$evidence/sing-box-consumer.test"
```

run.sh запускает только точное имя ignored N4 test. Он не запускает новые 16
probes. После сохранения evidence проверить отсутствие процессов/открытых
ссылок и выполнить cargo clean и очистку собственного Go cache/task tree.

Full Session factory, production bootstrap/mint и app-owner broker admission
остаются закрытыми. Fake Known proof отсутствует: используется non-app routing.
Cross-UID, installed-systemd/rejected campaign и Vnet acceptance не выполняются.
