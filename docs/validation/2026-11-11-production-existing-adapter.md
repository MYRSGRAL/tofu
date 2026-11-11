# PR90: fresh production gate с существующим name-based adapter

Exact fixture/source commit: `cddfa1c23b05bb2aaa5c09ecae6c6fb655f68b88`.
Production code/source equivalence: **386 blobs** под crates/, adapters/, packaging/,
Cargo.toml/Cargo.lock побайтово одинаковы у 340ccf4 и 5312cd8; cddfa1c добавляет
только Python fixture. Список path/blob IDs сохранён в code-blobs-340-5312.json.
Готовые helper/worker переиспользованы по exact SHA без Rust rebuild.

Evidence вне TTL:
`/home/myrs/projects/yonik/local/validation/adapter-production-5312/`.
Manifest записан **до cases**: git head, все source/unit/file SHA, artifact hashes,
adapter upstream/4 patches/build manifest, controller hashes, порядок и retry budget=0.
Manifest в guest и host побайтово совпадает. Controller fixture входит в commit;
его libdbus string-RPC codec предварительно прошёл private-bus roundtrip. Actual
cases используют настоящий system bus/polkitd и production helper, не private bus.

## Артефакты и контракт

- Helper: `39164032f409cd185d4eaaea49daaa166aa2f6b8ce1cc0adbda4e0881343e907`.
- Worker: `2ad61c765606402216eeb59646e2ed0e98cb3de990ec0fc1014cffb2c9d610cc`.
- Xray 26.10.31: `cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1`.
- Sing-box adapter 1.14.4: `4c31db07cb394479b58cc9aace8c8bef5da05a73db6dacca4a019aeb8b14e16d`.

Adapter manifest привязан к upstream c005f35300ce76427dc285481ec5695f5b179cd2
и четырём canonical patches текущего repo. Все четыре patch SHA сверены до staging;
в guest registry schema/version/hash остаются прежними, файлы root-owned regular,
nlink=1, binaries mode0500, registry mode0600. Существующий TrustedRegistry создаёт
sealed immutable snapshot и проверяет digest; его проверки не обходились.
Это **external_tun_configuration/name-based** adapter. Он не объявлен owned-FD
no-effects adapter, feature ack нового ABI не выдуман, Go code не изменён.

Официальный image SHA
794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003,
подпись primary 1B9A16984A4E8CB448712D2AE0B78BF4326C6F8F проверена в отдельном
GNUPGHOME. Своя QEMU/KVM VM 2CPU/2GiB, kernel 7.2.11-arch1-1. Host packages,
network, units и keyring не менялись. Polkit установлен только внутри VM.

## Четыре однократных cases

До первого case manifest фиксировал порядок: Xray app denial, sing-box app denial,
Xray positive, sing-box positive. Все RPC и data-plane клиенты работают от отдельного
разрешённого UID968; production worker — UID969. Не меняли owner/reset/recovery
для обхода initial State, не повторяли failed case, Apply или traffic.

1. Xray app denial: fresh State=no_managed_policy → Prepare=typed Unsupported →
   State=no_managed_policy. PASS; worker/check cgroups и journal не наблюдались.
2. Adapter sing-box app denial: те же исходы, PASS. Не positive FR20.
3. Xray non-app policy: State → Prepare → Apply=tunnel_starting → четыре успешных
   TCP/UDP IPv4/IPv6 echo через actual TUN/VLESS peer → Stop=recovery_required →
   Unlock/State=no_managed_policy. PASS.
4. Adapter sing-box non-app policy: тот же production lifecycle и четыре echo,
   PASS. Это устраняет stock-vs-adapter acceptance discrepancy для нового helper.

Case controller не выводит credentials/handles/native config. После Apply он
проверяет actual native executable SHA, UID/caps/NoNewPrivs и stdin metadata до
однократного traffic; это bounded read-only readiness barrier, не повтор RPC/traffic.
Полный source контроллера и exact args сохранены. Cases не использовали sleep helper,
подмену authorizer, Go/native hooks или дополнительные capabilities.

