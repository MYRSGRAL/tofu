# Read-only ROOT capture: исходный код, runtime CLOSED

База решения: accepted main `0f69d5e` + `c411b32`. Этот модуль — независимая
утилита доверенного outer guest fixture, не новый production observer/RPC/service.
В этой задаче выполняются только stdlib mock-тесты. Никаких guest/root capture,
регистрации, kernel/host запросов, настоящих subprocess или packet flows здесь нет.

## API для ROOT/W3

`collect(stage, nonce, case, *, backend=None) -> dict` измеряет фиксированный набор.
`read_capture(stage, nonce, case, *, backend=None) -> dict` читает sealed capture.
`compare_cleanup(baseline, connected, stopped) -> dict` сравнивает три измерения.
Импорт не выполняет запросов. CLI намеренно отсутствует: существующий доверенный
outer fixture вызывает API после отдельно выданного пользователем разрешения.
Нормальный DesktopHost/session/flow client принадлежит W3; модуль его не заменяет.

Стадии строго `baseline`, `connected`, `stopped`. Case — ASCII `[A-Za-z0-9_-]`,
1–96 символов; nonce — 64 lowercase hex, свежий для каждого guest/run.
Ни caller JSON, ни env сами по себе не дают полномочий.

До любого `/proc`, systemd или net чтения проверяются euid=0 и фиксированные:

- `/run/yonik-acceptance/authorization.json`;
- `/run/yonik-acceptance/input.json`.

Все компоненты пути открываются через dirfd/O_NOFOLLOW, каталоги root-owned и
без group/other write. Конечные sealed файлы — regular, root-owned, один hardlink,
без любых write bits, с совпадающими dev/inode/size/mtime/ctime до и после чтения.
Размер JSON/stdout не более 1 MiB. Root outer fixture является trust anchor:
он размещает эти файлы только после отдельного USER grant и проверки sealed
fixture, свежести nonce, source/build/manifest mapping и запуска правильного guest.
Новый RootGrant/native permit/privileged service не требуется. Компрометированный
root вне модели: readonly mode не является криптографической подписью.
Существующий repository `authorization.json` CLOSED не изменён и этому формату
не соответствует, следовательно не разрешает capture.

Точный auth JSON (никаких дополнительных ключей):

```json
{"schema":"ROOT_CAPTURE_AUTH_V1","execution":"USER_GRANTED_GUEST_CAPTURE","nonce":"<64hex>","case":"case-1","stages":["baseline","connected","stopped"],"input_sha256":"<SHA256 exact input bytes>"}
```

Точный input JSON:

```json
{"schema":"ROOT_CAPTURE_INPUT_V1","source":"<Git commit 40hex>","manifest_sha256":"<sealed fixture manifest SHA256 64hex>"}
```

Это доверенные provenance labels outer fixture, не actual measurement ELF mapping
сами по себе. Модуль отдельно измеряет SHA256/размер пяти фиксированных immutable
ELF и собственного установленного source-файла
`/usr/lib/yonik-acceptance/collect_observations.py`. Outer fixture обязан сверить
эти значения с reviewed sealed manifest. Здесь manifest не регистрируется.

Схема результата `schema: int = 1`, `status: "ACTUAL" | "UNAVAILABLE"`.
При недоступности результат только `{"schema":1,"status":"UNAVAILABLE"}`;
исключения/сырые тексты команд не экспортируются. Пример структуры ACTUAL
(метапеременные ниже описывают типы, это НЕ сохранённое измерение):

