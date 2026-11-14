# A4: bounded serial-only bootstrap diagnostic — proposal

Статус: **DESIGN/TESTS ONLY, NOT AUTHORIZED TO BOOT**. Основание:
`0e679ea9a3a9fa95d5c1ce8a5ecc01417bcc0041`, offline graph UNKNOWN.
Root должен отдельно review этот документ и минимальный будущий driver diff.
Новая VM, seed, overlay или guest collector в этой задаче не создавались.

## Выбор и точка вставки

Выбран guest readonly collector: guest kernel читает собственные compressed
unit bytes, host декодирование Btrfs больше не повторяется. Третий SSH retry
без нового evidence не подходит; новый decoder/mount/repair исключены.

В actual `Driver.cloud_config` (`tools/a4_package_hook_vm.py`) маски создаёт
**не systemctl mask**, а пять `ln -s /dev/null`. DiagnosticDriver переносит
существующий runcmd в bootcmd. Первый potentially blocking systemctl —
`systemctl enable --now sshd.service`. Нельзя заранее называть его виновником.

Минимальный будущий diff только в отдельном serial diagnostic driver:

```python
config = super().cloud_config(public_key)  # existing DiagnosticDriver config
assert config['bootcmd'][6] == ['systemctl', 'enable', '--now', 'sshd.service']
# mkdir + пять ln уже выполнены; существующие команды не заменяются.
config['bootcmd'].insert(6, ['/usr/bin/python', '/run/a4-serial-collector.py', '--initial-and-one-delayed'])
```

Collector source доставляется cloud-init write_files как mode0600 с SHA binding
в seed manifest. Implementation ещё отсутствует; текущий новый module
`tools/a4_serial_projection.py` — чистая тестируемая projection без I/O/exec.
Нельзя запускать старый driver как fallback: отдельный entrypoint должен иметь
свой exact root decision, одноразовый O_EXCL authority marker, запрещённые
workload/SSH/package dispatch, а также явный execution gate.

Пять masks остаются перед collector и первым systemctl; существующий sshd
вызов, nonce write и cloud-init ordering не меняются. Collector **не вызывает**
mask/start/stop/enable/reload. Existing sshd enable остаётся bootstrap behavior,
не новым diagnostic действием. Если root считает даже его недопустимым — STOP,
не удалять команду самовольно. Product service activation всегда FAIL/STOP.

## Закрытый collector contract и точные readonly команды

Девять required units фиксированы в `a4_serial_projection.REQUIRED`:
sshd.service, cloud-init-network.service, cloud-init-main.service,
cloud-config.service, cloud-final.service, network.target,
network-online.target, time-sync.target, systemd-time-wait-sync.service.
Никаких argv/unit/path от serial, SSH, host user data или guest environment.
Environment subprocess: PATH=/usr/bin, LC_ALL=C, SYSTEMD_PAGER=cat,
SYSTEMD_COLORS=0; stdin DEVNULL; shell=False; stdout/stderr только в bounded
memory. Raw stderr не печатать, только exit/timeout/parse status.

Для каждого required unit, последовательно, timeout 1s на вызов:

```text
/usr/bin/systemctl --no-pager show <EXACT_REQUIRED_UNIT> -p LoadState -p ActiveState -p SubState -p Result -p Job -p After -p Before -p Requires -p Wants -p Requisite -p BindsTo -p Conflicts
/usr/bin/systemctl --no-pager show <EXACT_REQUIRED_UNIT> -p FragmentPath -p DropInPaths
```

Пути второго запроса — **private in-memory discovery**, не serial payload.
Это не systemctl cat: произвольные ExecStart/Environment/Description не выводятся.
Job snapshot один раз в каждой фазе:

```text
/usr/bin/systemctl --no-pager --no-legend --plain list-jobs
```

Парсить ровно четыре столбца `id unit type state`, только девять required units,
job id uint32, type/state из `project_job`. Иные строки/формат → UNKNOWN,
не печатать raw. Foreign jobs только считать; product unit job из пяти masks
→ отдельный fixed `PRODUCT_JOB_FAIL`, немедленное завершение собственного VM
host driver после сохранения evidence. Raw job names/payload не публиковать.
Вызовы могут обратиться к PID1/load definitions; это readonly observation,
не гарантия отсутствия временного влияния на загрузку. Замерять начало/конец.

