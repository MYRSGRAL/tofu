# P04: canonical FD admission и helper/worker bundle transport

Подготовленная база `3c0c4e6d062a540cec439cc7d031d1e0696fb3a7` объединяет
PR90 `7fedcd07014b410ecb278b19be58c55d5f2d8775` и PR101
`8b7261ef81978b3dc473b913c0a5ccc92f512cc1`. P2 inventory fix
`12803936f500c3ecb798b9e1843f8fc21ff1fe82` импортирован как `fdc39f0`;
OFD custody fix `79df8539fbe5c1c474be8f3f5a2f2d3e0b73b1ea` импортирован как `43bd261`.
canonical sidecars используют `reserve(bytes, 1)`, reuse — `reserve(0, 0)`.
Исходные encoder и native custody format не менялись.

## Новый RPC и identities

Старые шесть RPC, closed capabilities-v1, signatures и error/owner fences сохранены.
Отдельный `BundleCapabilitiesV1(s)` возвращает closed DTO
`{version:1, canonical_fd_transport:bool}`. Это наличие transport, а не доказательство
native process attribution, DNS provenance или готовности TUN.

`PrepareBundleV1(s, ah)` принимает bounded typed metadata и ≤16 ordered canonical
source FD. Общий body ≤64 KiB. DTO содержит ServerIntent, SystemPolicy, ordered
bindings, revision, engine/pin, closed compile requirements, canonical descriptors
и expected native refs. Artifact/config bodies, caller paths, runtime ports,
probe credentials и asset env/path не принимаются. Signature и transport FD count
проверяются отдельно от manifest count. Неизвестные поля отклоняются.

`yonik-intent-bundle-v1\0` связывает canonical typed server/policy, bindings,
revision, engine/pin, requirements и оба ordered manifests. Helper сам считает SHA
и сравнивает intent. `yonik-runtime-bundle-v1\0` дополнительно связывает actual
config digest, asset strategy, retained runtime-context digest и ordered native
refs. Старый `yonik-bundle-v1` и смысл CompiledResourceBundle не переопределены.

Helper резервирует реальные ports/network plan, сам генерирует private probe
credentials и fresh 16-byte resource session/generation context. Контекст связывает
также unique bus owner, проверенный peer PID/starttime/namespaces и network stamp.
Новый reply содержит one-use handle и intent/config/runtime digests; config body
и probe credentials не возвращаются. Client проверяет intent echo, closed DTO,
sender/current owner. Ни один mutating RPC не повторяется автоматически.

## Admission и native content

Fresh polkit и daemon-derived peer identity выполняются до privileged effects.
Все FD metadata проверяются до первого pread: ordered count, readonly regular
memfd, mandatory seals, exact size, distinct inode/digest и physical reservation.
Затем stream SHA/count и закрытый canonical grammar. Caller hash/count не заменяют
проверку содержимого. JSON/native protobuf extras, regex и compressed payload не
интерпретируются. Rebuild использует прежний encoder, сравнивает exact native
size/SHA/records/kind/version и выдаёт sealed readonly artifacts.

Индексы строятся из тех же canonical bytes с отменой по records. BoundPolicy
проверяет policy identity/revision/bindings; SystemTUN compiler сохраняет старый
control flow, inline OR external и AND между категориями. IpOnly/domain и native
app-attribution gates сохранены. Реальный DNS provenance этим transport не выдаётся.

## Original source и reservations

ResourceCatalog сохраняет exact original source, включая порядок/дубликаты,
отдельным `c-<native digest>.enc`. AAD `yonik-canonical-source-v1\0` связывает
роль/version, native descriptor и bounded source header. Source association
проверяется повторным native encoder SHA/length/count. Старые artifacts без
canonical proof возвращают Unsupported с требованием rebuild из original source.
Native/state schema и atomic current/previous прежние; sidecars учитываются
staging/current/previous/active-quarantine/retained и inventory, автоматического GC нет.

4 MiB config, 8 MiB legacy retention и существующий 64 MiB config pool не повышены.
Отдельный physical-artifact pool — 64 MiB: canonical FD/buffers/indices, helper и
worker native copies, Xray tmpfs copy и explicit native parse/matcher work credit
(1 KiB/record + fixed overhead). Credit удерживается с payload до empty evidence.
Store cap 512 MiB относится к DISK. Canonical persistence ограничена 16 MiB/proof;
128 MiB whole-buffer encoder cap не является обещанием RSS.

## Worker v2 и Xray private tmpfs

