# Интеграция reviewed helper leaves

База: `5fd6153839a8c55f67ba31ddbe9ae2d403328a1f` (`origin/main`).
Обычные merge без ручных разрешений конфликтов:

- `233bf8ee9359f7059de0ebf7f9066a21cf288c84`: main + PR #164 `a4a165b231e53d64ee250cb98631c450b997131a`, включая PR #159 `540b0eb3c8d9e2592c09445c7ad5fe225a705a38`.
- `d5a75bf1aec771915982ffded718f7d849e2eaaf`: результат + PR #157 `249dcff321a5d513a0cc1891575deaab05bd9f77`, включая PR #153 `c7da3812aee35ae8d8fe730ae1f95caefdf3cf34`.

Shared union: helper mod сохраняет main и добавляет run_authorization; nsfs
сохраняет kernel_probe и добавляет tun; Cargo.lock добавляет libc только nsfs.
Оба remerge diff пусты. Новых production вызовов producer/authority нет.
Full Session factory и ProductionPermit::mint закрыты; legacy start/path без
bound queue сохранён. RunAuthorization one-shot/revoke, exact credential/report
v3 без fallback, Backend Arc<BoundTun>, exact metadata, single CLOEXEC duplicate,
confirmed-stop cleanup и ownership lease перенесены без дополнительных правок.
Namespace legacy clone gap и approval остаются pending. PR #161 не включён.
Ready-ветки, package preflight `9154e16582920656b0bbc44f26a70da8514d3cbc`
и чужие cross-UID untracked сохранены.

## Проверки и ограничения

MSRV Cargo/Rust 1.85.1, --locked, jobs=1 через with-rust-slot; внешний target
под shared flock собственного transient root. Evidence вне TTL:
`/home/myrs/projects/yonik/local/orchestration/evidence/helper-reviewed-integration/`.

- fmt --all --check и strict clippy --no-deps -p yonik-linux -p yonik-nsfs --lib -- -D warnings: PASS.
- Реальные существующие isolated user/net/mount controls: queue CLOEXEC/error,
  Journal custody/actual ATTACH и Backend unknown-stop/confirmed-stop cleanup: PASS.
- Первый параллельный lib run: 499 PASS, 1 process-wide FD-count race в v3 test,
  34 ignored. Последовательный direct-ELF run подтвердил v3, но 3 host tests
  не собрали fixture через host rustc launcher; требуется явный toolchain PATH.

Known ignored scopes не являются PASS: Linux lib 34 + nsfs 1 + installed helper
7 = 42. Дополнительно installed worker содержит 2 VM-only ignored tests.
Privileged installed-systemd, cross-UID, production admission и native forwarding
campaign не выполнялись. Полный workspace/all-targets clippy здесь не заявляется.

Evidence содержит parents, source diff, remerge diff, source/ELF hashes,
полные логи и inventory ignore reasons; host environment failures сохранены.

Финальный cargo lib run с явным MSRV PATH, --test-threads=1: Linux 500 PASS /
34 ignored, nsfs 3 PASS / 1 ignored. Installed helper 1 PASS / 7 ignored,
installed worker 1 PASS / 2 ignored, resource FD receiver 9 PASS; native bundle
1 ignored (pinned cores/user-net namespace). Итого в выбранных suites 45 ignored,
из них 42 относятся к оговорённому known scope; 3 kernel controls отдельно PASS.

Cleanup: cargo clean под exclusive root flock без global slot; transient root удалён.
Логи, source/ELF/log hashes сохранены вне TTL. Чужие untracked SHA256 совпадают.
