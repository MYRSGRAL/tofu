# A4 serial-only: implementation/test handoff

Продолжение `59748be719b01cb74f8b3f4d210230e6093b3902`. **Только код и тесты**:
новый driver/collector не запускались; seed, overlay, VM, SSH, package actions,
mount/repair/download/build отсутствуют. Root отдельно review до любого boot.
Старые entrypoints, authority records, proofs, source142/packages и userdata
не изменены. FullA4/finalLinux=false, factory CLOSED.

## Конкретная реализация

- `tools/a4_serial_driver.py`: самостоятельный closed entrypoint, constructor
  без filesystem side effects; future authority обязателен до mkdir/commands.
  Не наследует SSH bootstrap/cleanup. Из старого driver переиспользует только
  pure cloud_config и verified hash/ownership/reference helpers.
- `tools/a4_serial_collector.py`: guest-only QEMU/PID1/cgroup2 guard, пять masks,
  product process/job refusal, fixed systemctl show/list-jobs, bounded file reads,
  initial snapshot и ровно один fork для delayed30s snapshot без definitions.
- `tools/a4_serial_protocol.py`: strict typed framing/parser и новая public
  projection без raw token/произвольных strings. Existing projection сохранена.

`serial-only-implementation-contract.json` содержит точные source SHA256,
launcher/tool pins, future host/QEMU argv, **неавторизующий** пример decision.
Source hashes вычислены после GREEN tests; JSON — review artifact, не root grant.
Файл authority читается только из фиксированного root proposal path. Его hash
фиксируется до начала; изменения authority/source до boot → STOP. Один O_EXCL
marker с authority digest вне TTL исключает повтор того же разрешения.
Existing147 standing decision также проверяется, source142/COLLECTED finalfalse,
frozen controller, оба package и все прежние input/tool SHA сохраняются.

## Важное уточнение доставки

Исходный design предполагал `write_files`; сохранённый cloud-init.log лишь
показывает, что write_files был skipped в прошлой конфигурации. Он **не доказывает**
выполнение write_files до bootcmd. Чтобы не зависеть от module ordering,
единственная вставка в bootcmd теперь:

```text
/usr/bin/python -I -c <closed digest-bound loader_source(token)>
```

После пяти прежних `ln`, до прежнего `systemctl enable --now sshd.service`.
Удаление вставки даёт точную прежнюю последовательность, проверено тестом.
Никаких systemctl mask/--no-block/order/timesync/clock/network изменений.

Loader — не generic exec: literal allowlist трёх имен, literal `/run/a4-serial`,
QEMU/root/PID1/cgroup2/mask guards **до записи**, собственный mkdir0700/O_EXCL,
files0600/O_NOFOLLOW, digest verification и единственный literal runpy target.
Нет path/unit/command argv inputs. Payload ≤96KiB, Python isolated mode,
источники связаны с reviewed authority до seed и повторно перед запуском.
Это доставка собственных fixture files; сам collector читает только свой
contract/source guards и readonly diagnostic inputs. Production/user state
не инжектируется. Loader отсутствует в старых drivers, новых seed не создавали.

## Read/stream границы

Девять required units и dependency-token/state/job allowlists фиксированы в
projection. Initial: show properties+paths для каждого, show пяти product masks,
list-jobs. Delayed: только properties/masks/jobs; definitions не повторяются.
Нет systemctl cat, journal raw messages, arbitrary shell/exec/unit/path fallback.
Product process или active/job у masked product unit → fixed PRODUCT_JOB_FAIL;
непрочитанный mask state → GUARD_RED/UNKNOWN, не false product activation claim.

Unit pathwalk — только шесть reviewed systemd roots, exact fragment или
`<required>.d/*.conf`, bounded aliases≤8, каждый компонент dirfd/O_NOFOLLOW.
Финальный inode сначала O_PATH/fstat regular+nlink1, затем read через held fd;
max16KiB/file, четыре dropins/unit, общий read budget256KiB, fstat до/после.
Escape/special/hardlink/truncated/missing/change/limit → RED_READ/UNKNOWN.
Полные bytes hashed, только relation projection публична, не raw directives.
Unknown dependency/default semantics → graph UNKNOWN.

