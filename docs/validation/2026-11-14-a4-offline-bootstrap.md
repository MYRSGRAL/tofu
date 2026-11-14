# A4 offline extraction: journal и masks прочитаны, required unit Zstd read gap

Same af/PR181 после `fc4e957da2cfdb5a6c6748e402aceaa3587a91d0`.
**Один bounded offline run**, без нового boot/VM/guestfish, mount/loop/NBD,
root/sudo/caps, package actions, units activation, clock/network changes,
новых inputs/download/build. Source142/MTREE4 и userdata/keyrings не менялись.

Результат — **PARTIAL_READONLY_EVIDENCE_REQUIRED_UNIT_READ_GAP**.
Actual dependency/job graph с After/Requires/Wants не получен; causal finding
не объявлен. Не повторяется недоказанная гипотеза time-sync из одного serial.
FullA4/finalLinux=false, factory CLOSED; package live hooks не проверялись.

## Inputs, inventory и bounded copies

Inputs взяты из сохранённого diagnostic report, без изменения:

- failed overlay SHA256 `337f15849e5a2ae47662e225a72dd3f3ba6746b75f5f175f0272cd15b82f5a99`;
- exact ранее GPG-verified signed147 backing SHA256
  `794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003`.

Точные private paths остаются в исходном retained report и новом private
manifest; raw bootcmd nonce в новый public evidence не переносится. До image
операций проверены bytes/SHA/backing и отсутствие active source/QEMU references.
После run совпали также inode/device/size/mtime обеих inputs. Новые файлы
создавались только в fresh own output/scratch; исходные overlay/base не chmod-ились.

Existing inventory:
`qemu-img 11.1.1`, `debugfs 1.47.4`, `btrfs-progs v7.1 (+ZSTD)`,
`journalctl systemd262`; e2ls/e2cp отсутствуют. Exact tool SHA/owner UID —
`docs/evidence/a4-offline-bootstrap-142/run1/prepare.json`.
Debugfs к Btrfs не применялся. Новых tool binaries/deps не получалось.

У текущего `qemu-img dd` нет skip. Поэтому сначала на synthetic fixture проверен
`convert --image-opts driver=raw,offset,size,file.driver=…` с exact bytes и
неизменным source. Actual read: dd только первых 1 MiB для GPT; header/table CRC,
entry limits, overlaps, partition bounds проверены Python. Read window offset/size
также проверены на sector alignment и предел virtual image. Никакого kernel device.

Image virtual size 12 GiB; единственный supported root — GPT partition3,
Linux root GUID, offset **316669952**, length **1829765120**, Btrfs superblock.
Sparse root.raw находился только под exclusive BSD directory flock в собственном
`/var/tmp/codex-myrs/transient/a4-offline-extract-f19965f3`; allocated
727605248 bytes. Backing и overlay directory держались под shared flock.
Large partition/GPT/probe copies удалены после проверки отсутствия references.
Никакой large raw copy в workspace не остался.

## Фактически прочитанные данные

Тонкий `btrfs restore` использовал anchored path selector только unit namespaces,
cloud-init logs и journal directory, fresh mode0700 output, без -i/-o/-m/-x,
alternate super/tree recovery, replay/repair. Btrfs root list и dry-run selectors
не читали user files. Guest `/home`, `/root`, `/etc/ssh`, shadow/passwords и
`/var/lib/cloud/*user-data*` не извлекались. Unit symlinks не dereference-ились
как host paths; public targets сохранены только как metadata strings.

До первой ошибки успели восстановиться:

- один 8 MiB `system.journal` из failed guest;
- `cloud-init.log` 54264 bytes;
- во втором, units-only fresh output — ровно пять Yon ik masks → `/dev/null`.

`journalctl --directory <own extracted path>` читался отдельными фильтрами
по cloud-init/time-sync/sshd units и `_PID=1`. Получены **287** sanitized records
из одного boot. `_CMDLINE`, raw bootstrap nonce, guest IDs и credential payload
не публикуются. Binary journal/raw tool streams сохранены **только private**,
root/dirs mode0700, regular files0600. Public JSON содержит закрытый metadata
subset и sanitized messages; private SSH hostkeys не извлекались.

Actual наблюдения:

