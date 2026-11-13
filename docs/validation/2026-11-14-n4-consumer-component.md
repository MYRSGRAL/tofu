# N4: Journal queue → actual Plain consumers

PR175, зависимая база PR171 `ca801bc1b6916522954b6e97a081b305f7b0bfe7`.
Root явно согласовал пункты 1–4 bridge proposal. Production/shared contracts,
Go patches, pins/package metadata и production ELF167 не изменены.

## Результат

В собственном same-UID user/net/mount namespace реальный Journal создаёт
persistent Plain TUN и bound queue. Приватный cfg(test) Session удерживает тот же
Arc<BoundTun>, отправляет actual ATTACH с единственным CLOEXEC SCM_RIGHTS FD.
Go test overlay получает fixture-only bootstrap из живого Session и вызывает
неизменённый private closedAttach: live issuer PID/starttime/pidfd, credentials,
binding, metadata, single FD и ACK проверяются существующим кодом. Готовый
production bootstrap этот путь не открывает.

Оба actual pinned constructors получили имя/MTU из проверенного ATTACH context.
Для Xray и sing-box прошли cancel/revoke/stop/wrongmetadata: всего 8/8 случаев.
Каждый positive case подтверждает kernel TCP/UDP echo через actual core routing
и outbound; затем writer с 32 MiB остаётся заблокированным до terminal, завершается
с ошибкой после него, следующий UDP payload не проходит. Wire REVOKE отправляет
Rust fixture issuer; cancel закрывает helper Session; stop закрывает actual core.

Wrongmetadata — live MTU меняется с 1400 на 1500 после binding: closedAttach
отказывает до вызова constructor. Это scoped metadata control, не полная negative
admission matrix. Существующие 153/157 kernel controls и 156 constructors отдельно
не перезапускались; новые 16 probes не вызывались.

Go FD count возвращается к baseline во всех 8 случаях; pipeline проверяет
отсутствие активных core TUN reader/copy goroutines. Rust ждёт подтверждённый exit
child, проверяет original queue и возврат Arc count, освобождает последний owner,
проверяет FD baseline и только затем удаляет TUN через Journal. Timeout вместо
terminal запрещён assertion. Используются отдельные child processes: длительный
resource plateau и production Backend stop admission здесь не заявляются.

## Проверки и binding

MSRV Rust 1.85.1, locked, jobs=1 через общий two-slot wrapper, внешний target под
flock task root. Scoped strict Clippy --all-targets --no-deps -D warnings PASS.
15 existing Session tests PASS. Cargo fmt, Ruff check/format, bash -n и diff check
PASS. Go test ELF собраны с -p=1, -mod=readonly, -trimpath, -buildvcs=false и
отдельными test overlays; оба kernel pipelines PASS.

Evidence вне TTL:
`/home/myrs/projects/yonik/local/orchestration/evidence/n4-consumer-175/`.
Там raw runtime/build/check logs, exact commands, overlay maps, source manifests,
accepted167 manifest hashes, source.patch и retained test ELF. Native patch,
callsite hashes и go.mod/go.sum сверены с accepted full-pinned-158 manifests167.
Первый связанный запуск тоже прошёл; окончательный добавляет Go FD baseline и
берёт имя/MTU из actual ATTACH. Production ELF167 сохранены отдельно, повторная
production сборка не выполнялась. После сохранения ELF и source manifests
проверено отсутствие открытых ссылок на task tree; cargo clean и go clean
выполнены, собственный transient root удалён.

SHA256 окончательных test ELF:

- Rust: `a9df451df984da8e897bf2af5d75efb4a351efee2e36d7e55e932488862b5216`.
- Xray: `40dfe2ef127bd68816b48d10c9ec8c342e9394f4988b482122e2465c60d9da06`.
- sing-box: `84ca5455f918241c2341347d02528e0525a30687a25d67d284870c7df6249143`.

## Пределы и NEXT

Это component consumer witness с fixture issuer и non-app routing, без fake
Known proof. Full production Session factory/bootstrap admission/mint остаются
закрытыми. Не заявляются full N4 RunV2 admission, app-owner broker matrix,
foreign/cross-UID, installed-systemd/rejected campaign или Vnet acceptance.

NEXT: независимый review test bridge и evidence; затем root определяет отдельный
production admission leaf с настоящим issuer/owner context. Ни этот witness, ни
production ELF167 сами по себе такой authority не создают.