Каждый subprocess: closed argv, stdin DEVNULL, stderr DEVNULL, ≤32KiB stdout,
timeout≤1s + kill/reap. Initial alarm25s; delayed один раз на30s, snapshot≤15s,
absolute child alarm≤50s. Child не держит cloud-init stdout/stderr; SPAWN PID/
starttime и END связывают фазы. Initial launcher не wait child: после его выхода
reap — guest PID1; host утверждает только завершение guest references через
финальный QEMU wait/reap. Нет новой systemd unit/scope/daemon activation.

Serial: ≤4096B/frame, ≤128 records/256KiB payload, raw≤1MiB; strict token,
boot UUID, monotonic order, seq/phase, source digest, parent/child PID/starttime.
Duplicate JSON keys, unknown fields/kinds, changed boot/child, malformed/partial
frame → UNKNOWN/STOP. Interleaved console frame не reparsed/fallback.
Public objects строятся заново; Environment/ExecStart/Description/raw stderr/
private token/прочие guest strings не попадают в export. Unit digests не являются
key/SecretService hashes. `CAPTURED` — completeness transport, **не** read integrity,
causal graph или live hook PASS; RED definitions могут быть доставлены полностью.

## Future runner и owned cleanup

Host commands закрыты фиксированными kinds: own GPG import/verify
(`--no-options --no-autostart --batch`, без gpg-agent activation), pinned qemu-img,
ssh-keygen для неизменного bootstrap public key, xorriso и QEMU. SSH connection,
scp, package workload и arbitrary command methods запрещены.

Normal UID/KVM, shared base-directory flock, own transient root BSD flock с
первого использования. Existing signed base SHA и all old tools сверяются;
нового input/tool fallback нет. QEMU argv из старого reviewed contract, restrict=on
network и прежний bootstrap sshd сохраняются; SSH attempts0. QEMU/host command
environment закрыт, inherited wrappers/LD_PRELOAD туда не передаются. Future
launcher argv/environment явно приведены в JSON; Python ELF также SHA-pinned.

Serial observation≤90s и не позже total870s; retention cutoff880s резервирует
TERM10/KILL5/reap и base verification внутри исходного total900 budget. Каждый
signal только после UID/PID/starttime/full argv check, перед KILL проверка повторна.
Source-defined bounds являются monotonic checks и bounded I/O, а не обещанием
hard realtime latency диска/kernel. Любой deadline/unknown — blocker, без retry.

До signal сохраняются capped serial и точные собственные seed metadata/user-data
в private out. Retention error не мешает остановить собственный VM: root остаётся.
Consistent failed disk копируется только **после reap**, до scratch delete,
max256MiB (больше → blocker/retained root, не молчаливое truncation disk).
Base SHA/identity и references проверяются перед delete. Changed owner/PID,
refs или SHA mismatch → никакого удаления/чужих signals, cleanup-blocker evidence.
Только own root/marker, never old failed images/userdata/foreign directories.
Private commands, source copies, seed/token/serial/disk receipts и manifest
вне TTL0700/0600. На partial failure запись evidence и release locks идут через
finally; неизвестное состояние не разрешает повтор bootstrap.

## Проверено сейчас

71 A4 unittest GREEN, включая30 новых implementation tests; Ruff check/format
GREEN. Evidence: `docs/evidence/a4-serial-implementation-142/tests-green.log`.
Сценарии: malformed/token/seq/boot/child mismatch, source digest/unknown payload,
partial frame/missing phase, path escape/alias loop/special/hardlink/changed bytes,
size/command-output/deadline guards, один fork/initial+delayed без actual fork,
closed loader names/digests, exact original bootcmd subsequence, missing authority
до любых side effects, pin drift до host commands, no SSH/scp/package dispatch,
partial cleanup/ref/base/PID refusal, repeated identity check before KILL,
retention failure still stopping own mocked VM.

Все process/fork/signal calls в этих новых тестах mocked; unit file fixtures и
cleanup fixtures — свои небольшие TemporaryDirectory, автоматически очищены.
Тесты не являются actual QEMU/PID1/serial proof. Runtime gate остаётся отдельным
root review; сейчас никаких claims о bootstrap cause или full A4.
