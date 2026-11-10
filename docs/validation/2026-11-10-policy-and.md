# P07: И между группами домена и CIDR — 2026-11-10

Base фактического worktree: e33fa1908dfb3deb5434506341674de0b3a827b4 (проверен до изменений).
Head реализации: a44c114ad10269efb152e2397ad71dcf29dc7816. Следующий документационный коммит добавляет этот отчёт.
Ветка: t3/policy-domain-cidr-and.

## Контракт и причина

docs/specs/routing-and-tray.md:52: «Условия домена и CIDR одновременно означают И».
docs/specs/data-and-contracts.md:30: «Группы условий соединяются через И, значения внутри группы — через ИЛИ».
Равноавторитетного противоположного требования в этих specs не найдено.
В истории evaluator, тест destination_or и README перенесены коммитом
9cbe2de41931e6b770af40e2675c4c2d347a60c4. Они закрепляли OR между доменной и CIDR группами:
совпадение домена обходило DNS и проверку сети. Ошибка реализации и её старое ожидание
приведены к конкретному контракту редактора.

## Изменения

- crates/yonik-domain/src/evaluation.rs: после executable сначала проверяется доменная
  группа, затем CIDR через прежний Addresses::get; OR внутри списков сохранён.
- crates/yonik-domain/tests/dns_policy.rs: исправлено ожидание старого OR-теста;
  добавлены bounded матрицы user/preset, exact/suffix, IPv4/IPv6, несовпадения
  без DNS, literal без имени, Missing/Failed/TTL/трёх epoch/partial, executable,
  Fake IP обеих семей и пустых групп.
- README.md и README.ru.md: исправлены только абзацы семантики условий.
- Этот новый отчёт. Зависимости и прочие компоненты не изменены.

## Red → green

До production-правки cargo +1.85.1 test --locked -p yonik-domain --test dns_policy:
12 passed, 3 failed. Ошибки:
- source_short_circuit_preserves_required_dns_for_mixed_rule: Direct вместо NeedResolution.
- смешанное_правило_не_обходит_неподтверждённый_dns: Direct вместо NeedResolution.
- смешанные_группы_соединяются_через_и: Block вместо Proxy при чужом CIDR.
После правки: 17 passed, 0 failed (в том числе D-01…D-07).

## Проверки

Все Cargo-команды: CARGO_BUILD_JOBS=2,
CARGO_TARGET_DIR=/var/tmp/codex-myrs/transient/yonik-policy-and/target,
flock -s /var/tmp/codex-myrs/transient/yonik-policy-and.
Toolchain: 1.85.1, зависимости --locked.

- cargo +1.85.1 test --locked --workspace — exit 0:
  529 passed, 0 failed, 24 ignored.
- cargo +1.85.1 test --locked -p yonik-domain --test dns_policy — exit 0, 17 passed.
- cargo +1.85.1 clippy --locked --workspace --all-targets -- -D warnings — exit 0.
- cargo +1.85.1 fmt --all -- --check — exit 0.
- git diff --check — exit 0.