```text
provenance:
  source: Git40hex
  manifest_sha256, collector_sha256, authorization_sha256: SHA256hex64
  kernel_boot_id: lowercase UUID
  case, stage, nonce: string
  collector_pid: positive int
observations:
  collector: {pid: int, start_ticks: int, uids: [int,int,int,int]}
  units:
    yonik-helper.service / yonik-worker.service:
      main: null | {pid, start_ticks, uids}
      invocation: "" | systemd InvocationID hex32
      cgroup: "" | /system.slice/<canonical unit>
      populated: null | 0 | 1
      members: [{pid, start_ticks, uids}]
  network:
    links, addresses, routes, rules, routes6, rules6, nft: [canonical JSON object SHA256hex64]
  artifacts:
    /usr/lib/yonik/yonik-helper: {sha256: SHA256hex64, size: int}
    /usr/lib/yonik/yonik-worker: {sha256: SHA256hex64, size: int}
    /usr/bin/yonik-desktop: {sha256: SHA256hex64, size: int}
    /usr/lib/yonik/engines/sing-box: {sha256: SHA256hex64, size: int}
    /usr/lib/yonik/engines/xray: {sha256: SHA256hex64, size: int}
  tracked_processes: [{pid: int,start_ticks: int,absent: bool}] # stopped only
inference:
  native_report_and_roles: SOURCE_INVARIANT
```

`read_capture` принимает только фиксированный
`/run/yonik-acceptance/capture-<stage>.json`, проверяет sealed layout, вложенные
типы, PID provenance, nonce/case/stage. Outer fixture сохраняет результат туда
атомарно и делает root-owned readonly; модуль сам не пишет файлов. Отсутствующий,
нечитаемый или malformed capture никогда не становится ACTUAL. Обычный driver
может читать эти файлы без root при доступных read/traverse permissions, но не
может создавать grant или capture. Для криптографически независимого provenance
за пределами trusted guest нужен outer trusted capture, не detached caller JSON.

## Измерения и ограничения

Whitelist: systemctl list-units/show (фиксированные canonical properties),
ip JSON link/address/route/rule show, nft JSON list ruleset. Никаких caller argv,
shell, StopUnit, mutations, capabilities, BPF, ptrace, install или foreign kill.
Каждый child имеет deadline 5 s и capped stdout; finally убивает только созданный
child при необходимости, reap и закрывает pipe. Ошибка reap/timeout/overflow —
UNAVAILABLE. Общий набор конечен; ELF capped по 64 MiB, PID members до 256.
Root использует только уже имеющиеся guest permissions; denied read — UNAVAILABLE,
никакой CAP_SYS_PTRACE не предполагается.

Proc identity перечитывается, canonical properties и cgroup membership/populated
перепроверяются. Это bounded snapshots, не атомарная kernel транзакция. На stopped
читается sealed connected capture, проверяется отсутствие каждой отслеженной
PID/starttime пары (PID reuse отличается); permission denied не равен отсутствию.
Это наблюдение отсутствия, **не waitpid чужого процесса**. Статус reap как факт
родительского wait не утверждается. Убежавшие/ненаблюдавшиеся потомки неизвестны.
Пустая либо отсутствующая cgroup не подставляется по MainPID=0; недоступная
cgroup честно делает capture UNAVAILABLE.

Network экспортирует только hashes actual объектов, не URI, credentials или raw
Secret Service. Сравнение возвращает actual `network_restored`,
`canonical_cgroups_empty`, `tracked_processes_absent`, `own_network_delta`
(connected минус baseline), `foreign_missing` (baseline минус stopped).
Delta не доказывает ownership: outer fixture обеспечивает изоляцию изменений.
Counters/параллельные network changes могут консервативно дать различие; это
не PASS и не причина игнорировать foreign changes. Сбой/unknown не успех.
`escaped_descendants` всегда UNAVAILABLE. Native full report/roles остаются
SOURCE_INVARIANT; байты report не выдумываются. Полная Linux acceptance,
packet-policy PASS и весь cleanup этим инструментом не объявляются.

Проверки: Ruff 0.17.4, `python -B -m unittest discover -s
 docs/proposals/n5-guest-acceptance -p test_collect_observations.py` и тот же
запуск с `python -B -O`. Все OS/child операции тестов только mocked.