Legacy worker v1 unchanged. V2 — typed header, fresh helper resource context и ≤16
native descriptors; SCM roles только `[binary, config, artifact0..N]`. Extra roles,
duplicate digests, mixed engines, wrong count/size/hash/format отклоняются.
Worker делает bounded pread в новый worker-owned memfd0600, проверяет bytes/SHA,
ставит seals, reopen readonly и дублирует в exact slots 32+i. Занятый slot вызывает
отказ с закрытием только собственного duplicate. CLOEXEC снимается только с typed
resource FD перед native exec. Image/control/directory FD не наследуются.

Sing-box renderer использует fixed `/proc/self/fd/N`, `format: source`. Raw pinned
Xray (`cc4b84…`) отверг absolute ext path: `asset path must stay in asset directory`.
Raw FAIL сохранён в `native-fd-gate-diagnostic.log`; adapter `4c31db…` прошёл тот же
FD config-check. Это отдельный loader gate, не production Apply.

Для Xray renderer генерирует relative `ext:r-<digest>.dat:YONIK`. Per-command
XRAY_LOCATION_ASSET задаёт только worker-owned generated root на unit-private
`/dev/shm`, не global env/DTO/ABI authority. Controlled units задают
`rw,nosuid,nodev,noexec,mode=1777,size=64M,nr_inodes=128`. Existing PrivateDevices,
CAP и DeviceAllow сохранены. Новый Xray lane проверяет semantic unit options и
получает actual kernel tmpfs/cgroup proof через NO-FD preflight до FD transfer.
Нет private/bounded tmpfs — отказ, без disk fallback; legacy v1 feature не требует.

Materialization: exclusive0700 role/CSPRNG root; digest-only filenames; bounded
stream/hash/fsync/verify и regular files0400. Это не обещание F_SEAL для filesystem
copy; исходный worker memfd остаётся sealed. Private custody ledger содержит
session/generation/cgroup, role/nonce, root dev/ino/mount и ordered inode/hash/size.
Cleanup до выдачи consumer либо после возврата неуспешного exec удаляет только
known owned entries после проверки всего ledger: dev/ino/UID/nlink, exact bytes,
bounded streaming SHA и неизменные mtime/ctime во время чтения. Подмена оставляет
ledger, данные и reservation в quarantine. Direct-child reap не разрешает cleanup:
после успешного config-check assets остаются issued, включая возможных потомков,
до завершения unit/cgroup. Helper удерживает payload credit до trusted empty proof.
Foreign entry/unknown child → bounded quarantine. Run/crash namespace cleanup
зависит от controlled unit lifetime и подтверждённой пустой cgroup, без name-only GC.

## Проверки и границы

Evidence вне TTL: `/home/myrs/projects/yonik/local/evidence/p04-fd-receiver/`.
MSRV Rust1.85.1, --locked, jobs1, global build slot и root BSD flock.
Receiver/store, typed client/private-bus, worker v1/v2, prepared owner/epoch/auth
и quota/reopen regressions проверяются отдельно от actual VM gate.

Большой component fixture: 100k domains + 50k CIDR, encrypted original source,
sealed export/native rebuild обоим formats, cancellation и metadata negatives.
Это component proof, не domain Apply. Actual CIDR bundle gate обоим cores и
private tmpfs guard/foreign/budget fixture выполняются на frozen candidate в
собственной signed-base VM с real system bus/polkit и production executables.
До результатов VM production PASS не заявляется. Native app AND и Domain+CIDR/
real provenance остаются genuine gates; полного P04 этот slice не объявляет.

### Historical frozen69 и review P2

Frozen `69d7ac4414bd051bd876e28e6f91de8150b4d7e7`: настоящий system bus/polkit,
PrepareBundle/Apply, 50000 CIDR каждому pinned core, TCP/UDP IPv4/IPv6 match/miss,
worker-owned sealed FD32 и private Xray tmpfs/ledger прошли; Stop завершился
пустым cgroup перед release lease, foreign network sentinels сохранились.
VM выключена с qemu_exit=0. Evidence `historical-69d7ac4/` и `actual-assets-retry.log`.
Первый guard launcher получил EXEC203 из-за 0700 transport-каталога в госте;
после root-owned0755 guest-only fixture directory guard прошёл. Этот прогон —
только historical evidence: два независимо найденных cleanup P2 блокируют его
приёмку как final gate. Нельзя приписывать его следующему source head.

Три regressions воспроизвели RED: issued assets после reap, same-inode size change,
same-size rewrite. Исправление сохраняет custody и quota; forked descendant с
открытым FD подтверждает, что reap прямого parent не является empty evidence.
Actual private tmpfs fixture дополнительно переписывает owned0400 файл тем же
inode и проверяет сохранение ledger/reservation. Итог нового VM gate фиксируется
отдельно после сборки изменённых helper/worker.
