# P-04 delta: 20 subscription snapshots / commit

База delta: `4cae2942854d680a56ef46eace10fff8c52a6e43` (PR92).
[Shared-contract proposal](shared-contract-proposal.md) вынесен отдельно;
полный P-04 по-прежнему не закрыт. Catalog bench не повторён.

Новый `subscription_snapshot_resource` использует production
`Subscription::{begin,stage,commit}` и
`EncryptedSubscriptionRepository::{create_source,commit_snapshot,load,close}`.
Двадцать controllers/sources удерживают по одному snapshot. Собственная
0700 ephemeral directory/SQLite DB, синтетический ключ только для этой БД,
никакой сети/keyring/TUN/user data. `rusqlite` уже является зависимостью Linux
crate; read-only observer читает только counts/length/page_count, не payload.
SQLite writer-lock используется только в cancellation probe своей БД.

Для 1000 и 10000 суммарных distinct профилей (50/500 на подписку): четыре
раунда полного обновления endpoints. В каждом раунде 20 successful commits,
ровно N staged imports и N повторно импортированных профилей из committed DB;
каждый disk-imported профиль реально compile+serialize для Xray и sing-box.
Результаты содержат 80 successful normal commits, 4N staged/disk imported,
по 4N compiled на ядро и сумму размеров сериализованных конфигов каждого ядра.
Внутренний prepare_profiles repository также выполняет production import;
он не выдан за дополнительное измеренное число parsed records.

На каждом раунде actual SQL counts обязаны быть ровно 20 snapshots и N
profiles; endpoints между раундами различаются. Favorites в fixture отсутствуют:
исторические избранные — отдельный production контракт, здесь не проверяются.
Проверяются N retained parsed servers в controllers и bounded retained source
bytes; VmRSS читается после каждого раунда. Runner требует ≤192 MiB и разброс
≤8 MiB в раундах 2–4: это явный scoped критерий plateau, не математическое
доказательство постоянной памяти для произвольного uptime. SQLite page_count и
суммарные encrypted snapshot bytes сохраняются для проверки reuse/bounded disk.

Один warmup-child + три measured-child для каждого N; CPU и peak RSS через
wait4 per-child, wall включает startup/DB/four rounds/negative probes/cleanup.
100 ms намеренного ожидания в cancel probe входит в wall и не является работой
импортёра. Время каждого раунда отдельно в ns. RSS child включает runtime,
SQLite и retained reports; RSS процесса-пускателя перед exec может задавать
нижнюю границу wait4 maxrss. Это не PSS и не energy/UI gate.

`manifest.json` записывается до запуска: source SHA256, алгоритм fixture/seed407,
хэши каждого source/round/count fixture, binary SHA256, platform/load/process names.
Warmup/повторы не запускают Cargo. Глобальный Rust slot, flock transient root,
jobs1, pinned1.85.1 используются только для сборки/проверок.

Negative actual probes:

- malformed body и stale revision должны дать InvalidInput/Conflict;
  затем `load` подтверждает byte-for-byte прежний body и revision.
- Abort future во время заблокированного commit **не** кооперативная отмена.
  Probe требует увидеть дополнительный commit после abort (revision 5),
  иначе падает как недоказанный. Этот дополнительный commit учитывается
  отдельно в `cancel_probe`, не в 80 normal commits. `load` ожидает освобождения
  repository gate; значит worker закончил перед финальным закрытием БД.
  Конкретные callsites и минимальный API proposal находятся в соседнем документе.

Воспроизведение: собрать example `subscription_snapshot_resource` с
`cargo +1.85.1 build --release --locked -p yonik-linux --example subscription_snapshot_resource`
под установленными slot/flock/jobs1/CARGO_TARGET_DIR ограничениями, затем:

```text
python tools/perf_subscription_snapshots.py <binary> <evidence-directory>
python -m unittest discover -s tools/tests -p test_perf_subscription_snapshots.py
cargo +1.85.1 clippy --locked -p yonik-linux --example subscription_snapshot_resource -- -D warnings
cargo +1.85.1 fmt -p yonik-linux -- --check
```

Новый runner использует bounded pipe-reader для output64KiB, отдельный
file-size limit64MiB для ephemeral SQLite, CPU/wall60s, reap и очистку
каталога своего child при timeout. Ошибка/refusal не становится success. После успешных runs
собственная ephemeral БД удаляется; бинарник сохраняется вне TTL в
`local/p04-subscription-snapshots/subscription_snapshot_resource`, Cargo target
очищается штатным cargo clean. Форматы native rule sets обоснованы первичными
ссылками в proposal; их поддержка pinned engines ещё требует actual gate.


P2 original head: обычные warmup/measured runs в `perf_import_compile.py`
теперь требуют cancelled=false, exit=0, полный result с точными counts и
валидными метриками; только отдельный cancel probe принимает отмену.
Regression вызывает actual main с mocked child: all-timeout и один failed
normal child дают RuntimeError (CLI nonzero); positive control сохраняет ровно
48 successful rows и принимает 49-й explicit cancel. Red/green logs приложены.
Принятые original 48 actual runs и их manifest/raw logs не перезаписывались.
