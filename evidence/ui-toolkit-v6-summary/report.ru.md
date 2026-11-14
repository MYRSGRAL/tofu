# Сводка toolkit v6

Критерий: только программные наблюдения завершённой Xvfb-кампании; 18 ячеек, 36 фаз, 19 cleanup receipts. Повторные измерения не запускались.

Матрица: Qt/GTK/Slint × 1000/10000 строк × 3 повтора; visible 900 с + hidden 900 с, шаг 5 с. Страница 200 строк.

CPU: Δcpu_ticks между первым и последним sample; ticks/s = Δticks / Δmonotonic_ns × 1e9; CPU seconds = Δticks/100. PSS/USS: медиана 181 отсчёта, затем медиана трёх повторов. Память в таблице — MiB (2²⁰ байт). Xvfb исключён, учитывается только leader.

| Toolkit | Строки | Фаза | ticks | ticks/s | CPU с | PSS MiB | USS MiB | Δ native counter |
|---|---:|---|---:|---:|---:|---:|---:|---:|
| qt | 1000 | visible | 0 | 0.000000 | 0.000 | 52.689 | 46.398 | 0 |
| qt | 1000 | hidden | 0 | 0.000000 | 0.000 | 52.694 | 46.398 | 0 |
| qt | 10000 | visible | 0 | 0.000000 | 0.000 | 55.547 | 46.426 | 0 |
| qt | 10000 | hidden | 1 | 0.001111 | 0.010 | 45.532 | 31.273 | 0 |
| gtk | 1000 | visible | 25 | 0.027777 | 0.250 | 76.688 | 69.348 | 577 |
| gtk | 1000 | hidden | 0 | 0.000000 | 0.000 | 76.707 | 69.348 | 0 |
| gtk | 10000 | visible | 24 | 0.026666 | 0.240 | 73.563 | 64.094 | 569 |
| gtk | 10000 | hidden | 0 | 0.000000 | 0.000 | 73.567 | 63.988 | 0 |
| slint | 1000 | visible | 12 | 0.013333 | 0.120 | 98.149 | 93.105 | 5 |
| slint | 1000 | hidden | 5 | 0.005555 | 0.050 | 98.145 | 94.629 | 0 |
| slint | 10000 | visible | 13 | 0.014444 | 0.130 | 98.171 | 92.098 | 5 |
| slint | 10000 | hidden | 4 | 0.004444 | 0.040 | 98.158 | 92.059 | 0 |

Действия: latency_ms = (ack received − send started)/1e6 из frozen runner. Для каждой ячейки p50/p95 — nearest-rank по 12 разным действиям (включая sensitivity-scroll, hide/show), затем медиана трёх таких квантилей; это смешанный набор, не распределение повторов одного действия. В JSON также есть все 12 индивидуальных latency и p50/p95 каждого label по трём повторам (p95 при n=3 = максимум).

| Toolkit | Строки | Медиана action p50 мс | Медиана action p95 мс |
|---|---:|---:|---:|
| qt | 1000 | 6.616 | 9.248 |
| qt | 10000 | 8.015 | 10.414 |
| gtk | 1000 | 11.112 | 46.635 |
| gtk | 10000 | 13.627 | 56.579 |
| slint | 1000 | 13.667 | 31.209 |
| slint | 10000 | 13.704 | 30.099 |

Границы достоверности:

- Qt: QQuickWindow.frameSwapped; GTK: GdkFrameClock.after-paint (paint cycle, не presentation); Slint: RenderingState.AfterRendering. Это разные native counters, не сопоставимый FPS. Интервалы Probe включают overhead, settling не вычтен.
- X11/Xvfb, software Qt, GTK cairo, llvmpipe. Нет вывода о Wayland, реальном дисплее, GPU или физической энергии. Недоступные wakeups/GPU/power сохранены null; context switches не переименованы в wakeups.
- Фоновая сборка наблюдалась в 1813 из 6516 sample conditions. Это same-UID comm snapshots, не полный census нагрузки; причинная коррекция не выполнялась.
- Cleanup подтверждён сохранёнными receipts и root gate, не новым осмотром процессов. Отсутствие observed descendants не исключает краткоживущий fork/exit между сканированиями.
- Все 28 frozen файлов перепроверены по manifest. Host pins сохранены как исторические входы, текущие /usr и процессы не исследованы. SHA и read-only snapshot обеспечивают cooperative integrity, не adversarial sealing и не герметичную ОС.
- source_tree/native_build_subtree обозначает только bench/ui subtree; frozen_native_commit — ссылка на native source, не утверждение об идентичности всего root checkout. compiled_inputs сверены с native_inputs; binary hashes — с frozen manifest. Архив исходников и checks log лишь исторические pins, отсутствующие в этом snapshot, не объявляются проверенными.
- Frozen runner/doc plan не задаёт нормативного правила выбора победителя. Автоматический выбор toolkit и повышение G-UI/G-POWER не выполнены. P02 — отдельная работа.

Точные SHA-256 всех входных JSON, каждой cell.json и frozen файлов находятся в summary.json. Raw samples не имеют самостоятельных внешних pins: их целостность покрывается SHA cell.json; фазы проверены структурно и по sample identity, monotonic order, CPU monotonicity, Probe counters и контракту.

Воспроизведение: `python evidence/ui-toolkit-v6-summary/aggregate.py` из подготовленного worktree. Только чтение указанных входов и запись двух сводных файлов рядом со скриптом.
