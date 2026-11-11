# P07: матрица модельных доказательств

База: `b9e5071784065c1fcb4813608d195421a146f80c`. Это handoff остатка,
а не объявление P07/A-04/A-07/A-08 завершёнными. Production не менялся.

Требования: routing-and-tray §§2–7, data-and-contracts §§1/7,
networking §§2/4; validation-and-roadmap A-04/A-07/A-08.
`D` ниже — crates/yonik-domain/tests, `E` — crates/yonik-desktop/tests/editor.rs,
`B` — crates/yonik-desktop/src/backend.rs. Имена тестов — точные поисковые refs.

| Leaf требования | Доказательство на базе / дополнение | Граница |
| --- | --- | --- |
| AND executable/CIDR, OR списков, отсутствие owner, точный путь вместо basename | D/policy.rs:116 `source_and_destination_are_and_values_are_or` | Caller path синтетический; owner не проверен |
| AND domain/CIDR, OR exact/suffix и IPv4/IPv6, user/preset, отрицательные значения | D/dns_policy.rs:477 `смешанные_группы_соединяются_через_и` | Чистая модель |
| Executable AND mixed, ложный source без lookup | D/dns_policy.rs:339 `source_short_circuit_preserves_required_dns_for_mixed_rule`, :537 `смешанное_правило_не_обходит_неподтверждённый_dns` | Без resolver I/O |
| Пустые группы, широкое действие | D/dns_policy.rs:640 `пустые_группы_сохраняют_широкое_правило`; E `broad_confirmation_ids_and_limits` | Подтверждение только DTO |
| Первое правило между альтернативами, disabled, user/preset/default/LAN | D/policy.rs:142 `lan_user_preset_and_default_have_explicit_order`; D/dns_policy.rs:136 `d03_any_address_uses_first_rule_independent_of_answer_order` | OR между правилами с first-match |
| Private выше LAN/user; VPN missing; пересечения приватных сетей | D/policy.rs:62, :86; D/dns_policy.rs:89, :165, :359 | Не доказательство host route |
| Private zone/resolver, запрет публичного fallback | D/dns_policy.rs:227 `d06_private_zone_never_requests_public_resolver` | Модель ResolverPath |
| Канонический CIDR, границы/семейства | D/policy.rs:39 `canonical_networks_and_boundaries` | Без разворачивания сети |
| Canonical domain IDNA/case/root dot/suffix boundary | D/dns_policy.rs:58 `idna_case_root_dot_and_suffix_label_boundary` | Общий DomainName |
| NeedResolution vs Failed/partial, не переходить к младшему правилу | D/dns_policy.rs:191, :537 | Ошибка DNS не Direct |
| TTL и network/engine/resolver epoch | D/dns_policy.rs:254, :432, :537 | Не runtime cache proof |
| Fake mapping missing/session stale; fake не real CIDR | D/dns_policy.rs:294, :393, :600 | Реальный worker/provenance открыт |
| Shadowed vs PotentialOverlap, DNS uncertainty | D/policy_diagnostics.rs:20, :39, :80, :94 | Совместимость не доказанный поток |
| Diagnostics IDs/disabled/tiers/budgets/truncation, неизменность Policy | D/policy_diagnostics.rs:54, :116, :151, :190; E `bounded_diagnostics_keep_identity_and_disabled_semantics`, `truncated_report_is_explicit_and_preserves_policy` | Нет ложной полноты |
| Unknown без DNS/application witness, fixture не наблюдение | E `explanation_uses_evaluator_without_claiming_runtime_witness`; editor.rs `Draft::explain` | Unknown — DTO status, не вариант domain Decision |
| ID update сохраняет позицию, порядок и решения при 10x росте | Новый E `обновление_id_сохраняет_порядок_и_решения_при_десятикратном_росте` | 10/100 правил + fallback, 220 точных решений; oracle задан входными адресами/ID/действиями |
| Canonical DTO → Policy сохраняет AND трёх групп и OR значений | Новый E `составной_dto_сохраняет_таблицу_истинности_после_сериализации`: 18 комбинаций source/domain/IP с независимым boolean oracle | Literal с синтетическим именем, не DNS witness |
| Invalid edit сохраняет весь saved DTO; compound DTO roundtrip | E `compound_roundtrip_and_invalid_input_preserve_draft`; новый тест | Реальные put/parse/policy/json |
| Revision CAS, reopen, повреждённый raw сохранён | E `encrypted_draft_roundtrip_and_cas` | Существующий repository fixture без keyring |
| Reorder/partial/stale не удаляет записи; revision/report/ACK | B:3736, :3753, :3795, :3819 `routing_regressions` | Reuse существующего proof, backend не изменён |
| PID/unknown field не теряется тихо | E `pid_is_not_a_persistent_selector`; serde deny_unknown_fields | Отказ DTO, не сохранение неизвестного платформенного правила |
| Сохранение неприменимого платформенного селектора и разрешение перед activation | ОТКРЫТО: Conditions/RuleInput хранят только executable/domain/CIDR | Отказ parse сам по себе не выполняет требование сохранения |
| Simple→advanced→simple на реальных editor functions + persistence | ОТКРЫТО в pure-model proof: editor.rs не содержит mode transformation; QML Main.qml:838/867 переключает presentation bool | Существующий PR94 actual Qt не переносится на текущий QML автоматически |
| Порт/protocol/process name/UID/cgroup capability | ОТКРЫТО: текущая общая модель их не выражает | Unsupported не удаляется ради green |
| Failed Apply сохраняет рабочее поколение, старые потоки не мигрируют | ВНЕ pure-model proof: actual P20/native acceptance | Не подменять saved Draft проверкой Apply |
| A-08 bounded catalog/no Exec/no secret args/hidden scanning | ВНЕ этого ownership; нужны catalog/UI доказательства | Отказ PID не закрывает весь A-08 |

