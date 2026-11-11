# Executable attribution: admission и отрицательное multiUID доказательство

Fresh follow-up cddfa1c с existing adapter: [production gate](2026-11-11-production-existing-adapter.md).
Он отдельно подтверждает четыре cases на exact source; старый final FAIL ниже не изменён.
Там же исправлена причинная атрибуция worker FD fix: source memfd реально mode0777,
а прежний mode0600 control был условным, не доказательством причины production failure.

База: `95e88474c0b4a18797dc28fa873a4b6b3adf2fa4`. Добавлен отдельный
`system_profile_policy`, legacy `system_profile` не принимает policy и сохраняет
прежнюю компиляцию. Положительный FR20 этим изменением **не закрыт**.

## Проверенный предел прав

`tests/isolatedfixture/attribution_privileges.py` создаёт отдельные user/net/pid/mount
namespaces и настоящий диапазон subordinate IDs. Карта: guest UID 0 → host 1000,
guest 1..65536 → host 100000..165535. Клиент 1001 и worker 1002 соответствуют разным
kernel UID, это не два имени mapped root. Клиент держит TCP/UDP IPv4/IPv6 сокеты.
Worker сохраняет только CAP_NET_ADMIN, NoNewPrivileges; exe, fd directory и ns/net
клиента дают EACCES. У guest root helper с одним CAP_NET_ADMIN попытка запустить
child с user/group=1002 и пустыми supplementary groups даёт EPERM: production
helper unit не предоставляет SETUID/SETGID. Разрешённый read-only CLI probe потому
не добавлен: он не сможет перейти на worker UID в существующем unit.

Production unit `yonik-worker.service`: User/Group=yonik-worker, CAP_NET_ADMIN,
PrivateNetwork=no, ProtectHome=yes, RestrictNamespaces=yes; ProtectProc не задан.
Host userns fixture воспроизводит UID/capability kernel barrier (kernel
7.2.9-1-cachyos). Дополнительно поднята собственная QEMU/KVM VM, 2 CPU/2GiB,
на официальном Arch image (guest kernel 7.2.11-arch1-1) с SHA-256
`794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003` и проверенной
GPG подписью primary `1B9A16984A4E8CB448712D2AE0B78BF4326C6F8F`.
Host network/packages/keyring не менялись. Старые VM artifacts PR73 и baseline
64/64 PR86 не выданы за новые production доказательства.

Исправление по independent review: earlier `actual-app-negative.log` имеет PASS
для dirty snapshot на базе 95e8847 (helper dbdd4aee…, worker a678a76e…, gate
fd649656…). Это не clean committed head. Финальный `actual-final-app-negative.log`
с helper 39164032…, worker 2ad61c76…, gate 62d9e823… имеет FAIL на initial
State=unknown, до Prepare/denial. Для app admission head 340ccf4 actual PASS не
установлен; есть static/code evidence. Равные snapshots не доказывают denial,
если сам case не дошёл до него. Raw logs сохранены неизменными.

Actual Xray non-app policy проходит новый Prepare→Apply→Stop→Unlock;
живой production worker имеет UID/GID 969, все Cap masks=0x1000,
NoNewPrivs=1, TracerPid=0. Sing-box non-app TUN в этом run отклонён из-за выбора stock вместо adapter; различие
описано ниже. Это не положительный FR20.

`attribution_unit_privileges.py` отдельно повторяет security properties production
worker unit, меняя только ExecStart/socket wiring/logging для read-only witness.
Проверены default proc view и дополнительное ужесточение ProtectProc=invisible:
чужие exe/FD/net namespace недоступны, capabilities ровно NET_ADMIN. Source unit,
точный derivative unit, source hash и PID/UID записаны до запуска. Это diagnostic
unit с production security settings, не подмена production binary; положительный
Xray Apply выше использует неизменённые production units и текущие binaries.

Evidence вне TTL:
`/home/myrs/projects/yonik/local/validation/attribution-minimal-95e8847/`.
`manifest-before.json` записывает source/unit/tool hashes перед kernel case;
`kernel-negative.json` — mappings, caps, sockets, mountinfo/cgroup, errno и timings.
`primary-sources.json` — pinned upstream files и SHA256.
`final-native-gate-manifest-before.json` фиксирует все source hashes до final
production gate; `additional-gate-manifest-before.json` — дополнительный exact test
binary/source. `final-vm-state.log` подтверждает hashes установленных executable/unit.
`app-no-effects.json` подтверждает побайтовое равенство snapshots link/rule/route/nft
и worker state до/после неуспешного final case. Это не подтверждение denial. Source Linux v6.12 —
объяснение проверки ptrace, не заявление об exact source установленного kernel.

## Admission и контракты

