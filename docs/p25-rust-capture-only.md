# P25: частичный Rust capture

Точный opt-in вызов после отдельного trusted freeze нового release-id:

```sh
python tools/release_controller.py collect --release-id p25-rust-capture-01 --rust-capture-only
```

Это описание CLI, а не разрешение запускать сборку. В рамках этой реализации
выполняются только Python-тесты с synthetic runner; native builder, Go,
компиляторы, Qt query, ELF, engine, services, VM и activation не запускались.

Контроллер проверяет frozen source/tooling/profile, восстанавливает собственный
source snapshot, выполняет существующие Rust commands и retain pipeline.
Source, ELF и CXX capture records сохраняются с существующими fsync-гарантиями
до `cargo clean` собственного target в `finally`. Ошибка retention/fsync
оставляет target; production wrapper сохраняет task и пишет failure.json.

После Rust retention выполнение останавливается перед native builder,
восстановлением upstream source archives, source offers и упаковкой.
Обязательные A4/native/source gates полного collect этим не выполнены.

Успех означает только `state=RUST_CAPTURED_ONLY`, запись
`rust-captured-only.json`, `final_linux_acceptance=false`. Поля frozen source,
toolchains, controlled_env, commands, elf и build_capture связывают результат
с фактическими входами. Статус MISSING внутри build_capture остаётся MISSING:
частичный результат не доказывает полноту CXX attribution или final Linux.
Нет collected.json, package.tar, payload и утверждения release readiness.

Partial record не читается как full record. Даже перенос его в collected.json
отклоняется проверкой состояния. Повторный partial/full collect того же ID
запрещён: нужен новый trusted freeze и новый ID. Существующие fixed reuse ID
в partial режиме отклоняются как MISSING до запуска команд: старые Rust stages
не объявляются новым capture и не подтверждают полную source/toolchain/env
closure. Нового механизма reuse этот режим не добавляет.

Без флага collect сохраняет прежний полный pipeline и обязательные gates.
Флаг допустим только с collect, произвольных argv/env/command inputs нет.
