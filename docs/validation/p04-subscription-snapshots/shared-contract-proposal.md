# Предложение root: external rule sets и кооперативная отмена

Статус: **proposal, не реализовано**, shared contracts требует согласовать root.
Простое повышение `routing::validate_policy` selectors cap не предлагается.
Полный P-04 остаётся открытым до положительных actual gates обоих ядер.

## Минимальный external-rule-set контракт по performance §4

Разделить editable Conditions и внешние immutable наборы: текущий лимит 100000
selectors продолжает защищать inline-policy. Новый `ExternalRuleSetRef`
содержит content digest, format/version, record count, decoded byte bound,
тип (точные/суффиксные домены либо CIDR), revision; никаких произвольных путей
от UI. `RuleSetStore::open_verified(ref)` выдаёт ограниченный reader/mmap lease
неизменяемого snapshot. Запись — streaming staging → validation → atomic publish,
один current и один rollback; lease активной сессии запрещает преждевременное
удаление. Старый набор доступен до commit нового.

Предлагаемые точки изменения после согласования:

- `yonik-domain`: отдельные refs в политике, индекс для evaluation oracle:
  reverse-label trie для доменов, radix/prefix index для IPv4/IPv6;
  строгая нормализация и тип exact/suffix, никаких объектов на каждый IP.
  O(input bytes + N log N) для построения; сопоставление по длине имени/адреса.
- `yonik-engine::compile_policy[_with_dns]` / `compile_system_policy`: новый
  `RuleSetCatalog`-аргумент через отдельный API, возвращающий `CompiledBundle`
  (не только JSON). Bundle = config + immutable content-addressed artifacts.
  Проверка и компиляция сохраняют порядок правил, OR внутри категории и AND
  между категориями/приложением. Не раскрывать app × domain × CIDR.
  IP-only продолжает требовать доказанного DNS provenance для доменов;
  external sets сами по себе не снимают этот запрет.
- Xray: native geosite/geoip resource `.dat` и `ext:file:tag`; сериализатор
  ограниченного protobuf subset exact/suffix + canonical CIDR на Rust,
  зафиксированный format/version и golden fixtures. Engine resource directory
  принадлежит worker; filename выводится из digest, tag — из validated kind.
  Это предложение реализации: совместимость требуется проверить на закреплённом
  Xray, включая malformed DAT, отсутствие tag и конечные границы CIDR.
- sing-box: local source rule-set JSON на первом шаге, затем indexed `.srs`
  только через уже закреплённый engine tool с bounded subprocess или отдельно
  согласованный Rust writer. Source JSON — нативный supported формат и позволяет
  положительный gate без новой зависимости/Go, но память его загрузки включается
  в engine gate. Не remote rule-set: загрузка/автообновление исключены.

