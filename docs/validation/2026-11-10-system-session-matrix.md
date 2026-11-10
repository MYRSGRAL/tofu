# SystemSessionHost: actual VM matrix

Проверенный head: `c758bd243faba7909ea8f06d626dcf97508e8166`.
PR49 влит как `2be9a1bc8fcd553594e36e8a4c91614c15ba6251`.

| Ядро | kill switch=false: stop/restart/worker_exit | kill switch=true: stop/restart/worker_exit |
|---|---|---|
| Xray 26.10.31 | PASS / PASS / PASS | PASS / PASS / PASS |
| sing-box adapter 1.14.4 | PASS / PASS / PASS | PASS / PASS / PASS |

Все 12 cases проверили actual TCP/UDP IPv4/IPv6: 48 protocol/family probes.
Дополнительный protected sing-box diagnostic также PASS. Apply/Stop/traffic/cases
не повторялись для получения успеха. State допускал только typed Busy по
ограниченному контракту приёмки: всего Busy=10, calls=22, максимум 169 ms.

В 13 cleanup episodes sampler наблюдал пустые cgroups раньше уменьшения journal.
Это наблюдаемая последовательность снимков, не универсальное доказательство
порядка всех kernel syscalls. Финально MainPID=0, cgroups отсутствуют, records=0,
public_policy=null, session_recovery=null; своих TUN/routes/RPDB нет.
Последний intentional SIGKILL оставил systemd unit failed без живого процесса.
Поэтому сырой inactive-only checker=false сохранён; отдельное resource-cleanup
подтверждение не подменяет его значение.

## Границы

Настоящие pinned workers запущены в одноразовой Arch VM; host OS не менялась.
SystemSessionHost/backend создавались внутри guarded lib-test/private D-Bus и
polkit fixture. Lifecycle unit helper был sleep fixture; production helper
executable/system bus policy/реальный crash helper не проверены. Restart означает
пересоздание runtime fixture. `system_tun:false`, общий P09/P12/P13 остаётся partial.
DNS/Fake-IP, LAN/private VPN, attribution и GUI этой матрицей не принимаются.

Предыдущий external Evidence failure не воспроизведён; его причина не установлена.
Исторические Unavailable и SSH hang также не объяснены этим успешным run.

## Evidence

Полный отчёт, cases, serial/journal, sampler и provenance вне TTL:
`/home/myrs/projects/yonik/local/validation/system-session-vm-gate/external-c758bd2/`.
Exact lib-test SHA-256:
`5e603603004fa67fd5c8056aba9b6bf9a2e16849be394ca38487afbe7fc40064`.
VM штатно выключена, собственный transient удалён; rebuild в этом прогоне не было.

## Сверка сохранённого evidence 11 ноября 2026

Перед публикацией сводки повторно сверены размер и SHA-256 сохранённого
lib-test с `binary-provenance.json`: 168 514 064 байта, значение выше совпало.
Это проверка целостности сохранённого артефакта, не новый запуск VM.

В `summary.json`: cases_seen=cases_passed=traffic_groups=12,
matrix_order_observed=matrix_confirmed_full_records_witness=12.
Хеши файлов исходного evidence (байты не изменялись):

- `report.md`: `314edbf7bdeb88f1cd6aebe47cb40c4fca5abe6d7522ea5e163bccabffd8cd1f`.
- `summary.json`: `57d64b0a0ab315c12dc6d6dc0b72bdcb9928fafba27c6fbe0e502363e688c90f`.
- `binary-provenance.json`: `7557db0282c6d0d76f15327aa518a3c870ece2afa76762cddbd5067c47c327d8`.
- `cases.json`: `ba1ffde6d48fee872c1b75705509f6da665e1deb13e78d7220d97179f695424f`.
- `cleanup-order.json`: `953d9d25d8a91da2e7cc86c33ef36094bda52c15e08aea1ec2ebed3eaad6f8cb`.

Проверенные core artifacts этой матрицы:

- Xray: `cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1`.
- sing-box **adapter**: `4c31db07cb394479b58cc9aace8c8bef5da05a73db6dacca4a019aeb8b14e16d`.

Эта матрица не доказывает совместимость другого sing-box CLI artifact с полем
`external_configuration`; exact binary/config/adapter сравниваются отдельно.
