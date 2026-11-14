# FR16: предложение подключения фиксированных output pipes

**NOT COMPILED / NOT EXECUTED / NOT APPLIED / NOT REGISTERED.** Это proposal,
не полная FR16 и не разрешение root применять patch. Runtime16, установленный
runtime и отдельные user grants этим документом не подтверждаются.

База: `3247e6c32a5ed83ec5d812dfc1f70695d3f2b3b4` (#202).
Frozen private codec #204: `e1970210190b7e89ab9ffd06a0a33203dec82d85`, прочитан
через Git objects, остаётся на review. В записи запроса `204e197…` префикс
204 интерпретирован как номер PR; это подтверждено head SHA PR #204.
Root contract `5d050` использован ровно в изложении задания: такой Git ref
в данном хранилище не разрешается, иных утверждений о его содержимом нет.
Frozen LocalSocks #196: `1c1786ce4d3df6654d21c235b6f65dd7ac2b6132`;
прочитаны `host/process.rs::RuntimeReaders/reader` и
`event_log/native_details.rs::Input`. Ни один из этих файлов не перенесён.

## Артефакт и граница готовности

`closed-boundary.patch` — unified diff нового, **незарегистрированного**
`crates/yonik-linux/src/worker/output_v1.rs` относительно базы. Его SHA-256
в `manifest.json`. На диске существует только текст patch внутри docs.
Он содержит фиксированные JSON/header/FD проверки, не sender, apply или exec.
Это минимальный статический кусок будущей интеграции; он не является готовым
сквозным production diff. Ни `mod`, ни D-Bus introspection не добавлены.
Имена/типы сверены чтением исходников; разрешение и компиляция Rust не проверялись.
Root должен отдельно решить размещение JSON parser: логичнее перенести его
в service при последующем применении, чтобы worker не зависел от helper API.
4096 — предлагаемый локальный JSON bound для двух полей, не новый wire limit.

## Фактический call graph базы и предлагаемые закрытые API

| Узел базы | Предлагаемое изменение, только после root review |
| --- | --- |
| `yonik-desktop/src/backend.rs::Flight::start`, PolicyConnect, prepare_policy → apply | Передавать captured engine и scoped log в SystemSession. Имя appSystemSession в задании соответствует этому backend/Flight и SystemSession, отдельного символа с таким именем в базе нет. |
| `yonik-desktop/src/system_session.rs::SystemSession`, `prepared: Option<Prepared>` | Заменить только output path на `Option<PreparedOutput>`; bound объект создаётся после Prepare, до Apply и до любой mutation. `revise`, stop и unlock уничтожают только неотправленный объект. |
| `yonik-linux/src/system_client.rs::SystemClient::apply` → state_call("Apply") | Новый конкретный `apply_output(prepared: PreparedOutput, deadline: Instant, cancel: Cancellation) -> Result<OutputApply, ClientError>`. Consume, проверка owner/peer как в apply, точный метод ApplyOutputV1, signature sah, JSON version=1/handle, ровно два FD. Legacy apply не меняется. |
| `helper/service.rs::parse/dispatch Request::Apply` | Отдельный `Request::ApplyOutput(PreparedHandle, RunOutputWriters)`; до claim bounded JSON, signature sah, declared UNIX_FDS=2, array len=2, уникальные FD индексы, два validated writer. Затем тот же `claim_for_apply` с `Prompt::Allow`, peer UID, unique sender, context closure под host lock. Не выдавать новый handle. |
| `ServiceHost::apply` → `ProtectionHost::apply` → job(Operation::ApplyProfile) | Новый закрытый `apply_output(uid, &Artifact, RunOutputWriters, Cancellation) -> Result<ProtectionState, ServiceError>` и одноимённый Operation с owned writers. Только SessionArtifact; GuardPlan без session отклоняется. Не класть FD в сериализуемый artifact. |
| `SessionHost::apply_profile` → `start_profile` → `supervisor.start(payload)` | Параллельный закрытый output path принимает writers by value; все topology/stamp/owner checks и journal/worker-first cleanup сохраняются. Не использовать публичный generic FD registry. |
| `SystemWorkerSupervisor::start_inner/send_inner`, `InvocationBinding::transfer/transfer_payload` | Только `start_authorized_output(payload, authorization, writers, cancel) -> Result<AuthorizedRun>`; writers в одноразовой transfer custody, не Arc WorkerPayload. Сохранить N5 permit до передачи. Exact invocation PID/UID/cgroup/digest/authorization checks как у binding, включая повторную валидацию после ACK. |
| `WorkerPayload::send` → `worker::receive_payload` → `serve_stream` → `configured_assets` → handoff.exec | Новый закрытый send_run_output, новый explicit V3 decode arm. ReceivedRunOutput хранит validated пару. После configured_assets перед exact Run exec пару consume в stdout/stderr. Не менять exec в данном patch. Check не принимает V3. |

Типы будущего output path (ни один не Clone/Serialize/Debug):

```rust,ignore
// system_client: поля приватны; factory единственная, split публично отсутствует.
struct PreparedOutput {
    prepared: Prepared,                 // настоящий existing handle + owner + peer
    readers: OutputReaders,             // stdout/stderr, две разные pipe()
    writers: RunOutputWriters,           // fixed stdout/stderr, не Vec registry
    log: EventLog,                      // immutable session_scope().with_generation(...)
    engine: Engine,                     // captured из успешного Prepare
}
// Закрытая фабрика получает existing Prepared by value, captured generation/engine/log;
// возвращает объект только после создания обеих пар и привязки log.
// Generation является локальным контекстом, не JSON/wire authority.
struct OutputApply { state: State, readers: OutputReadersGuard }
// SystemSession хранит guard до EOF/stop cleanup; guard не выдаёт writer/handle.
```

`apply_output` **до отправки** переводит обе читающие половины в owned guard
с captured engine и immutable log. Guard уже читает к моменту mutation: иначе
worker может заблокироваться на полном pipe. Транзакция держит guard при await;
успех возвращает его SystemSession, ошибка уничтожает,
drop отменяет reader tasks. Публичного извлечения writers/readers из
PreparedOutput нет: внутреннее consume выполняет ровно одна state machine.
Фабрика при ошибке второй пары/лог-context закрывает уже созданные FD через RAII.
Revision fence проверяется до consume; никакой callback не читает actor.current
или current generation. В лог попадает тот же immutable scoped handle даже
после начала следующего поколения.

## Wire, parser и FD custody

ApplyOutputV1: existing handle JSON `{"version":1,"handle":[…32 bytes…]}`,
D-Bus `(s, ah)`, ровно два уникальных индекса FD и ровно два UNIX_FDS.
Лишние JSON поля (включая session, generation, stdout, stderr), дубли полей,
неверная версия, длина handle, signature/count отклоняются. Повтор FD индекса
отклоняется до дублирования. Два разных индекса одного inode также отклоняются.
Нельзя добавлять raw bytes, roles или output metadata в ACK.

Обе половины pipe создаёт app. Writers: anonymous PIPEFS, FIFO, O_WRONLY,
blocking, FD_CLOEXEC, разные `(st_dev, st_ino)`. FIFO filesystem сам по себе
недостаточен: именованный FIFO на обычной FS отвергается через fstatfs.
Readers неблокирующие, CLOEXEC. Validator не исправляет флаги.
Не доверять имени `/proc` или строке `pipe:[…]`.

WorkerRun V3: magic `YNKWRK\0\x03`, header `112 + 54*R`, `R=0..16`,
FD count `4+R`: binary, config, R resources, stdout, stderr.
Bytes 96..100 = `[0,2,1,2]`, 100..112 нулевые; legacy reserved byte 11 = 0;
mode строго Run. При R=0 scope 80..96 весь нулевой; R>0 — прежняя resource
scope/manifest validation V2, без output authority. Base Header validation
сохраняет engine/length/hashes. Проекция V3 в legacy decoder в patch служит
только внутренней проверке; это не wire downgrade и не повторная отправка.
V1/V2 decoder и YNKROLE1/ABI260 неизменны, fallback после V3 ошибки запрещён.

| Стадия | Кто держит FD и что закрывает |
| --- | --- |
| До отправки | PreparedOutput держит пару writers/readers; отказ/отмена закрывает всё. |
| App → helper | App держит sender copies только на один call, закрывает после send outcome, включая timeout/drop. Message owns received copies; borrowed FD проверяются, затем ровно одна пара owned handles переходит в request. Message drop закрывает originals. |
| Parse/claim/auth отказ | Request/message RAII закрывает все принятые FD; будущий receive обязан fail-close также лишние SCM_RIGHTS, truncation, malformed control. Нельзя сохранять только валидную часть. |
| Helper job → invocation | Единственный move writers через Operation и SessionHost. Отказ до отправки закрывает пару; artifact/payload не удерживают дублей. Helper sender copies закрываются на любой результат send. |
| Worker receive → configured Run | FD все owned с момента recvmsg/CLOEXEC при получении; reject закрывает binary/config/resources/output. Успех переносит пару только в stdio exact invocation. Worker не удерживает лишних write copies. |
| Reader cancel/error | Abort/await bounded tasks, стирание scratch по frozen #196; reader закрывается. Это не команда stop worker. Writer может получить EPIPE — cleanup остаётся worker supervisor. |

Cancellation **после неизвестного результата отправки**: prepared consumed,
state Unknown, никакого retry ApplyOutputV1/legacy Apply и никакого resend writers.
Helper после journal mutation не освобождает worker custody по caller cancellation;
существующий worker-first cleanup и recovery fence обязательны. Только
подтверждённый stop/cgroup-empty снимает uncertainty. EOF лишь завершает чтение
этого потока: он не доказывает stop, exec success или пустой cgroup. ACK — прежний
status/PID, не доказательство отсутствия потомков.

## Consumer и отсутствующие API

#196 ещё не входит в данную базу. Будущее разрешённое подключение должно
переиспользовать его `Input::new(engine, immutable_log)`/`feed` и bounded reader,
не копировать classifier/version map. Нужен закрытый `attach_output_readers`
в host, который принимает два AsyncRead и запускает тот же reader; отдельно
парсит stdout/stderr (фрагменты не смешиваются). Input использует pinned grammar,
4096-byte line/16KiB turn, yield, reject/withhold и очистку scratch из frozen ref.
EOF не публикует незаконченный frame. Unknown version не позволяет raw-log fallback.
`RuntimeReaders::attach(Child, …)` из #196 не подходит anonymous pipe напрямую;
его adapter и доступность guard — реальные отсутствующие API, не существующие
в базе функции. Перед реализацией root согласует их с владельцем #196.

Остальные отсутствующие API перечислены в таблице: PreparedOutput factory,
D-Bus FD-call с one-shot ownership, ServiceHost/Operation output variants,
SessionHost output start, invocation-specific send/receive, stdio consume.
Заполнять эти пробелы обходом N5 или переключением SessionHost на непроверенный
start_authorized нельзя. Существующий `ProductionPermit::mint` остаётся без
изменений. Kernel grant не авторизует будущие production APIs, runtime install
или host tests. Снятие permit gate — отдельное решение и отдельное доказательство.

Компромисс: один новый fixed method вместо расширения Apply/registry оставляет
legacy поведение и минимизирует authority. Цена — новый сквозной owned path и
обязательная композиция с #196. Минимальный patch сознательно не прячет эту
цену за заглушками, generic executor или работающей регистрацией.