## Reuse и source hashes

Принятые PR [64](https://github.com/MYRSGRAL/yonik/pull/64)
(`eec7a56b6b03f2b022a87682f9097ce78b61fa0b`) и
[94](https://github.com/MYRSGRAL/yonik/pull/94)
(`1297d87b694242e45a0696b02912fe0242923c59`) имеют state MERGED.
`git diff 9ad6069 HEAD -- crates/yonik-domain/src/policy_diagnostics.rs` и
`git diff 8dc426d HEAD -- crates/yonik-desktop/src/editor.rs` пусты.
Evidence reuse: 2026-11-10-policy-and.md, 2026-11-10-policy-diagnostics.md,
2026-11-11-editor-policy-diagnostics.md. Полные прежние proof не повторялись.

Локальный manifest PR94: `/home/myrs/projects/yonik/local/evidence/editor-l15/manifest.json`.
SHA editor.rs совпал: `79ffe3e13c2fd0a862caaefa162b5fd9dd7b77b046e53d98b178d0e9dbd82d14`.
Cargo.lock также совпал. Backend и Main.qml изменились после manifest;
его actual Qt 40/40 нельзя считать проверкой текущих исходников. Точные текущие
hashes всех перечисленных model/test sources сохранены в `p07-model-sources.sha256`
рядом с этим отчётом.

## Остаток для ROOT

Для полного common Linux P07 ещё нужно решить контракт сохранения unsupported
selector и выделения реальной границы двух представлений. Production proposal:
сначала согласовать с ROOT место opaque platform selector и его activation refusal,
а также API представления, позволяющий проверять реальные переходы и persistence
без копии QML-логики в тестах. Этот PR такого API не изобретает.

Actual worker/provenance, application AND domain в TUN и оба native ядра остаются
будущими gates; `system_tun=false`. Синтетический caller witness не verified owner.
P11 backend error contexts и P05 source_changed не затронуты. Нет Go, новых deps,
unsafe, host UI, keyring, маршрутов, nft или установки. Нет production fix, поэтому
production RED/GREEN не заявляется; новый независимый regression test проверяет
ранее не проверенный bounded редакторский сценарий.

## Собственные проверки

Rust 1.85.1, --locked, jobs=1, один Cargo на target, через with-rust-slot
и flock корня `/var/tmp/codex-myrs/transient/p07-model-99307094`.

- `cargo test --locked -p yonik-domain`: 30 integration + 1 doc-test, passed.
- `cargo test --locked -p yonik-desktop --test editor`: 9 passed.
- `cargo clippy --locked -p yonik-domain -p yonik-desktop --all-targets -- -D warnings`: passed, exit 0 (clippy-final.log).
- `cargo fmt --all -- --check`, `git diff --check`: passed.

Первый clippy выявил field_reassign_with_default в новом тесте; исправлен
инициализатор. Это lint failure, не semantic RED. Полные собственные логи вне TTL:
`/home/myrs/projects/yonik/local/evidence/p07-model-99307094`.
GUI/backend/runtime проверки не запускались; прежние результаты явно помечены reuse.
