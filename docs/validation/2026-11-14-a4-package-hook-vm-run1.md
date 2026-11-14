# A4 hook-only VM run1: реальный bootstrap blocker до transactions

Same PR181 после `6a02d91e14a28b1161f928425b4452d3acfe0aa7`. Root decision
`/home/myrs/projects/yonik/local/orchestration/proposals/a4-package-hook-vm-root-decision.json`
подтверждает reuse ранее authorized147 scope, review R2 e4fc8f5f CLEAN. Driver
проверил exact reviewed source и allowed set; это standing authorization,
не новый user grant. Решение и его SHA сохранены в authorization binding.
Никакой automatic approval rejection в этом run не наблюдалось.

**Результат: BLOCKED_BEFORE_GUEST_PREFLIGHT_AND_PACKAGE_TRANSACTIONS.**
Одна VM, один bootstrap attempt; ни одной package transaction, product/witness
build, download, native/TUN/N5/caller операции. Новый attempt/fallback не делался.
Live hook proof не достигнут; full A4 и finalLinux false, factory CLOSED.
Source142/MTREE4 и все старые data/runtime records неизменны.

## Что реально исполнилось

Новый `tools/a4_package_hook_vm.py` сверил pinned inputs, source/tree,
frozen controller tooling, real old/candidate versions/vercmp, отсутствие
`.INSTALL`, exact union из пяти product unit paths и существующий normal-user
KVM access. Signed base image перепроверен GPG: exact VALIDSIG primary
`1B9A16984A4E8CB448712D2AE0B78BF4326C6F8F`. Base directory shared flock,
новый task root mode0700/exclusive directory flock с первого использования.

Сформирован fresh qcow2 overlay, readonly backing, pinned CODE/copy VARS,
unique cloud-init/SSH key и reviewed q35/KVM/2CPU/4096MiB argv. Network строго
`restrict=on`, только own localhost SSH forward. VM PID **1962525**, UID1000,
starttime/argv/nonce связаны в `active.json`. Нет host writable mounts,
host services/settings/deps/privilege changes. Guest standard boot infrastructure
не подменялось fake systemd/executables.

Bootstrap cloud-init запрашивает пять masks до product install, затем SSH/nonce
в runcmd согласно reviewed plan. **Actual runtime verification этих masks не
достигнута** — она не объявляется PASS по seed content.

## Точный остановивший gate

В пределах исходного SSH deadline **240 секунд** выполнены 48 bounded read-only
readiness attempts на ту же VM/port/key. Все завершились exit255:

```text
kex_exchange_identification: read: Connection reset by peer
Connection reset by 127.0.0.1 port 35865
```

Nonce через SSH не получен; существование самого nonce-файла внутри guest не
проверено. Driver остановился с `TimeoutError: SSH readiness deadline240s`.
Command window 243.88s плюс bounded cleanup — внутри общего 900s бюджета.
Python guest driver и package archives **не передавались** в VM; в commands нет
ни `guest.py`, ни pacman transaction. Поэтому реальные guest hook owners/
semantics, reloadable DBus, monitor readiness, Manager.Reload correlation
остаются **NOT_REACHED**. Ни manual Reload, ни chroot-skip PASS нет.

Retained serial доказывает:

- standard guest systemd загрузился; Network is Online;
- cloud-init local/network/config stages завершены;
- старт `Wait Until Kernel Time Synchronized` виден, его завершение не видно;
- завершения cloud-init final stage в serial нет.

**Наблюдение ≠ доказанная причинная связь.** Offline network/time-sync ordering
является конкретной гипотезой задержки final/runcmd, но guest journal/unit graph
не получены. Нельзя объявлять это дефектом source142/package либо уверенно
объяснять недоступность SSH только NTP. Доказанный blocker — отсутствие
обязательного bootstrap SSH/nonce gate под reviewed configuration.

## Cleanup и предел evidence

SSH collect/poweroff также получили exit255, ошибки сохранены. После bounded
poweroff attempt driver дождался 20s, проверил own QEMU UID/PID/starttime/argv,
отправил TERM и reap: QEMU exit0. `qemu.log` содержит actual SIGTERM source.
References cwd/exe/fd/maps к task отсутствуют; root удалён. Base SHA **после**:
`794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003` — неизменен.
Flock удерживался до проверки/cleanup.

Raw evidence outsideTTL (mode0700 directory):
`local/validation/a4-package-hooks-142/87a62b01813b0600bdc73eb69b882c5a/`.
Там input bindings/GPG command results, actual executed driver source с exact
SHA, intended-but-not-transferred guest source, все commands/PIDs/exits,
QEMU argv/serial/log, failure/cleanup. Public summary/receipts:
`docs/evidence/a4-package-hooks-142/run1/`.

Предел: guest journal и failed-bootstrap overlay **не сохранены**; run1 cleanup
считал bootstrap overlay восстановимым и сохранил serial/commands/bindings.
Поэтому нет оснований утверждать точный time-sync dependency. Для будущих run
owned harness теперь сохраняет overlay **при любом** failed VM run (не только
unknown transaction), делает serial snapshot до shutdown. Регрессия проверена
без запуска новой VM. Уникальные original schema4 copies не касались VM:
`diagnostic-a4-backup-cause-5thkiqwh` и `retained-a4-nonempty-qdcjm959` остаются
mode0700, contents не читались/не менялись.

Ещё один честно сохранённый предел: первая CLI обвязка caught failure и вернула
process exit0, **при `failed=true`/failure.json**, это не success. До любого retry
исправлен только CLI contract: failed → exit1, targeted RED→GREEN. Exact executed
run1 source восстановлен и SHA совпал с pre-run binding; новый code не выдан
за уже исполненный. Новых VM attempts после исправлений нет.

## Следующий bounded contract — только после root review

`docs/design/a4-package-hook-proposal-142/bootstrap-diagnostic-next.json` —
конкретный **предложенный**, не реализованный/не исполненный delta:
перенести уже допустимые guest sshd/nonce bootstrap команды в bootcmd после
тех же пяти masks, чтобы получить read-only unit graph/journal **до** зависимого
cloud-init final. Только диагностика, **package transactions запрещены**;
не ждать `cloud-init status --wait` для diagnostic-only endpoint, не объявлять
его guest fully ready. Снять actual time-sync/cloud-final/sshd dependencies и
journal, сохранить failed overlay новым cleanup, остановить own VM.
Никакого stop/mask time-sync, изменения часов, outbound/NTP исключений, новых
inputs/privileges или weakening PID1/package preconditions не предлагается
исполнять без отдельного root решения. Этот bootstrap delta не покрыт exact
reviewed6a argv/stage contract автоматически; **вторая VM не запускалась**.

Существующий NONEMPTY userdata proof f0 уже CLEAN и не повторяется. Для A4
по-прежнему нужны actual guest preflight и пять live hook phases **после**
устранения/квалификации реального bootstrap prerequisite; original rollback
interpretation не изменена. Final release source/artifact freeze остаётся
отдельным root gate.

## Проверки кода

Новые host/guest drivers — fixture tooling. 14 A4 Python tests PASS, Ruff
check/format PASS. Targeted controls: wrong decision/identity/hook/guard,
неполный/двойной Reload, product JobNew, CLI failure exit, terminal approval-rejection guard и сохранение failed
boot overlay. Unit monitor examples не выдаются за actual Manager.Reload PASS.
Production Rust/Go/source/recipe не менялись; новых cargo targets нет.
