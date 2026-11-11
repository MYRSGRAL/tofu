# Сохранённая Policy → SystemSession

База: `5275f5d599dfebcf89685e8008223ad2bfeaf99b` (combined PR90 и actual
Maintenance Actor Option PR107). Android отложен. Изменения FD bundle,
helper/worker ABI и maintenance jobs в этот срез не входят.

GUI `routing_apply` передаёт expected routing revision, request ID, source
generation, snapshot revision и selected profile ID. Policy и ServerIntent
берутся из текущего состояния actor. Перед отправкой проверяются сохранённая
routing revision, поколение источника из snapshot и repository, выбранный
профиль и snapshot. Native config, claims и пути ресурсов в команду не входят.

Типизированный InlinePolicy использует существующий
`prepare_profile_policy` → `Prepare(system_profile_policy)`; общий транспорт
SystemClient сохраняет owner/sender, deadline, cancellation и error fences.
Семантика user/preset/private/LAN и порядок правил сохраняются. Exact private
DNS не преобразуется в suffix: wire его не выражает, поэтому возвращается
Unsupported. Ограничения wire 64 KiB/128 правил не расширены; превышение
отклоняется целиком.

Flight привязан к routing/profile/source revisions. Изменение Draft или выбора
отменяет flight перед первым await редактирования; смена сети также отменяет
подготовку. Подготовленный handle извлекается до await Apply и не используется
повторно. Matching ACK формируется при completion, queued Stop, startup,
maintenance busy и отказе dispatch. Recovery guard остаётся sticky; состояния
TunnelStarting/VerifiedRules не объявляются Connected.

Кнопка Apply требует свежей capability реального helper, выбранного профиля,
готового idle actor и отсутствия recovery. Capability получается тем же
типизированным клиентом, без изменения registry и без Local fallback.

## Проверки

Rust 1.85.1, `--locked`, jobs=1, общий `with-rust-slot`, shared flock корня
собственного transient-каталога:

- Linux private-bus SystemClient/SystemSession: 8 PASS, включая inline
  Policy/Prepare/Apply, one-use handle и инвалидирование ревизией.
- Desktop lib: 43 PASS; DesktopHost: 41 PASS и 1 прежний IGNORE.
- Dispatch ACK: 5 PASS, включая routing_apply при busy/startup.
- Clippy85 для обоих пакетов: `--all-targets -- -D warnings`.
- Qt GUI собран MSRV85. Настоящий Qt/private-bus COMPONENT сценарий `policy`
  сохраняет Block в Draft и нажимает routingApplyButton; fixture проверяет
  текущую Policy и наблюдает Prepare/Apply. Также выполнены normal,
  unsupported и cancel. Усиленный policy-edit требует фактический Prepare до
  изменения Draft и отсутствие Apply после него; applycancel подтверждает Stop
  при уже отправленном Apply.

Локальные логи и provenance находятся в
`/home/myrs/projects/yonik/local/evidence/p20-inline-5787/`, вне TTL сборки.
Private-bus fixture не запускает production helper/TUN: этот результат не
является положительной приёмкой FR-20, G-DNS или kernel routing. Реальный
CIDR-only route gate в отмеченной VM остаётся отдельной проверкой production
transport/helper; capabilities для такой проверки не переписывались.

Конкретный blocker production positive: `helper/service.rs`, обработчик
Capabilities, в этой базе публикует `system_tun:false`. Поэтому production
кнопка остаётся недоступной. Этот срез не меняет helper capability ради теста;
положительная проверка использует только явно маркированную COMPONENT fixture.
Дальнейший route gate требует production capability/transport integration
в зоне helper/FD writer и отдельного запуска в отмеченной VM.