Ignored остаются неизвестными; этот результат не является их приёмкой:
- test route_changes_invalidate_completed_and_inflight_tcp_checks ... ignored, только отдельный unshare network namespace с YONIK_HOST_NETWORK_NS
- test credentials::tests::secret_service_readonly ... ignored, Требует доступного Secret Service; только чтение, ключи не создаются
- test fetch::tests::public_https_direct ... ignored, Требует публичной сети; проверяет реальный HTTPS/DNS/peer Direct
- test helper::protection_host::tests::isolated_service_applies_preserves_and_unlocks_real_rules ... ignored, Требует отдельные user/net/mount/pid namespaces и приватные /run, /tmp
- test managed_network::nft::block::tests::real_public_block_tcp_udp_dual_stack_and_lifetime ... ignored, Требует isolated user/net/mount/pid namespace с приватными /tmp, /run, proc и sysfs
- test managed_network::nft::policy::tests::crash_worker ... ignored, Только дочерний процесс из изолированного crash-теста
- test managed_network::nft::policy::tests::sigkill_recovery_preserves_desired_previous_and_unlock_intents ... ignored, Требует явно изолированный user/net/mount/pid namespace с приватными /tmp и /run
- test managed_network::nft::tests::real_namespace_journal_create_reopen_foreign_and_stale_handle ... ignored, Требует явно изолированный network namespace и YONIK_NFT_TEST_HOST_NETNS
- test managed_network::tun::config::tests::real_namespace_owned_addresses_routes_and_cleanup ... ignored, Требует отдельные user/net/mount/pid namespaces и приватные /run, /tmp
- test managed_network::tun::rule::tests::real_namespace_owned_rpdb_and_cleanup ... ignored, Требует отдельные user/net/mount/pid namespaces и приватные /run, /tmp
- test managed_network::tun::session_tests::compiled_profile_crosses_owned_tun_and_proxy ... ignored, Требует отдельные namespaces и явно выбранный закреплённый engine
- test managed_network::tun::session_tests::compiled_worker_crosses_owned_tun_and_proxy ... ignored, Требует отдельные namespaces, engine и собранный yonik-worker
- test managed_network::tun::session_tests::unpatched_engine_is_rejected_before_network_effects ... ignored, Требует отдельные namespaces и явно выбранный unpatched sing-box
- test managed_network::tun::tests::real_namespace_tun_ownership_reopen_and_cleanup ... ignored, Требует отдельные user/net/mount/pid namespaces и приватные /run, /tmp
- test managed_network::tun::topology::tests::real_namespace_projection_matches_external_baseline ... ignored, Требует отдельный user/net namespace и YONIK_NFT_TEST_HOST_NETNS
- test managed_network::tun::topology::tests::real_namespace_session_plan_configure_capture_verify_cleanup ... ignored, Требует отдельный user/net namespace и YONIK_NFT_TEST_HOST_NETNS
- test network::read::tests::native_queued_changes_invalidate_reader_and_final_fence ... ignored, Только новый network namespace с YONIK_HOST_NETWORK_NS
- test network::route::tests::native_read_only_subscription_owns_nonblocking_close_on_exec_socket ... ignored, Требуется AF_NETLINK/NETLINK_ROUTE в окружении исполнения
- test tcp_batch::tests::ten_thousand_loopback_series ... ignored, Явная нагрузочная проверка: 10000 серий, 30000 loopback-соединений
- test tcp_probe::tests::production_resolver_localhost_connects_and_closes_socket ... ignored, требует работающий системный systemd-resolved Varlink; только localhost
- test worker::tests::canonical_worker_process_check_and_run ... ignored, Нужны canonical ELF, собранный yonik-worker и собственный user namespace
- test installed_real_cores_cross_owned_tun_and_proxy ... ignored, Только disposable root QEMU VM с установленными units и двумя staged ELF
- test installed_fixture_tree_and_helper_binds_to ... ignored, Только одноразовая root QEMU VM: fixture ELF, реальный KillMode и BindsTo
- test installed_real_cores_check_start_stop ... ignored, Только одноразовая root QEMU VM: установленный systemd worker и два canonical ELF

## Ограничения и остаток P07

Native compiler проверен read-only: crates/yonik-engine/src/routing.rs:365–381
отклоняет CIDR в Names (RequiredRealResolution), домены в IpOnly
(DomainsInIpOnly), включая mixed правило. Gate не изменён; существующий
engine routing test unproven_native_combinations_are_errors_not_semantic_changes
прошёл в workspace. Нативное смешение и его live fail-closed доказательство остаются
незакрытыми. GUI редакторы и полная матрица P07 вне этого среза.
Qt feature/smoke не запускались: GUI не менялся. Пользовательские данные,
сеть, keyring и системные helpers не изменялись.
Независимый review и merge выполняет root.

Сырые логи red/green/clippy были прочитаны; ниже их SHA-256 для сверки локальной
диагностики (самодостаточные результаты и ограничения сохранены выше):

- /tmp/yonik-policy-and-red.log: 5ee9003c31820a22f7a9bc95a5e8dc0ca81ac1ade3031c30b79d0d77f38e919b
- /tmp/yonik-policy-and-green.log: ecbcb66b87999a07933263441203b8a6f0e1f50e7b6d93bbb37ff925c6de8b42
- /tmp/yonik-policy-and-clippy.log: db73d98040d3811b34546100de84d21eb20727de588c9d230c11a568f890499a