Новый closed flat DTO переводится через существующие domain constructors,
лимиты, source-selector validation и compiler capabilities. Полная Policy
не сохраняется в prepared artifact: остаются sealed compiled config, его digest,
helper-private requirements и ограниченная peer identity. Размер Policy wire ≤64KiB,
128 правил; общий RPC ≤64KiB/depth5. Дополнительные peer identity/pidfd и три
namespace identifiers покрыты существующим консервативным reserve 128KiB artifact.

После свежей авторизации helper получает ProcessID/UnixUserID того же unique
sender из canonical bus daemon; payload PID не принимается. Capture закрепляет
pidfd/starttime/UID/net,pid,mnt namespaces. Отказ capture либо admission возвращает
Unsupported до компиляции, registry, worker и network effects. Gate повторяется
перед Apply; immutable requirement получен только из enabled компилируемых правил.
Отсутствие executable условий сохраняет прежний путь. Тесты покрывают closed
wire/roundtrip, private bus authorization и one-shot, ранний отказ backend без
ресурсов, смену starttime, same-UID proc access и исчезнувший PID.

Runtime probe ограничен одним authenticated PID, 64 FD и 16KiB stat. Он различает
отказ proc-доступа, смену процесса, лимит и недоказанный native mechanism. Исчезнувший
отдельный FD допустим при неизменной process identity; такой flow не объявляется
известным. Успешный helper lookup не доказывает worker view, поэтому app-policy
пока Unsupported даже при успехе этого отрицательного witness. Это консервативный
admission до реализации подтверждённого worker/native механизма, а не готовая
положительная attribution. Существующие native unbounded scans не оптимизированы.

## Причина и варианты следующего решения root