### Чтение definitions

До subprocess: проверка guest PID1 `/proc/1/exe` → systemd, `/proc/1/comm`,
boot UUID `/proc/sys/kernel/random/boot_id`, QEMU DMI как существующий guard,
own run token из seed, пять точных `/dev/null` masks. Guard failure → fixed RED,
никакого дальнейшего collector action. Это existing QEMU scope, не обход #147.

Только FragmentPath/DropInPaths этих девяти units. Разрешённые roots:
`/etc/systemd/system`, `/run/systemd/system`, `/usr/lib/systemd/system`,
`/run/systemd/generator`, `/run/systemd/generator.early`,
`/run/systemd/generator.late`. Путь canonical absolute, под этими roots,
basename exact required unit либо `<required>.d/*.conf`, без NUL/..;
symlink resolution maximum8, только между этими roots. `/dev/null` — MASKED,
не definition; другие special files, hardlink ambiguity, escape → RED.
Canonical root walk через dirfd/O_NOFOLLOW, regular file only, no mount operation.
Не обходить дерево guest: maximum9 fragments + 4 dropins/unit, 16KiB/file,
256KiB total read. При превышении/ошибке/truncation/missing/changed fstat
до/после → RED/UNKNOWN; не hash усечённых bytes как полного файла.
SHA256 полных unit bytes + byte count + source kind/index разрешены, это
**не** key/hash SecretService данных. Absolute source paths не serial-print.

Definition serial payload — `project_definition`: только relations из [Unit],
unit из required allowlist, bytes/digest/status. Comments, Description,
Environment, Exec*, arbitrary directives и raw bytes не выводятся.
`NAMES` — закрытый список допустимых dependency tokens в module; неизвестные
имена только увеличивают redacted count. Dropins последовательно по effective
systemd path precedence и lexical order, reset/continuation учитываются;
неразрешённые aliases/specifiers/default dependency semantics → UNKNOWN.
Projection намеренно **не full raw definition**: digest связывает actual bytes,
но пропущенные semantics не дают объявить graph complete. PID1 effective fields
сопоставляются с projected relations; противоречие → UNKNOWN, не auto-fix.

## Bounded serial framing и delayed snapshot

Один initial collector process запускает ровно один sleeping readonly child
перед своим выходом; **не** systemd-run/unit, не daemon/service. Child sleep
30s от guest monotonic initial entry, затем повторяет только properties/list-jobs
и identities/masks, не чтение definitions. Это snapshot именно пока следующая
existing bootcmd команда может быть blocked. Нет циклов polling/retry.

Initial hard deadline25s (18 show + list-jobs по1s плюс bounded read/projection),
delayed deadline15s (9 properties + list-jobs по1s); child absolute monotonic
deadline50s от initial entry.
Оба process PID/starttime и outcome фиксируются в serial. Initial launcher
не может wait child без блокировки bootcmd: после выхода launcher child
reparented guest PID1. END frame — evidence completion, не proof wait/reap;
отсутствующий END означает UNKNOWN. Финальный owned QEMU wait/reap завершает
все guest process references; host не заявляет direct guest child reap. Не переносить задержку в основную
bootcmd: initial launcher возвращается после bounded initial snapshot, child
не держит stdout pipe; пишет только `/dev/ttyS0` с O_NOCTTY и bounded writes.
Serial write watchdog2s; failure не повторять. Новый child нельзя считать
готовым, пока ACK с PID/starttime не принят host; UNKNOWN если ACK отсутствует.

Каждая compact JSON строка ≤4096B, ≤128 records, суммарно ≤256KiB. Fixed frame
`A4SERIAL1`, sequence, phase(initial/delayed), monotonic_ns, run_token,
boot_id, collector_pid/starttime, record kind и sanitized payload.
Digest выбранного collector source в начальном frame. Sequence строго растёт;
duplicate/overflow/malformed/incomplete END/unknown kind → UNKNOWN.
Host принимает только свой private seed token + QEMU PID/starttime/base SHA,
оба snapshot одного boot_id; raw token не попадает в public projection.
Не принимать похожие строки из printk как substitute. Token не является
криптографической attestation; доверие ограничено signed guest и own seed.
Public export строится заново по allowlist, не regex redaction raw serial.
Никаких raw journal MESSAGE, command lines, key/password/config/userdata.