`guest/case-results.json`, raw serial/controller/events и checked-case-summary.json
показывают каждый исход отдельно. Новых initial State=unknown в этом run нет.
Старый actual-final-app-negative.log из предыдущего run **остаётся FAIL**; он не
переписан и не объявлен PASS задним числом. Новое подтверждение относится к cddfa1c
и совпадающим production blobs; старый earlier dirty snapshot остаётся отдельным.

## Права, FD и cleanup ordering

Оба actual native worker имеют UID/GID969; CapInh/Prm/Eff/Bnd/Amb=0x1000,
NoNewPrivs=1, TracerPid=0, ProtectHome=yes и ProtectProc=default. Systemd units
побайтово совпадают с committed source. Worker runtime metadata, namespaces/proc
mountinfo/cgroup и config SHA сохранены без вывода plaintext config.

Sing-box stdin: worker-owned UID969, mode0600, seals=15. Copy path действительно
выполнен. Xray использует исходный FD: UID0, **mode0777**, seals=15.

Sampler5ms читает обе cgroups до/после journal. После наблюдения живого worker
обоих ядер есть отдельный snapshot с 8 Confirmed records и обеими empty/absent
cgroups, затем более позднее уменьшение records. Gap от последнего такого witness
до первого decrease: Xray 5.258131ms, sing-box 5.229574ms. Все наблюдённые decreases
происходят при empty cgroups с обеих сторон чтения. Это sampled ordering evidence,
не универсальная трасса каждого syscall между samples.

Чужие для helper sentinel nft table/chain, dummy link/address и RPDB pref1234
сохранены до/после **каждого** case; snapshots совпадают. Final journal records=0,
public_policy/session_recovery=null, worker/check inactive/MainPID0. Финальные
metadata и hashes сохранены в final-state.log.

## Важная причинная поправка worker FD fix

Предыдущий permissions control вручную ставил root memfd mode0600. Он доказывает
только: такой FD нельзя reopen через /dev/stdin от worker UID. Он **не** доказывал,
что именно это вызвало прежний production sing-box failure.

Fresh runtime показывает исходный config_file() memfd mode0777. Узкий дополнительный
control на existing adapter, без Prepare/Apply/traffic/case replay: root-owned sealed
memfd с {} в default mode0777 проходит check от UID969; тот же FD после chmod0600
даёт EACCES. Source factory memfd_create не вызывает fchmod; UMASK0077 unit не
является доказательством mode0600 для memfd. Логи и hash контроллера сохранены.

Поэтому commit585f53d корректно характеризовать как worker-owned mode0600 hardening
и поддержку строгого source-FD режима, а не подтверждённое исправление причины
старого stock failure. У stock отдельно доказан unknown external_configuration.
World permission bits unnamed memfd сами по себе не доказывают утечку: reachability
через proc/FD также ограничена. SourceFD mode менять в этой задаче не разрешалось
и production код не менялся.

Конкретное предложение root для отдельного решения: явно задавать source config
memfd mode0600 в helper factory, оставив проверенную sing-box owned copy и её budget
custody; Xray продолжает читать унаследованный FD напрямую. Это будущая узкая Rust
правка с отдельной проверкой, **не выполнена** здесь. Go разрешения пока нет.

## Предел результата

Positive здесь — обычный non-app TUN routing обеих engines с existing adapter.
Executable policy пока даёт Unsupported; положительный FR20 не объявлен. Existing
adapter не заменяет предложенный owned-FD/no-link-effects ABI или trusted broker.
Новых cap/proc-view/production-contract изменений и Rust builds в этом run нет.
VM и transient cleanup фиксируются в vm-shutdown.json/cleanup-result.json; source,
raw evidence и ссылки на постоянные binaries сохраняются вне TTL.
