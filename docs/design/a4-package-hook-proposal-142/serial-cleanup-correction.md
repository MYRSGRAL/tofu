# A4 serial preflight: два cleanup finding

База исправления: `51a89e6c3e0a2f12ece1fdc802de665c814a6606`.
Независимый preflight `01daaeb7` нашёл unbounded host communicate/reap и
collector alarm с `os._exit`, обходящий cleanup текущего systemctl.
VM, collector и systemctl не запускались; разрешения на boot по-прежнему нет.

Host использует nonblocking capped reads обоих pipes: суммарно 65 536 bytes,
не более 45 секунд capture и отдельный bounded wait после SIGKILL (1 секунда).
Bootstrap subprocess создаёт собственную session. При timeout/overflow группа
сигналится до reap leader; после освобождения PID group signal не выполняется.
Held-open pipe не требует ожидания EOF. При неизвестном reap scratch сохраняется,
cleanup остаётся UNKNOWN; новый boot этим не разрешается. Generic host exec API
не добавлен: набор bootstrap команд прежний.

Alarm initial/delayed теперь поднимает TimeoutError: исключение проходит через
finally `_command`, который убивает и ожидает собственный systemctl. Доставка
alarm во время read и wait проверена без collector execution. Финальный alarm
отключается при выходе; initial/delayed budgets и bootstrap ordering сохранены.

Два адресных теста на frozen baseline завершились assertion RED; на исправлении
6 новых regressions и весь serial-набор — **43 PASS**. Проверены overflow,
удерживаемый fixture pipe после выхода собственного child, bounded waits при
reap timeout, handler и обе точки alarm внутри `_command`. Tests используют один
собственный Python child и mocked subprocess collector; VM/seed/overlay нет.
Ruff check/format и diff-check — PASS. Это control-flow/process-boundary proof,
не реальный VM reap и не доказательство причины прежнего bootstrap failure.

Логи RED/GREEN сохранены вне TTL:
`/home/myrs/projects/yonik/local/orchestration/evidence/a4-serial-cleanup-red.log`,
`/home/myrs/projects/yonik/local/orchestration/evidence/a4-serial-cleanup-tests.log`.
Future source SHA в implementation contract обновлены; пример authority остаётся
NOT A GRANT. Перед любым boot нужны narrow recheck и отдельное root-решение
для окончательных source SHA/argv. `fullA4=false`, `finalLinux=false`.