## Identity, inputs, host commands, deadlines и cleanup

Source142 `14229843f9a11bf98ec4f5992df00f3247102ea2`, tree
`7daa4ebb7cc25d3ceafab76809afc78753dc5ec3`, MTREE4 package SHA
`26a013747e4c1dbb95e7290cd37b981f5dc7ef891a8865897f09fc45bacd0237`
immutable. Package даже не передаётся guest, transactions0.
Base `/home/myrs/projects/yonik/local/validation/vm-base/Arch-Linux-x86_64-cloudimg-20261101.620452.qcow2`,
SHA `794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003`,
existing signature/key fingerprint `1B9A16984A4E8CB448712D2AE0B78BF4326C6F8F`.
Exact signature/tool/OVMF hashes: existing `readonly-inputs.json` и current
`verify_inputs` сохраняются; hash drift → STOP, no download/replacement.

Future host driver использует existing QEMU argv из `execution-contract.json`,
тот же normal-user KVM UID без новых caps/UID mapping. Fresh own overlay, seed,
vars и directory flock с первого использования; shared base-directory flock
весь срок ссылок. Не использовать retained failed overlays как writable disk.
Никаких host writable mounts/userdata/keyrings; restrict=on existing network,
clock/NTP/network flags без изменений. SSH endpoint может существовать из
неизменного bootstrap, но **ни одного SSH connect/retry/transfer** в этом run.
Host deadline: максимум90s от QEMU start до последнего serial snapshot;
collectors missing/partial на90s → terminal UNKNOWN, не продлевать до SSH240s.
Existing total900s — только верхний cleanup budget, не разрешение повторять boot.
Host serial reader capped1MiB raw, bound checks не копируют весь диск/guest.

До запуска root должен approve exact future driver/collector source SHA и
новый one-boot authority. Следующий перечень — **не исполнялся**:
1. Verify normal UID, ancestry/source hashes, signed base/tool hashes, available
   `/dev/kvm`, no old QEMU references; create own transient root+flock.
2. Existing `qemu-img create -f qcow2 -F qcow2 -b <exact base> <own disk>`;
   seed own collector+public bootstrap key+fresh private token. No build/download.
3. Existing QEMU argv, no SSH workload; bounded serial framing collection.
4. Retain raw serial, seed/commands/exits/source/hash manifest, failed overlay
   outsideTTL mode0700/files0600 **до** остановки/удаления scratch. Seed/token
   остаются private. Public только sanitized projection/receipt.
5. Проверить same own QEMU PID/UID/starttime; bounded TERM10s → при живом exact
   identity KILL5s → wait/reap. Никакого guest poweroff через SSH/systemctl.
   Не сигналить reused PID/чужой процесс. Проверить all collector/reader/QEMU
   references gone, base SHA/identity после. Удалить только собственный root;
   если refs/immutability неизвестны — сохранить и сообщить blocker/размер.

## Какой результат достаточен

До/после fixed monotonic интервала: manager Job=id, list-jobs тот же id waiting
или running, actual allowed dependency path + predecessor job/state позволяют
сформулировать **ограниченное** наблюдение о блокировке. Два snapshot не доказывают
вечный deadlock или внешнюю NTP причину. Job отсутствует/изменился → гипотеза
конкретного wait не подтверждена. Missing transitive definitions/fields,
redaction, timeout, read error или source mismatch → partial graph UNKNOWN.
Не выполнять исправлений при любом результате. Live package hook по-прежнему
NOT_PROVEN; fullA4/finalLinux=false, factory CLOSED.

## Проверки этой design-задачи

Чистая projection не импортирует subprocess/не читает host files. Tests проверяют
секреты в Description/ExecStart/Environment/dependency tokens, closed unit/job
allowlist, truncated/zero/invalid/oversize RED, reset/continuation, strict state
projection. Они не доказывают actual guest boot/runtime file-reader correctness.
Перед будущим запуском нужны implementation tests framing/token/limits,
canonical path/no symlink escape, partial child/VM reap, original bootcmd subsequence
и отказ package/SSH dispatch. Это явные root review preconditions, не выполненный
collector. Старые drivers, evidence и refs не изменены.
