# P04A: explicit mixed compiler RED

Harness: `crates/yonik-linux/examples/p04a_mixed_red.rs` (example, не default test).
Production API, capability и DTO не изменены. Два synthetic source: exact
`a.example.invalid`, `203.0.113.0/24`; app `/opt/fixture/app-0`.

Путь: ResourceCatalog::build → native encoder → EncryptedRuleSetStore →
commit → RuntimeLease::sealed_resources. Перед mixed проверкой каждый компонент
отдельно проходит compile_bound_policy и seal_bundle; mixed resources читаются
из sealed memfd, bytes/count/digest сверяются с actual catalog manifest.

Mixed: catalog.bind → compile_bound_policy → routing::compile_policy_resources.
Обычный compiler path отклоняет app через Unsupported(ProcessAttribution).
Без app Names+CIDR даёт RequiredRealResolution, IpOnly+domain — DomainsInIpOnly.
Это RED текущего compile-only API, а не доказательство необходимости flow-specific
provenance для будущего data artifact. Выбор минимального нового контракта — root.
Реальный activation gate отдельно; фиктивные owner/capability evidence не создаются.

Negative fixture validity: неверные source digest/count отклоняются до публикации;
неверный policy digest и неполный binding отклоняются; indexes проверяют match/miss;
sealed resources readonly; неверная revision даёт exact InvalidPolicy, конфликт
портов exact ProbePort. Любая ошибка на этих шагах не считается нужным RED.

Exit **42** означает, что fixture валидна и все восемь actual mixed вызовов
дали exact ожидаемые Unsupported (четыре app, четыре без app). Exit **1** означает
ошибку fixture, неожиданную причину отказа либо неожиданно успешный mixed compile.
Ни один из кодов не является положительным mixed PASS. Default CI harness не запускает.

Сборка: Rust 1.85.1, offline/locked, jobs1, with-rust-slot (max2 slots), shared
flock собственного transient root, отдельный CARGO_TARGET_DIR. Реальные команды/exit/source SHA256 находятся в [binding.json](binding.json),
stdout/stderr трёх запусков приложены рядом. Actual mixed exit=42;
--invalid-revision даёт exit=1/InvalidPolicy, --invalid-port даёт exit=1/ProbePort.
Synthetic каталоги negative cases после сохранения logs удалены. Performance baseline не повторяется.
Source snapshot, binary и полные build logs сохраняются в `local/p04a-mixed-red/`
этого worktree вне TTL; committed hashes связывают их с evidence. Никаких native
cores, TUN, crossUID, сетевых probes или host settings.
