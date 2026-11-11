# P-04: offline импорт и компиляция Linux

База: `2a36135bd7002037753cdfd6d4b0f595de8939a7`. Связи:
[performance P-04](../../specs/performance.md), FR-06/07 (импорт),
FR-11/20/21 (политика), FR-18 (ресурсы) в
[requirements](../../specs/requirements.md).

Полный P-04: **FAIL / остаётся открытым**, production не изменён.
Scoped импорт и компиляция: PASS. `profile_catalog_bench` уже покрывает
SQLite-каталог/поиск/избранное; здесь он намеренно не повторён.

## Что реально выполнено

`import_compile_resource` принимает ограниченный offline URI fixture (seed 407),
вызывает `import_bytes`, требует точного количества и отсутствия issues,
компилирует и сериализует **каждый** профиль через `compile` для выбранного ядра.
1 000 и 10 000 уникальных синтетических Trojan endpoints; профили не
отбрасываются и не подменяются одним успешным экземпляром. Сериализация
последовательная, буфер переиспользуется, конфиги всех профилей не накапливаются.

Отдельно один профиль получает `compile_system_policy` с 10/100/1 000
правилами executable AND CIDR → Block. Для каждого правила проверяются
точный executable, CIDR, действие и количество native app-rules.
Это компиляция конфигурации системного входа, без создания TUN/сокетов,
запуска engine или изменения host. Доменные AND-условия проверяются отдельно:
отказ обязателен и не считается успешной компиляцией.

Для каждой из 12 комбинаций: отдельный warmup-child и три measured-child.
`runs.json` содержит число реально imported/compiled, import/compile/policy ns,
wall, CPU, peak RSS **этого child** через Linux wait4, размеры конфигов и
число правил. `summary.json` содержит медианы и максимальный RSS из трёх runs.
`manifest.json` записан **до** runs: seed, SHA256 исходников/fixture/бинарника,
базовый HEAD, platform, loadavg, имена процессов для контекста concurrent builds.
Сборка release — pinned Rust 1.85.1, jobs=1. Wall включает создание процесса и
шаг опроса 5 ms; короткие фазы следует оценивать по внутренним ns и CPU.

## Ограничения и конкретный probe для root

- Контрольная спецификация включает 20 подписок, 100 000 доменов и 50 000 CIDR.
  Здесь URI подаются одним импортом; 20 snapshots/commit этим прогоном не покрыты.
- Тест `control_rule_set_is_explicitly_refused` подаёт 100 000 доменов и
  50 000 CIDR в настоящий `compile_policy` обоих ядер: **InvalidPolicy**.
  `routing::validate_policy` ограничивает суммарные selectors значением 100 000;
  системный IP-вход также отклоняет domains. Поэтому время такого отказа не
  выдано за быстрый успешный compile. Десятикратный полный rule-set не доказан.
  Предложение root: согласовать трактовку внешних rule sets из §4 и selector
  budget/API компилятора до реализации отдельного представления больших наборов.
  Изменять compiler/domain в этом ownership запрещено.
- `cancel.json`: принудительная остановка отдельного resource-child и его reap,
  с CPU/RSS. Timeout 1 ms инициирует kill при следующем опросе. Это проверка
  process boundary, **не** кооперативная отмена import/compile API и не UI Stop.
- Рост размеров правил проверяется точным native app-count, без раскрытия
  декартова произведения для одного executable и одного CIDR. Эти измерения
  не доказывают асимптотику всех production hotpaths. В используемом compile
  профили обрабатываются последовательно; `validate_policy` использует HashSet
  ID и линейный обход selectors. Большие составные политики остаются вне PASS.
- Peak RSS не является PSS группы и не доказывает возврат памяти долгоживущего
  controller. Никаких утверждений об энергии, Android или physical UI gate.

## Воспроизведение и проверки

Сборка требует собственного transient root с shared flock, глобального
`with-rust-slot`, jobs=1 и CARGO_TARGET_DIR внутри этого root:

```text
cargo +1.85.1 build --release --locked -p yonik-engine --example import_compile_resource
python tools/perf_import_compile.py <binary> <evidence-directory>
python -m unittest discover -s tools/tests -p test_perf_import_compile.py
cargo +1.85.1 test --locked -p yonik-engine -p yonik-import --all-targets
cargo +1.85.1 clippy --locked -p yonik-engine -p yonik-import --all-targets -- -D warnings
cargo +1.85.1 fmt -p yonik-engine -- --check
```

Runner ограничивает stdin 8 MiB, вывод 64 KiB, CPU 60 s, wall 60 s;
ошибка child не преобразуется в успешный замер. Никаких новых зависимостей.
Python: ruff format/check 0.16.6 из существующего cache, без установки.
Логи проверок приложены. Нужный release binary сохранён вне TTL в
`local/p04-import-compile/import_compile_resource` текущего worktree;
его SHA256 в manifest. Transient Cargo target очищается штатным cargo clean.
