# Настоящий cold capture в контролируемом namespace

База: a1775c0df353c00e0c42b55a70844f13260f6ec9 (PR161).
Отдельный evidence harness; production-семантика, лимиты и зависимости не меняются.
PR157/153/155 не изменяются. Probe v2 не используется.

## Что установлено про Limit

Retained `local/orchestration/single-rpc/final-tests.log` базы содержит четыре
`cold unavailable=Limit witnesses=1`. Это доказывает прохождение первого
socket witness, но не различает счётчики PID/task/FD. В исходном capture после
первого witness есть MAX_PROCESSES=256, суммарный MAX_TASKS=256 и
MAX_FDS_PER_TASK=256. Сам лог не даёт права назвать конкретный из них причиной
старого host отказа; retained syscall trace среди доступных evidence не найден.
Host campaign не повторялась. Production bug не заявляется.

Новый отрицательный контроль с единственным PID, двумя test threads и 257
дополнительными собственными FD возвращает Limit после одного witness за
1.248125 ms. Здесь PID/task ниже порога, поэтому это именно FD-ветка.
Положительный fixture меняет охват /proc честной PID/mount namespace binding,
а не пропуском процессов в production scanner.

## Контракт и root guard table

| Guard | Сохранённый контракт / проверка |
| --- | --- |
| UID / PID | map-current-user, UID 1000; тест требует PID 1 и единственный PID в /proc |
| namespaces | user/mount/PID/network namespace; новый procfs, lo поднимается до exec |
| capabilities | permitted/effective/inheritable/ambient обнулены setpriv перед binary |
| executable | реальный retained /proc/1/exe; wire путь сверяется с /proc/self/exe |
| cold | Broker::new, cache пуст, last_cold отсутствует; snapshot/proof не подставляются |
| socket | реальные TCP/UDP IPv4/IPv6; Flow.cookie=None |
| witness | ровно 3 в thread, обслуживающем настоящий LOOKUP; не cache-hit witness=1 |
| bounds | прежние FD/process/rate/TTL/deadline/retry; RPC измеряется send→receive <20 ms |
| admission | verified_helper_bootstrap остаётся закрытым; test-only session fixture |

Namespace настройка использует только capabilities собственного user namespace;
после настройки у retained binary они нулевые. Отказ unshare/ip/setpriv является
ошибкой harness, не Skip/PASS. Все тесты требуют `1 passed` в выводе.

## Результат

Финальный bounded запуск: TCP4 7.702977 ms, UDP4 2.151409 ms,
TCP6 10.799744 ms, UDP6 3.353096 ms. Все четыре дали wire Known и три witness,
остаточный TTL 10.30–18.66 ms. Это локальный функциональный proof, не percentile
или обещание времени на произвольной host process population.

Пять именованных gates PASS: cold wire (четыре комбинации), FD Limit +
pre-cancel, реальный UDP reuseport ambiguity, in-flight cancellation/teardown
(1.949109 ms), retained stale/replaced/deleted executable + epoch rejection.
Последний existing контроль также содержит cache-hit subcase; он не считается
cold evidence и отдельная cache-hit campaign не запускалась.

Повтор: `python tools/cold_kernel_gate.py /absolute/retained/tests /absolute/evidence`.
Evidence вне TTL: `local/evidence/cold-kernel/` в task worktree: retained binary,
SHA256/source binding, namespace preflight, final-run/run.json и raw logs.
