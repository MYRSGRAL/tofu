# Сравнение Linux UI

Изолированный стенд Qt Quick/CXX-Qt, GTK4 и Slint на общем Rust actor. Он не
заменяет действующий GUI tofu и не использует реальные профили, SQLite, keyring
или proxy core. Состав окна и обмен JSONL описаны в [PROTOCOL.md](PROTOCOL.md).

Production MSRV остаётся прежним. Для стенда используется Rust 1.97.0: версии
GTK4 0.11.5 и Slint 1.18.1 требуют Rust 1.92. Зависимости стенда закреплены его
собственным Cargo.lock, основной workspace не меняется. Все кандидаты собираются
одним toolchain с одинаковым release profile. Noto Sans должен быть доступен
через fontconfig; системные библиотеки GTK4/Qt Quick требуются отдельно.

Сборочный каталог должен находиться вне checkout, под общим lock корня задачи:

```sh
mkdir -p /var/tmp/codex-myrs/transient/tofu-ui-comparison
flock -s /var/tmp/codex-myrs/transient/tofu-ui-comparison \
  env CARGO_TARGET_DIR=/var/tmp/codex-myrs/transient/tofu-ui-comparison/target \
  CARGO_BUILD_JOBS=2 cargo +1.97.0 build --manifest-path bench/ui/Cargo.toml \
  --release --bins --locked
```

Сравнивать следует реальные окна последовательно в одной незаблокированной
сессии, после завершения всех сборок. Offscreen не подтверждает энергопотребление.
Различия системных контролов и renderer необходимо отражать в результатах.
Счётчики созданных, живых и видимых строк различаются; Slint сообщает недоступные
live/visible как null. Прокрутка подтверждает запрос и кадр, но фактическая
геометрия требует отдельной визуальной проверки.

Полный план принятия решения находится в
[плане UI benchmark](../../docs/superpowers/plans/2026-11-09-linux-ui-benchmark.md).
Короткий smoke подтверждает протокол и работоспособность стенда. Он не выбирает
победителя и не закрывает G-UI/G-POWER, проверки других DE, accessibility,
fractional HiDPI, clipboard, tray и connected-idle с engine baseline.

Варианты внешнего вида независимо от toolkit находятся в
[design/ui-variants.html](../../design/ui-variants.html).
