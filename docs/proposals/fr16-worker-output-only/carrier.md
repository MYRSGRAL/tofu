# Staged Rust carrier после proposal 3cafa20

Root отдельно разрешил применение и компиляцию carrier только под existing
`test-support`/`cfg(test)`. Исторические `proposal.md`, `closed-boundary.patch`
и `manifest.json` относятся к исходному proposal, а не к текущим Rust-файлам.
Их NOT COMPILED остаётся утверждением о том текстовом patch. Текущие проверки
carrier фиксируются отдельно в `carrier-evidence`.

## Применённая граница

- `output_transport` — private crate common module, исключён из default library.
  В него перенесены pipe types/factory/validator #204, без второй реализации
  protocol model. Worker test harness импортирует их; его V3 validation,
  V1/V2/YNKROLE1/ABI260 golden остаются frozen. Старые source-mutant fixtures
  представляют исходник #204, не новое source-mutant proof carrier; старый
  generator привязан к расположению исходника #204, здесь не запускался.
- Common JSON parser не зависит от worker/helper. Owned DATA builder создаёт
  `ApplyOutputV1` `(s, ah)`, decoder проверяет тип/адрес/interface/member,
  bounded body/JSON, exact sah, header UNIX_FDS=2, actual FD count=2,
  array len=2, FD aliases и PIPEFS identities. JSON только version/handle.
- `system_client::output::PreparedOutput` содержит **существующий Prepared**
  by value, две уникальные pipe пары, captured Engine и новый immutable scoped
  EventLog/generation. Нет Clone/Debug/Serialize, setters, split, rebind или
  общего FD registry. Это data binding: соответствие переданного Engine
  успешному Prepare должен обеспечить будущий caller; kernel/identity authority
  factory не создаёт. Тест использует настоящий Rust Prepared с синтетическими
  handle/owner/peer; helper-issued grant этим не имитируется.
- Consume `encode` оставляет readers, Prepared, Engine и log в другом opaque
  owner вместе с DATA Message. Sender writer copies закрываются после build,
  не хранятся в payload. Публичного send/retry или извлечения readers нет.
  Это **не доказательство доступности consumer** и не готовый apply API.
- `helper/service::decode_staged_output` — закрытая typed DATA boundary,
  преобразующая handle в существующий PreparedHandle; **не claim/auth**.
  Нет нового Request, dispatch или introspection registration. Готовый decode
  не даёт полномочий обходить existing claim_for_apply/unique owner/auth.

Message владеет zbus copies FD, decode по значению закрывает их на всех выходах;
успех оставляет только две validated owned copies. Внешние Message::clone
принадлежат их владельцу; decoder не обещает закрыть чужие Arc references.
Guard flags не исправляются: недопустимая пара отклоняется. Для настоящего
recv SCM_RIGHTS CLOEXEC остаётся обязанностью транспортного receive path.

## Остальные API и разрешения

Сохраняется карта `proposal.md`: новый ServiceHost apply_output и Operation,
Protection/SessionHost forwarding/cleanup, invocation-bound SystemdWorker
one-shot transfer, worker V3 receive/configured stdio и app consumer #196
по-прежнему **не реализованы и не зарегистрированы**. Frozen #196 не копируется,
EventLog/Backend/Qt не меняются. ProductionPermit/N5 mint, existing Apply,
SessionHost::start и worker exec не меняются.

После неизвестного send outcome будущий caller обязан consumed handle,
Unknown state, NO retry/downgrade. EOF не означает stop/cgroup-empty. Полный wire,
registration и actual legacy/N5 run требуют нового root decision. Review/merge
#204 остаётся отдельным gate; merge его frozen head здесь — dependency, не
acceptance. Runtime16, installed runtime, user grants, host services/probes
не выполнялись и этим carrier не разрешаются. Это не полная FR16.
