# A4 diagnostic-only boot: early SSH gate не достигнут, failed overlay сохранён

Same af worktree/PR181 после `b9db90a12318cb5bf8f572afd9725c2b82dcec89`.
Root decision `a4-bootstrap-diagnostic-root-decision.json` прочитан и сверён:
exact source b9, APPROVED implementation и **один** diagnostic boot,
900s total/240s SSH, тот же signed147 base и прежние ограничения. Решение/SHA
сохранены отдельно. Это отличающийся bootstrap stage после известного terminal
run1; не unchanged retry. Дополнительных boot attempts нет.

## Actual результат

**BLOCKED_AT_EARLY_SSH_NONCE_GATE**. Один diagnostic boot, 48 bounded readiness
checks за исходный 240s deadline, exit255 connection reset. CLI exit **1**.
Guest unit graph/journal и runtime five-mask verification **NOT_REACHED**.
Package/SCP transfers=0, package transactions=0, live hook proof=0.
Early SSH не объявлен package-ready. Причина time-sync **не объявляется** без
unit graph; source142/package defect этим результатом не доказан.

В отличие от предыдущего run1, pre-shutdown serial показывает продолжающийся
cloud-init **Network Stage** и `Wait Until Kernel Time Synchronized`, а не
достигнутый config stage. Это реальное различие наблюдения после переноса SSH
в bootcmd; оно не доказывает, что time-sync блокирует SSH. Синхронное ожидание
bootstrap service job также нельзя исключить, но без actual unit graph это
только гипотеза. Существование nonce-файла внутри guest не проверено, поскольку
SSH связь не установлена.

## Source/tests до boot и граница режима

Добавлен `tools/a4_bootstrap_diagnostic.py`, общий Driver разбит на cloud_config,
prepare_guest, workload, success_result и collector hooks. Базовая hook-only
ветка сохраняет прежнее поведение; diagnostic subclass:

- использует те же пять masks **до** SSH/nonce в bootcmd;
- удаляет runcmd duplicates, сохраняет package_update/upgrade=false;
- допускает только фиксированные readonly identity/unit/journal/process
  команды, nonce read и bounded cleanup poweroff;
- отказывает любому guest pacman/daemon-reload/stop timesync, SCP и product
  callbacks до вызова SSH; workload вообще не вызывает package phases;
- возвращает `package_ready=false`, `live_package_hooks=false` даже при будущем
  успешном readonly capture;
- сохраняет единственный diagnostic overlay вне TTL и при неуспехе, и при
  успешном capture. Никакой clock/network/time-sync workaround не добавлен.

Тесты были запущены RED до source и GREEN **до** boot: 20 A4 tests PASS,
Ruff check/format PASS. Проверены masks-before-SSH/order/no-runcmd,
non-package workload, forbidden commands, wrong identity/decision/attempts.
Это unit controls, не actual runtime five-mask/guest-readiness PASS.

Actual source copies `executed-common-source.py` и
`executed-diagnostic-source.py` сохранены outsideTTL до boot; их SHA совпадают
с исполнявшимися файлами. Source142/MTREE4/controller/recipes/product ELFs
не менялись; новых Rust/Go builds/targets/download/deps нет.

## Evidence, ownership, cleanup

Run ID `70c91647e7d432157aeb8f917273b040`.
Raw evidence/private output (mode0700):
`local/validation/a4-package-hooks-142/70c91647e7d432157aeb8f917273b040/`.
Public source/decision/command bindings, summary и SHA receipts:
`docs/evidence/a4-bootstrap-diagnostic-142/run1/`.

Под task-root exclusive BSD directory flock с первого использования создан
fresh overlay; cached base — под shared flock. Source/tree/tool/input SHA и
GPG exact primary повторно проверены до bootstrap. Никаких host writable
mounts, host services/settings/caps или product activation попыток. Runtime
masks проверены только тестами/seed construction; не подменяются actual proof.
Actual guest package binaries или userdata не передавались.

Перед shutdown сохранён serial snapshot. SSH poweroff не установил связь;
после bounded wait проверены own QEMU UID/PID/starttime/argv, выполнен TERM и
reap exit0. Failed overlay скопирован **после закрытия QEMU и до own-root
cleanup** outsideTTL. Own task удалён, cwd/exe/fd/maps references=[], base SHA
после `794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003`.
Ни автоматического approval rejection, ни fallback/rephrase не наблюдалось.

Retained failed overlay, **не удалять как transient**:
`local/validation/a4-package-hooks-142/70c91647e7d432157aeb8f917273b040/blocked-disk.qcow2`
— 35.12 MiB; exact bytes/SHA и readonly backing info в summary/receipts/raw
`retained-overlay-info.json`. Сохранены также pre-shutdown и final serial,
commands/PIDs/exits, QEMU argv, root decision, executed sources и failure.
Guest filesystem/journal остаются внутри этого overlay; их извлечение не
выполнялось и доступность содержимого не обещается. Host SSH private key не
экспортирован. Два исходных schema4 state каталога mode0700 не читались,
не копировались в VM и не менялись; f0 preservation proof остаётся отдельным.

## Concrete NEXT root

Разрешённый один diagnostic boot исчерпан известным terminal result.
Следующий конкретный input для readonly расследования — **сохранённый qcow2
выше + exact signed base**; нового boot для этого автоматически не требуется.
Root может отдельно назначить bounded offline unit/journal extraction с
проверенным existing readonly tooling и own scratch/locks. До такой проверки
никакого нового tool/input, filesystem mount/replay/repair, дополнительного
VM boot либо изменения SSH/time-sync/network ordering не выполнялось.

Если выбирать ещё один boot, сначала нужен root review точного bootstrap
изменения по actual unit definitions; `--no-block`/service-ordering не
добавлялись наугад и не выдаются за доказанный fix. Package phases по-прежнему
запрещены в diagnostic-only режиме. Чтобы вернуться к A4 live hooks, нужны
actual guest guards и отдельный допустимый package-ready gate; readonly SSH
сам по себе этого не даёт. FullA4/finalLinux=false, factory CLOSED;
Runtime16/N5/callers и final release freeze не затронуты. No nested/watch.
