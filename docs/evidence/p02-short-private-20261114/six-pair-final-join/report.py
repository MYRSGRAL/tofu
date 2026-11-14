"""Русский отчёт из comparison.json; только чтение данных и stdout."""

import json
from pathlib import Path


def main() -> None:
    """Печатать детерминированный отчёт без runtime измерения."""
    d = json.loads((Path(__file__).resolve().parent / "comparison.json").read_text())
    print("""# P02: объединение шести пар, DATA ONLY

Статус: SIX_PAIR_DATA_JOIN_REVIEW_REQUIRED_BOUNDED_OBSERVATIONAL.
Пять старых пар переиспользованы по R-A, sing-box №2 заменена одной новой парой.
Это компонентный observational dataset для независимого review. Финальный Linux
P02, physical, GPU, энергопотребление и 8h PASS не установлены.

## Состав и метод

Retained: sing-box old №1/№3 + replacement №2; Xray old №1/№2/№3.
12 окон, по 31 sample, 372 retained точки; порядок всегда full→engine.
Вся старая sing-box pair №2 исключена из статистик. Её engine half сохранена
в raw вместе с noisy full. Overlap controls — 148,3366525 с; offsets примерно
55,04–203,37 с, выведены из mtime−elapsed, не из per-row wallclock.
Ничего не вычиталось из duration, samples не обрезались, raw не перезаписан.
Иная host-нагрузка не исключена; отсутствие известного targeted overlap не
означает noise-free измерение.

CPU — разность суммарных utime+stime ticks / 100 (соответствие всем исходным
summary проверено); память — сумма группы, конец окна. Также JSON содержит
начало, изменение конец−начало и диапазон sample points. Paired delta —
full endpoint минус engine endpoint соответствующей пары, не разность медиан.
USS=null: Private_Clean не сохранён; Private_Dirty — отдельная метрика,
а не точный USS. Context switches — суммы изменений только main-thread
voluntary/nonvoluntary, proxy, не wakeups. Wakeups=null.
Sampler исключён из всех групп. Full включает GUI с интегрированным product tray
и один engine; отдельного product tray PID нет. Private SNI host, private bus,
SecretService, HTTPS/peer infrastructure не включены. GUI+tray PSS/FD/threads
приведены отдельным process row в JSON; стоимость собственно tray не изолирована.

## Все пары

Память в КиБ, CPU в секундах; F/E = full/engine, Δ = F−E.

| Core/пара | CPU F/E/Δ | PSS F/E/Δ | Private_Dirty F/E/Δ | Context Δ F/E (vol; invol) |
|---|---:|---:|---:|---|""")
    for p in d["pairs"]:
        f, e, delta = (
            p["windows"]["full"],
            p["windows"]["engine"],
            p["full_minus_engine"],
        )

        def ctx(w: dict) -> str:
            return str(
                tuple(
                    w["main_thread_context_delta_proxy_not_wakeups"][k]
                    for k in ["voluntary_ctxt_switches", "nonvoluntary_ctxt_switches"]
                )
            )

        print(
            f"| {p['core']} №{p['repeat']} ({p['origin']}) | {f['cpu_s']:.2f}/{e['cpu_s']:.2f}/{delta['cpu_s']:+.2f} | {f['pss_end_kib']}/{e['pss_end_kib']}/{delta['pss_end_kib']} | {f['private_dirty_end_kib']}/{e['private_dirty_end_kib']}/{delta['private_dirty_end_kib']} | {ctx(f)}/{ctx(e)} |"
        )
    print("""
## Разброс трёх paired differences

Медиана [min; max], абсолютные величины. При near-zero CPU отрицательная разность
не доказывает экономию, положительная не устанавливает регрессию; fixed-order,
квантизация 0,01 с и host noise не позволяют такой причинный вывод.

| Core | Δ CPU, с | Δ PSS, КиБ | Δ Private_Dirty, КиБ |
|---|---:|---:|---:|""")
    for core, a in d["aggregate"].items():
        cells = []
        for key in ["cpu_s", "pss_end_kib", "private_dirty_end_kib"]:
            s = a["paired_full_minus_engine"][key]
            cells.append(f"{s['median']:g} [{s['min']:g}; {s['max']:g}]")
        print(f"| {core} | " + " | ".join(cells) + " |")
    print("""
## Source, config и cleanup

Старое measurement source: 35408d531a86ec858e1d2fc3489e8e092fda5d27,
driver SHA256 4e51d4676132da15e957479c84b404c3e3be0aa9adb31a766d796cfb0ff47ce7.
Validator/control source: 51fd15a3a94c1e5e2ca400e1dcd7ccf26438715e,
driver SHA256 508f837200ffa07aaaca6e88cc296c41dd789bf6dfbd9aa054d6bd8c43536f23.
Новая pair: selector b1b9ce10525082039ecbdf87a6abbf33fc11449f + тот же 51fd driver;
closure manifest 21f90aa0776ba26870c0e691c7479d9a122efbd64e961ae4beac8d781b51033e.
GUI artifact build source_head 6d1905eeff0e541300c8adf281445f9048904d38
(dirty build ID) не следует путать с measurement source; ELF остаётся тем же.
Per-pair SHA config, GUI/observer/core ELF, sample SHA и source binding — в JSON.
Captured full actual-config и engine actual-config byte-identical в каждой паре;
capture engine-binding связывает PID, stdin pipe, config и ELF. Новый config hash
отличается от старых: новые owned ports/credential; между repetitions byte equality
не требуется. Повторной компиляции для engine baseline нет по reviewed driver.

Старые 51fd posthoc результаты и lifecycle trace подтверждают сохранённые
PID/starttime/namespaces/descendants, Connected→hide→Stop, exit/reap/rebind,
campaign exit 0 и отсутствие сохранённых product identities на финальной сверке.
Это не lifetime held-pidfd и не retroactive fail-fast. Короткоживущий child целиком
между samples не исключён ни старым, ни новым snapshot-набором.
Новая pair: native handle 21523, exit 0, 900,041053880 с full и 900,031132714 с engine;
terminal receipt: все 8 записанных owned PID отсутствуют, failure/cleanup fallback
нет. Result SHA256 eb0442a7b0665e2b529e07724fe826586210c270c730732f5f7a20d31785bf0b.
Нормальные pidfd Stop/teardown/reap/rebind checks — в unchanged reviewed driver.
DATA join не запускает runtime и не повторяет cleanup/control probes.

## Покрытие исходных пунктов performance.md

| Пункт | Read-only evidence и статус | Остаётся открытым |
|---|---|---|
| §5 idle CPU | 12 group CPU и paired differences за ≥900 с, renderer внутри GUI | Не нормативный release budget PASS; tick/noise ограничения |
| §5 PSS/private dirty | Полная группа и отдельный GUI+tray/engine; per-pair deltas | Точный USS, изолированная память product tray, controller/helper budget |
| P-02 short connected-idle | 3 matched pairs/core при R-A, actual hidden GUI, config bytes binding | Physical Plasma/tray и полноценный system TUN не измерены |
| P-02 отсутствие второго engine | Exact group identity/descendants в 372 retained snapshot points; новая pair held-pidfd | Непрерывный process-spawn trace отсутствует |
| P-02 no optional probes | Reviewed idle завершён без HTTPS fixture requests; actual wrong-CA→Limited/tls/noHTTP и trusted-positive controls | Не исчерпывающий packet trace всей host-сети |
| P-02 group overhead | CPU/PSS/Private_Dirty и context proxy full−engine; sampler/infrastructure отдельно | Энергия, GPU, реальные wakeups неизвестны |
| §6 версии/условия | ELF/source/config hashes, old conditions.json: kernel, AC, powersave; dev/offscreen/private SNI | Новые power/thermal conditions не записаны, старые не переносим; battery отдельного набора нет |
| §6 config matrix | Только LocalSocks/no-app-rules, два pinned core | FakeDNS/ru-direct/WG/app-rules/editor matrix, traffic/sleep сценарии |
| §7 минимум три/разброс | По три retained pairs/core; min/median/max; при near-zero абсолютные значения | Это qualified reuse, не три новых непрерывно guarded/noise-free запуска |
| P-02 8h; G-POWER/release | Не покрыты этим набором | Выделенный physical baseline, 8h energy, AC/battery и release acceptance |

Existing r2 controls: actual GUI wrong-CA→Limited/kind=tls/noHTTP и trusted-positive
на обоих core; actual pinned engine SIGSTOP + driver abort→TERM/KILL→exit/reap/rebind,
6 tests PASS по сохранённому completion. Это engine_only teardown fault control,
не production GUI cleanup_fault API; не новый long-window guard для old dataset.

## Воспроизведение и целостность

В каталоге находятся aggregate.py, report.py, comparison.json, report.md,
inputs.json, verification.json и artifacts.sha256. inputs.json фиксирует SHA256
297 исходных файлов, включая excluded raw, controls и closure. Агрегатор только
читает JSON/файлы, не импортирует runtime source, не запускает процессы/сокеты.
Он сверяет все inputs до вычисления и каждый потреблённый binding, summary и sample.

Из этого каталога (stdout можно направить в отдельный новый файл для сравнения):

```sh
/usr/bin/python3 -B aggregate.py
/usr/bin/python3 -B report.py
sha256sum -c artifacts.sha256
```

verification.json фиксирует детерминированность JSON/report, независимый пересчёт,
проверки отрицательных случаев и неизменность исходных inputs после join.
Рабочие деревья, prod source, accepted raw/controls/closure, PR и ветки не менялись.
Данные готовы к независимому review и последующему переносу root в существующий 187.
""")


if __name__ == "__main__":
    main()
