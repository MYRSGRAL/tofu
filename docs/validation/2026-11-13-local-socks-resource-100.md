# P-05: компонентный witness LocalSocks

Источник production-компонентов: accepted `origin/main` `128a070`.
Это hash-bound component witness DesktopHost/Core, не final full-stack seal.
После freeze требуется повтор только релевантного нового source. G-POWER на
физическом устройстве и Android остаются deferred. TUN/admission/cross-UID
campaign не выполняется; pending probes v2 не применяются и не вызываются.

## Воспроизведение

Из корня checkout собрать только `local_socks_resource`:

```sh
flock -s "$TASK_ROOT" /home/myrs/projects/yonik/local/orchestration/with-rust-slot \
  env RUSTUP_TOOLCHAIN=1.97.0 CARGO_TARGET_DIR="$TASK_ROOT/target" CARGO_BUILD_JOBS=1 \
  cargo build -p yonik-desktop --example local_socks_resource --features test-support -j1
python crates/yonik-desktop/tests/local_socks_resource.py "$BINARY" "$ENGINES" "$EVIDENCE"
```

`TASK_ROOT` — собственный transient-каталог, созданный до flock; binary и evidence
сохраняются вне TTL. Только один core и один цикл одновременно. Стенд использует
existing `engine_host.https_server/peer`, approved local-import API с encrypted
synthetic store, private D-Bus и loopback. Пользовательские профили, Plasma,
network rules и production hooks не изменяются. GUI не включён.

На каждом core — один долгоживущий DesktopHost, 100 connect/HTTPS/Stop циклов.
Connected подтверждается production HTTPS probe к известному fixture через
pinned VLESS peer; HTTPS fixture должна наблюдать запрос в каждом цикле.
Stop ACK дополняется pidfd readiness, проверкой PID/starttime/reap и bind того же
порта с SO_REUSEADDR (учёт TIME_WAIT). Никакого kill неизвестных PID.

## Измерения и ограничения oracle

Сохраняются baseline 0, все connected host/core samples и все stopped samples
1…100. Прогрев — циклы 1…10. После прогрева считаются min/max/end/delta и линейный
slope каждого счётчика: FD, threads, RSS, PSS, private dirty и RSS `[heap]`.
`[heap]` не включает все allocator arenas: это отдельный узкий показатель;
PSS/private dirty нужны для mmap/прочей памяти. RSS разных процессов не суммируется.

RED: оставшийся ребёнок либо FD/threads выше baseline цикла 10. Memory REVIEW:
одновременно end delta > 8 MiB и slope > 32 KiB/cycle для PSS/private dirty/heap.
Это triage-порог стенда, не бюджет спецификации и не доказательство отсутствия
любых утечек. Все меньшие положительные slope и delta также публикуются.
Проверка oracle — собственный fixture со 101 реально удерживаемым `/dev/null` FD;
тот же oracle обязан дать RED. Production fault injection отсутствует.

EventLog имеет bounded capacity 2000; его заполнение и allocator retention могут
давать ограниченный рост. Стабильные постоянно живущие runtime/store/D-Bus
services входят в baseline, а не вычитаются произвольной поправкой. Сохраняется
первая bounded страница logs после 100 циклов; она не заявляется полным event trace.
В evidence также есть все cycle PID/starttime, reap/rebind/HTTPS ACK и closed ACK.

Соответствие `docs/specs/performance.md` P-05 ограничено LocalSocks DesktopHost/Core
и этим synthetic workload. P-03 GUI, переключение ядер внутри одного host,
энергопотребление, долгий uptime, нагрузка правил и полный G-POWER не покрыты.

## Результат 2026-11-13

Оба core: 100/100 циклов, closed ACK, все HTTPS/reap/rebind проверки пройдены.
Oracle канарейки: RED. Итог: BOUNDED_COMPONENT_WITNESS.

| Core | FD | Threads | Δ RSS KiB | Δ PSS KiB | Δ private dirty KiB | Δ heap RSS KiB |
|---|---:|---:|---:|---:|---:|---:|
| sing-box | 15 | 4 | 64 | 64 | 32 | 12 |
| xray | 15 | 4 | 80 | 80 | 48 | 12 |

Положительный рост памяти наблюдается и не объявляется ни доказанной утечкой,
ни доказанным allocator noise. Полные slopes/min/max/end находятся в evidence.

Evidence вне TTL: `/home/myrs/projects/yonik/local/evidence/local-socks-resource-100/`.
Бинарник — debug без GUI; результаты не сравниваются с абсолютным GUI budget.

Проверки: focused build и Clippy `-D warnings`, Rustfmt, Ruff; workspace-wide
и all-targets проверки не запускались. Штатный cargo clean удалил 2.2 GiB;
собственный transient-каталог удалён. Production-файлы и зависимости не изменены.

- SHA256 `local_socks_resource`: `38a574919a779773737bf8d16a8cc5dc97c826b4a3fb53f51a16ded45ae849ed`
- SHA256 `sing-box.raw.json`: `da73ac38b8e2a7fea31e00f33a7d1c4fe50f62b06bf6543fc770d7ebeed3a8fc`
- SHA256 `xray.raw.json`: `e783c598f8ba761f26ac323cef42cc60055feb25df7f21c740216c34fa58feae`
- SHA256 `canary.json`: `d4ac3487de18415a4ee2958f0e535aa0de3b81614a278cb94ec38cc7e330f309`

## Узкое закрытие P2 peer oracle — 2026-11-14

Добавлен независимый negative control тем же сохранённым DesktopHost binary:
собственный pinned peer приостановлен через SIGSTOP с подтверждением WIFSTOPPED.
При этом direct curl с проверкой CA и `--noproxy "*"` получает HTTPS 204.
Затем новый DesktopHost получает тот же профиль и настоящий `connect`:
обязан выдать только `ready`, завершиться с Timeout без Connected и не достичь
HTTPS. Если Connect обойдёт peer, этот oracle отвергнет его успешный ACK/exit.
SIGCONT выполняется в finally; затем новый host проходит Connect/HTTPS/Stop,
reap/rebind и closed. Контроль выполняется для обоих cores.

Режим `--peer-gate-only` выполняет только эти controls и один восстановленный
positive на core. Кампания 2×100 не повторялась; исходные resource evidence и
бинарник не изменены. На обоих cores direct=204, denied=exit 1 / Error: Timeout,
HTTPS при отказе не достигнут; после восстановления positive и closed PASS.
Это закрывает чувствительность oracle к direct bypass, не расширяет прежние
границы component witness и не доказывает общий native admission.

Evidence: `/home/myrs/projects/yonik/local/evidence/local-socks-resource-peer-negative/`.
Binding фиксирует обновлённый Python harness, неизменённый Rust source и старый
retained binary; пересборки Rust не было. Ruff format/check и diff-check PASS.