Первичные источники форматов (прочитаны 2026-11-11):
[Xray routing](https://xtls.github.io/en/config/routing),
[sing-box local rule-set](https://sing-box.sagernet.org/configuration/rule-set/),
[sing-box logical AND](https://sing-box.sagernet.org/configuration/route/rule/).
Текущая документация не заменяет проверку закреплённых в проекте версий.

## Limits, память и helper 64 KiB

Начальный предлагаемый budget: до 16 refs/session; 1500000 records суммарно
(100k domains + 50k CIDR, затем 10x); canonical domain ≤253 bytes, никаких
regex на первом шаге. До 512 MiB encoded/decoded на диске суммарно, до 128 MiB
на artifact, 64 MiB transient builder RSS добавки через streaming/external sort
с bounded merge fan-in. Это **параметры для измерения**, не достигнутый memory gate.
Контроль 192 MiB управляющего слоя сохраняется; full 10x может потребовать
дисковых индексов и ограниченного mmap resident set. После построения не держать
одновременно исходный текст, AST и native artifact целиком.

D-Bus helper `service.rs::MAX_WIRE_BYTES` / `mod.rs::MAX_REQUEST` остаются 64 KiB:
по ним идёт только refs manifest (16 × bounded metadata), а не сотни тысяч
rules. Нельзя вставлять rule bodies в JSON RPC или передавать caller path.
Worker/helper принимает sealed read-only FD либо доверенный content-addressed
artifact lease; проверяет type/size/digest/owner до Prepare, сохраняет generation
и reference leases до Stop. Привязать bundle digest к config и всем artifacts,
чтобы замена файла не меняла активную политику.

Существующие `prepared.rs` 4 MiB artifact, `memory.rs` 8 MiB retained/config и
64 MiB process config pool не покрывают большие rule sets. Не обходить их
скрытой памятью: отдельный `RuleSetByteBudget` для disk/mapped leases с current,
rollback и active reservations; общий RSS/PSS gate включает фактические mappings.
Новый ownership затронет application prepared-plan, helper prepared/session_host,
worker sandbox FD/resource directory и config-check. Без этого согласования
большой bundle не может считаться готовым к передаче helper.

Положительный acceptance после реализации: 100k/50k и 10x distinct records,
20 snapshots, оба native engine config-check; oracle membership/boundaries,
AND с 10/100/1000 apps, unchanged priority/default/DNS; конфиг O(refs+editable),
artifacts O(records); время CPU/RSS/build/disk отдельно от engine loading;
cancel во время parse/build/merge/config-check сохраняет old snapshot и удаляет
staging; превышение budget — дополнительный negative test, не замена gate.

## Минимальная кооперативная отмена production import/commit

Actual probe: `subscription_snapshot_resource` берёт BEGIN IMMEDIATE на своей
БД, запускает `EncryptedSubscriptionRepository::commit_snapshot`, через 100 ms
aborts JoinHandle и снимает lock. Затем `load` ждёт repository gate и проверяет
revision. Если revision выросла, отменённый future оставил выполняющийся commit.
Тест сигнализирует отсутствие cancellation guarantee; принудительный kill и
abort future не называются кооперативной отменой. Если задача не попала в worker,
probe завершится ошибкой, а не ложным подтверждением.

Callsites для минимального изменения после расширения ownership:

1. `yonik-import::import_bytes` → новый `import_bytes_cancellable(bytes, check)`
   с checkpoint между URI/JSON records и до больших decode/normalize этапов.
   Сохранить старый entrypoint как uncancelled compatibility API. Ввести отдельный
   Cancelled outcome, не смешивать его с повреждённым вводом/частичным approval.
2. `yonik-subscription::Subscription::stage`: cancellable variant передаёт token
   в parser; cancel не публикует report. `subscriptions.rs::offline` и
   `refresh_with_host` (blocking `controller.stage` + ожидание operation после
   watch cancellation) должны передавать тот же token и дождаться worker ack.
3. `repository.rs::run` и `commit_snapshot`: cancellable variant получает уже
   существующий `Cancellation`, проверяет перед gate, внутри blocking closure,
   в `prepare_profiles` и `reconcile` между bounded batches, перед SQLite COMMIT.
   SQLite busy wait заменяется ограниченным retry с checkpoint, иначе lock
   удерживает worker после cancel. RAII transaction rollback сохраняет snapshot.
4. `subscriptions.rs::commit` передаёт request cancellation в repository.
   Синхронизировать cancel/commit point: до atomic commit latch отмена гарантирует
   rollback; после latch вернуть `Committed(revision)` вместо ложного Cancelled.
   Не держать requests mutex через долгий await так, чтобы cancel не мог
   установить token. Это требует изменения ownership/contract, не wrapper.

Нужный failing acceptance после изменения: отменить пока запись заблокирована
write-lock, worker acknowledged within заданного deadline, после release revision
и body прежние, свободны IMPORT_GATE/repository gate; отдельно cancel при разборе
10000 записей и на границе COMMIT. В текущем scope фиксируется actual продолжение
commit после abort; cooperative PASS не заявляется.