Pinned [Xray 26.10.31](https://github.com/XTLS/Xray-core/blob/v26.10.31/common/net/find_process_linux.go)
ищет inode в proc/net и затем PID/FD/exe.
Pinned [sing-box 1.14.4](https://github.com/SagerNet/sing-box/blob/v1.14.4/common/process/searcher_linux.go)
использует socket diag, но executable всё равно требует buildProcessPaths с
Readlink exe/ReadDir fd. NET_ADMIN не предоставляет ptrace read access к foreign UID;
доступ к socket UID/inode не равен executable identity.
[Linux v6.12 proc](https://github.com/torvalds/linux/blob/v6.12/fs/proc/base.c)
показывает соответствующие ptrace access checks.

1. Рекомендуемый для отдельного дизайна путь: per-user bounded owner broker,
   запущенный пользовательским service, и native hook обоих ядер. Предлагаемый
   минимальный API: `Lookup(netns_cookie, protocol, local, remote)` →
   `Known(identity,starttime,generation)|UnknownFlow|MechanismUnavailable`.
   Ответ authenticated UNIX peer, ограниченный deadline/rows/FD/bytes и поколением;
   identity связана с PID lifetime, executable inode и namespace. Caller не передаёт
   произвольные proc paths/PID для чтения. При credential drop, nondumpable process,
   другом netns или потере broker действует Unsupported до Apply. API ещё не
   утверждён и код Go/native hook не написан. Требуется отдельно определить
   доверие к user-controlled broker, иначе его ответ не является security authority.
2. Отдельный read-only systemd probe с worker identity/view способен подтвердить
   отрицательный witness, но сам не решает foreign-UID lookup. Его unit/IPC и
   admission lifecycle требуют решения root; existing worker-check нельзя выдавать
   за эквивалент production worker (у него пустой capability set).
3. SYS_PTRACE/SETUID/широкие capabilities либо ослабление proc sandbox здесь не
   добавлены. Путь с privileged broker потребует отдельного решения о правах;
   текущий helper с NET_ADMIN не является таким broker.

## Sing-box: условный stdin control и неверный выбор adapter в fixture

Actual differential proof `stdin-negative-proof.log`: root-owned sealed memfd с
`{}` и mode0600 не может быть повторно открыт sing-box через `/dev/stdin` от
worker UID; worker-owned sealed memfd с точными теми же байтами проходит check.
Разрешённый scoped fix создаёт worker-owned memfd только после полной проверки
полученного config FD. Pread сохраняет source offset, buffer zeroize, mode0600,
CLOEXEC и seals; helper reserve учитывает вторую физическую копию до empty cgroup.
Cancellation/timeout не освобождают этот reserve до recovery. При Drop без empty
proof reserve сохраняется в ограниченном ByteBudget карантине до выхода helper.
Xray stdin path прежний. Отдельный production-unit stdin control проверяет этот
путь без TUN; его успех не объявляется успешным sing-box TUN Apply. Actual Check→Run→Stop
этого control прошёл: stdin UID=969, mode0600, sealed, 236 bytes.

Первоначально этот отказ был неверно описан как отсутствие необходимого adapter.
Bounded audit установил различие артефактов: PR90 поставил stock binary 1264361b…,
а старый c758 gate использовал существующий adapter 4c31db07…. На одном и том же
сохранённом config adapter check=0, stock check=1. Следующий факт относится к stock:
`compile_system_policy` передаёт `inbounds[].external_configuration=true`, а
pinned sing-box 1.14.4 возвращает `json: unknown field "external_configuration"`.
Поле намеренно присутствует в `95e8847:crates/yonik-engine/src/system_tun.rs:129`:
это существующий adapter-specific contract, а не доказанный дефект compiler.
`sing-policy-native-check.log` содержит exact error, `sing-policy-control.json` —
синтетический config, source/native hashes сохранены. Pinned
[option/tun.go 1.14.4](https://github.com/SagerNet/sing-box/blob/v1.14.4/option/tun.go)
не объявляет это поле. Удаление поля без adapter решения нарушило бы контракт
helper-owned конфигурации TUN. Engine TUN adapter/Go здесь не менялся.
Для восстановления прежнего contract нужен правильный pinned adapter, не новый
TUN Go-патч и не удаление поля. Более сильный owned-FD/no-link-effects ABI и
version/capability ack остаются отдельным предложением; существующий adapter
оставляет MTU/up/offload операции. См. [bounded audit/план](2026-11-11-native-adapter-broker-plan.md).

## Измерения и пределы приёмки

Kernel fixture читает четыре proc socket tables при 4 и 40 реально удерживаемых
сокетах; проверяет kernel inode/UID и headers. Для каждого масштаба сохраняет
стоимость 1/10 повторных reads и row counts. Успешного foreign procFD lookup нет:
доступ запрещён. Это не G-POWER и не доказательство bounded upstream scans.

В отдельном VM net/pid namespace native Xray и sing-box запускаются с UID 969,
NET_ADMIN, NoNewPrivileges, клиент — UID 968. TCP/UDP v4/v6, два executable пути и
два назначения: по 16 flows на ядро и по 160 при масштабе 10×. Все 352 flows
пришли в default D, включая выбранный executable, который root baseline направлял
в proxy/block. Это конкретный отрицательный native witness: proc-based lookup
не работает при этих правах. Raw-native fixture не копирует systemd mount sandbox;
его ограничения дополняются separate unit witness и actual production admission.
`elapsed_us` измеряет весь flow, не отдельный owner lookup; стоимость native
procFD/rows без instrumentation не выдана за измеренную. Таблицы/FD не оптимизированы.
Manifest source/binary/config/unit записывается **до** каждого native case.

Механизм attribution с этими правами не подтверждён, поэтому переход от рабочего
app plan к потере механизма перед Apply actual не достижим: Prepare уже отказал.
Повторная проверка в Apply остаётся защитой контракта; tests проверяют PID/starttime
и immutable requirements. Для положительного FR20 нужен отдельный root-approved
broker/native hook. Sing-box TUN non-app Apply этого run остаётся Unsupported из-за неверного выбора
binary (существующий adapter принят в #49 и в offline config control), поэтому full dual-core production acceptance не объявляется. PR draft.

## Проверки и завершение fixture

Engine library: 2/2; Linux library: 366 passed, 24 ignored. Общий all-targets прогон
engine/Linux прошёл до отдельного worker fix; после fix выполнены полный library
прогон, сборка actual ignored gate и pinned Clippy 1.85.1 всех targets с -D warnings.
Cargo fmt --check и Ruff check/format проходят. Actual final cases: app denial обоих
ядер от UID 968 — FAIL initial State=unknown (denial не достигнут); earlier dirty
snapshot denial PASS не перенесён на final head. Xray policy Prepare→Apply→Stop→Unlock — PASS; sing-box
worker-owned stdin Check→Run→Stop — PASS. Полный dual-core TUN non-app gate — FAIL
из-за выбора stock вместо existing patched adapter; повторный VM gate не запускался.

В source evidence сохранены pinned libbox PlatformInterface 1.14.4: OpenTun и
FindConnectionOwner — существующие точки для обсуждения adapter/hook, а не готовое
расширение standalone CLI. Предлагаемый bounded broker API выше требует отдельного
root решения и native интеграции обоих ядер. Никакие дополнительные capability,
ProtectProc ослабления, Go-код, Docker, host install или deployment не применены.

VM штатно выключена (QEMU exit=0); binaries/source/manifests/evidence сохранены вне
TTL. Собственная Cargo-сборка очищается cargo clean, затем удаляется только свой
transient task root под exclusive BSD flock. Результат — cleanup-result.json в evidence.
