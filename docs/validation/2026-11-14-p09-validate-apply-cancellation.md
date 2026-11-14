# P09: actual normal Host Validate/Apply cancellation

Узкий executable witness на базе `c079e9d`, не полный P09/native PASS.
Оба pinned core прошли normal DesktopHost с production key path через отдельный
Secret Service. Installer, exact approved unsafe patch, ABI и зависимости не менялись.

## Consumer и контракт стенда

`LifecycleGate` существует только с nondefault `test-support`. По умолчанию
HostConfig и Cancellation не содержат gate. Объект принадлежит одному Host;
stage и oneshot удержание одноразовые, mutex не удерживается через await.
Нет env override статуса, synthetic CheckedConfig/PreparedPlan или shipped delay.

- Validate: настоящий pinned config-check child уже создан внутри
  `process::config_check`, stdin ещё принадлежит parent. Это не Preparing observer.
- Apply: `LocalSession::start` уже выполнил prepare/apply и создал настоящий
  runtime child; barrier расположен перед записью конфигурации.
- Gate получает actual session generation и PID. При отмене biased select
  возвращает Cancelled, затем существующий terminate/stop выполняет cleanup.
- Test-support ACK наблюдается только после успешного join producer. Стенд
  требует совпадение с generation достигнутого consumer и обычный snapshot
  Disconnected/can_stop=false. Production wire Stop не принимает request_id;
  фиктивный wire ACK/request_id не добавлен.

## Итоговая матрица

| Core | Validate cancel/control | Apply cancel/control |
| --- | --- | --- |
| sing-box 1.14.4 | PASS / Connected | PASS / Connected |
| Xray 26.10.31 | PASS / Connected | PASS / Connected |

Все 8 случаев выполнены. На boundary записаны PID/starttime/ELF hash,
принадлежность child host и held pidfd. До Stop exit-oracle отвергает живой
child (8 RED canaries); это проверка чувствительности cleanup oracle,
не RED старого product build. Контроль release без отмены действительно даёт
Connected, то есть barrier не заменяет consumer искусственным статусом.

Во всех 4 cancel случаях совпали generation boundary/ACK, pidfd стал ready,
PID исчез из /proc после reap, children host опустел, порт доступен для rebind,
конкретный parent stdin pipe закрыт. Проверка относится к этому owned pipe,
не является общим доказательством отсутствия FD leaks.

Затем тот же Host действительно подключает replacement. Только после его
Connected освобождается старый barrier: send отклонён, поскольку отменённый
consumer уже завершён. Identity и единственный child replacement сохраняются;
его Stop снова подтверждает отсутствие children и rebind. Старое продолжение
не может публиковать Connected; искусственное stale SessionUpdate не вводилось.
Во всех восьми сохранённых snapshot Stop/replacement/Preparing revision остаётся 1,
selected_profile остаётся 1 (`state-preservation.json`). Это cancellation consumer,
не моделирование native late RPC completion или произвольного kernel effect.

## Receipt, изоляция и evidence

Итоговый каталог вне TTL:
`/home/myrs/projects/yonik/local/evidence/p09-cancellation-private-ss/`.
`source.tar`, freeze (9479 hashes), Cargo metadata, raw vv/JSON,
`receipt.json`, retained executables/rlib/rmeta, profiles, transcript и
`matrix/result.json` сохранены. Source/ELF были скопированы до runtime.
Все frozen inputs повторно сверены после runtime/clippy без изменений.

Rust 1.85.1, locked, exact x86_64-unknown-linux-gnu, jobs1, root BSD flock,
общий with-rust-slot, отдельный CARGO_TARGET_DIR. Receipt проверяет actual
linked linux_raw instances example → desktop → linux → nsfs → rustix и
standalone test → nsfs → rustix. Для нового example verifier получил закрытый
выбор `--example`; прежний default и backend checks сохранены.
Standalone parent-death test только собран для receipt, не запускался.

Retained candidate ELF SHA256:
`351a6c6f342aab29b104c43c5c0dc8f5562104c7102092a59023c55b1bd5052f`.
Receipt принят **до** первого выполнения этого ELF. Seccomp0, TracerPid0,
NoNewPrivs0 у launcher и каждого Host; без LD_PRELOAD/LD_AUDIT и errno emulation.
Обычные getpid/getppid и неизменённый approved installer. Новых unsafe callsites нет.

Для каждого случая отдельные HOME/XDG, private bus и foreground gnome-keyring
из уже подготовленного runtime P05; owner org.freedesktop.secrets проверяется
до запуска Host. Host использует subscription_master_key, ephemeral override
отсутствует. Только собственные loopback VLESS/HTTPS fixtures и прежние pinned
hashes. Нет TUN, host install/services, VM, crossUID или factory execution.

Предварительные прогоны в `local/evidence/p09-cancellation/` использовали
унаследованный от #200 ephemeral store и не являются итоговым private-SS proof.
Исправленный итоговый source был заново заморожен, собран и получил новый receipt;
эти предварительные результаты не подменяют итоговую матрицу.

Воспроизведение после отдельного принятого exact-build receipt:

```sh
python crates/yonik-desktop/tests/p09_cancellation.py \
  /retained/p09_cancellation /owned/pinned-engines /owned/evidence /owned/private-ss-runtime
```

## Проверки и пределы

Scoped example clippy с test-support и `-D warnings`, cargo fmt --all --check,
ruff check/format и pure receipt negatives (обычный Python и -O) проходят.
`final-binding.json` подтверждает unchanged inputs, retained hashes и отсутствие
ссылок процессов текущего UID на target перед cleanup. Cargo clean удалил
4234 файла / 2.2 GiB; собственный пустой transient root удалён.

SIGKILL/Stop proof #200 не повторялся. Native System/helper/network effects,
full P09, parent-death interleaving injection и произвольные late external effects
остаются OPEN. Этот результат не повышает остальные readiness gates.
