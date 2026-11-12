# Read-only instrumentation

База реализации: `919f06853798143c762a8e87acfbccf86687c49d` (main
`9ec9da07` + неопубликованная зависимость PR #104,
`821fdcf8bb2e9d6ab7585bae3146f7852007a32c`).

`availability.json` — единственная короткая проба доступности собственного PID,
не энергетический benchmark. GUI и Rust не запускались. P-01/P-02/P-03 и выбор
toolkit этим артефактом не закрываются; требования performance.md сохраняются.

Новый additive `process.resource_counters` содержит readings и intervals.
Старые `hardware_power=null` и main_thread_context_switches_proxy сохраняются.
Последний остаётся proxy главного потока. Wakeups — сумма
`se.statistics.nr_wakeups` всех наблюдавшихся потоков только при enabled
schedstats; сравниваются TID/starttime до/после чтения и между samples.
Изменение набора потоков инвалидирует интервал. Короткоживущие потоки между
samples могут быть не замечены, отключение/включение schedstats между samples
также нельзя исключить. Это ограничение sampling, а не доказательство отсутствия
пробуждений. Baseline один; новый процесс получает новый baseline через identity.

Счётчики energy_uj публикуются отдельно по зонам (µJ), батареи energy_now —
в µWh. Package, psys, subdomains, MMIO и обычный RAPL не суммируются:
они могут перекрываться и не доказывают total platform energy. Одинаковые sysfs
aliases дедуплицируются по resolved path. Отсутствие, отказ доступа и неверный
формат имеют структурированные причины, без текста исключений/серийных номеров.

Дельты требуют положительного интервала <=120 секунд, одинакового источника
и диапазона. Спад — reset_or_wrap, не автоматическая коррекция переполнения.
Для положительной powercap-дельты дополнительно нужен max_power_range_uw,
исключающий полный оборот за интервал и ограничивающий дельту. Без него
wrap_bound_unknown; нулевая энергия — quantization, не доказательство нулевого
потребления. Батарея учитывается только при Discharging на обеих границах
чтения. Смена питания между samples не наблюдается непрерывно.

Production roots фиксированы; собственный PID либо прямой ребёнок того же UID.
Fixture roots доступны через приватную фабрику. Чтение файла <=64 KiB,
<=512 потоков, <=64 entries за обход powercap, итог <=128 кандидатов,
<=32 power supplies. Никаких sysctl/tracefs/perf/sudo и записей в sysfs.

Первичные контракты:

- [Kernel powercap](https://docs.kernel.org/power/powercap/powercap.html)
- [Kernel power supply ABI](https://www.kernel.org/doc/Documentation/ABI/testing/sysfs-class-power)
- `kernel/sched/debug.c`, `proc_sched_show`, CONFIG_SCHEDSTATS;
  `kernel/sched/stats.h`, schedstat_enabled — поля не стабильный ABI:
  отсутствие nr_wakeups возвращает unavailable.