1. PID1 начал Cloud-init Network Stage около monotonic 8.34s.
2. Cloud-init log фиксирует `init-network/config-bootcmd`, `Shellified 8 commands`
   и запуск `/bin/sh /run/cloud-init/tmp/…sh`; завершения bootcmd в извлечённом
   log нет. Это execution evidence boot ordering, не полное объяснение ожидания.
3. PID1 journal в 8.4192s содержит:
   `Reload requested from client PID 410 ('systemctl') (unit cloud-init-main.service)…`.
   Это bootstrap systemctl, **не package hook Reload proof**; pacman не запускался.
4. Все пять masks реально присутствуют на диске как symlink `/dev/null`.
   Это offline file proof, не runtime LoadState/MainPID/job verification.

## Точный read gap и terminal stop

Первый actual restore exit1:

```text
ERROR: zstd frame incomplete
ERROR: copying data for …/var/log/cloud-init-output.log failed
```

Failure сохранён до любого следующего действия. Уже восстановленный journal
оказался читаемым. Затем более узкий **units-only** selector исключил logs и
journal целиком: это чтение другого разрешённого namespace в том же bounded
session, без ignore-errors и без повторного чтения повреждённого output-log.
Он также завершился exit1:

```text
ERROR: zstd frame incomplete
ERROR: copying data for …/usr/lib/systemd/system/network-online.target failed
```

`network-online.target` остался нулевым partial file; он **не** используется как
unit definition и его отсутствие directives не интерпретируется как факт.
После ошибки нужного unit namespace дальнейший Btrfs restore прекращён.
Не применялись `--ignore-errors`, другие superblocks, replay/repair, новый
filesystem decoder/helper/VM. Existing decoder заявляет +ZSTD, но не прочитал
два конкретных compressed file extent paths. Это **tool/data read gap**, а не
доказательство «Btrfs unsupported целиком» или повреждения source/kernel.
Без независимой проверки нельзя выбрать между decoder compatibility и
повреждёнными compressed data. Inputs остались byte-identical.

Поэтому actual After/Requires/Wants требуемых units недоступны. Нельзя назвать
точной причиной SSH refusal ни NTP/time-sync, ни ordering cycle на основании
одного bootcmd/journal/serial. Достоверные observations выше сохранены отдельно
от causal inference. Пустой partial unit не подменяется host/reference unit.

## Owned cleanup и retained evidence

Controller завершил сессию после зафиксированных read errors; его clean exit
не является filesystem/extraction PASS. Operative btrfs exit1 сохранены с
argv/PID/stderr отдельно. Все дочерние readers reaped, scratch references=[],
собственный transient root удалён. Input SHA и identity до/после совпали.
Original failed overlay, backing и прежние userdata copies не удалялись.

Private derived root вне TTL:
`local/validation/a4-offline-bootstrap/f19965f3/` — **8.59 MiB**, mode0700,
regular files0600. Содержит binary journal, sanitized/full-private manifests,
commands/PIDs/exits, executed worker/adaptive source, partial files и read failures.
Raw manifest receipt + SHA привязки:
`docs/evidence/a4-offline-bootstrap-142/run1/raw-manifest-receipt.json`.
Public files проверены на отсутствие исходного bootcmd nonce.

Новые tools/tests принадлежат этой задаче. 29 A4 tests и Ruff PASS: GPT CRC/bounds,
exact sparse window, thin selector, forbidden guest paths, journal filtering,
canonical own directory/no symlink escape и полная redaction multiline key payload.
Последние два guards усилены после actual extraction; exact executed source
сохранён отдельно, actual image extraction не повторялась.

## Минимальный proposal для root, не исполнен

`docs/design/a4-package-hook-proposal-142/offline-read-gap-next.json` предлагает
ровно один новый bounded **units-only** read тем же pinned `btrfs restore` с
его существующим readonly `-i/--ignore-errors`, fresh own output и per-file RED
accounting. Это не repair и не новая VM; однако текущий run уже terminal STOP,
поэтому флаг **не включён самовольно**. Exit0 при -i не будет PASS: каждый
failed/нулевой required unit останется отсутствующим proof. Если достаточные
actual definitions не прочитаются — gap сохраняется, Root решает необходимость
другого проверенного readonly decoder input; он сейчас не скачивался/не собирался.
Никаких filesystem mounts/replay/repair/new helper VM не предлагается выполнять.
