# Deadline без прерывания subprocess ownership

Recheck `1b07dd3b` для `988c463` обнаружил два незащищённых окна:
SIGALRM-исключение до входа в try/finally и внутри самого cleanup. Ранее
проверенные read/wait точки не покрывали весь lifetime. Boot не выполнялся.

Использована другая модель: handler только устанавливает expired flag.
Он не бросает исключение и не вызывает `_exit`, поэтому не прерывает Popen,
pipe setup, kill/wait или close. Весь полученный subprocess находится внутри
try/finally, stdout закрывается также при ошибке cleanup. Command/snapshot
проверяют flag и monotonic deadline в контролируемых точках; после истечения
следующие commands не запускаются, результат — UNKNOWN. Перед fork delayed
snapshot initial deadline проверяется ещё раз. Budget не продлевается.

Это cooperative deadline, не обещание прервать неотменяемый kernel I/O из Python.
Для такого случая окончательным fence остаётся исходный host QEMU deadline и
остановка собственной VM. Exceeded budget не создаёт успешного evidence.

Оба выявленных окна воспроизведены mock-регрессиями на `988c463` (kill/wait не
достигнуты). После изменения весь serial-набор: **45 PASS**. Существующий test
UNKNOWN masks получил действительный будущий monotonic deadline вместо числа
100 секунд uptime; прежние assertions не ослаблены. Только собственный Python
child и mocks; collector/systemctl/VM не запускались. Ruff и diff-check — PASS.

Implementation contract содержит новые source/test SHA и явную deadline model.
Пример authority остаётся NOT A GRANT. Перед одним boot нужен новый preflight
этой модели и отдельное root-решение exact argv/source hashes; full A4 не заявлен.
