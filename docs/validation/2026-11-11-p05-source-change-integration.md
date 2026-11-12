# P05: additive source_change и интеграция PR113

Команда закрытая (`deny_unknown_fields`):

```json
{"command":"source_change","request_id":42,"id":7,"expected_generation":2,"name":"Источник","url":"<новый секретный URL>","local_exception":false,"route":"proxy","hwid_enabled":false}
```

ACK: `source_change_ack` с request_id/id/generation/error; URL и credentials
отсутствуют. До commit при stale/error generation = null; если commit уже подтверждён, generation сохраняется и при последующей ошибке reload. Используется существующий
map_subscription/error catalog; новый message-key не вводится. request_id
положительный <= JS MAX_SAFE_INTEGER; id/expected_generation положительные;
лимиты имени/URL и SubscriptionSource validation совпадают с add_source.
Флаг local_exception передаётся явно, Proxy route не меняется на Direct.

Мутация вызывает существующий Service::change_source с CAS. До первого await
посылаются отмены manual/scheduled/batch/TCP/connecting Flight; перед мутацией
ожидаются acknowledgements. Отменённый manual result не публикуется. Для
scheduled используется существующий cancel/ack и reload_id того же владельца:
window_until/background policy не заменяются и не сбрасываются. Pending review
источника освобождается. Readonly source_change ACK не содержит исходного URL.

RED: на backend 9fcccfaa callback test отвергается invalid_input.
GREEN: additive command принимает request и отдаёт matching ACK generation=2;
повтор с прежней generation отклоняется; неизвестные поля отвергаются.

## Direct hunk proposal для ROOT при интеграции PR113

Проверен источник `da1d59f20df6e5b03f04f6b3a3d02e46a1c9c622`,
`crates/yonik-desktop/src/backend.rs:3423`. В существующий predicate
«Отмена до первого await редактирования» добавить ровно:

```rust
| Command::SourceChange { .. }
```

Сохранить PR113 Flight::start, connecting/mutating, request_id,
source_generation/routing_revision и fence completion с system_revision(actor).
Не заменять Operation::PolicyConnect прежним Connect из базы b9e5071.
SourceChange-specific signals/drain остаются дополнением к общему predicate;
повторная установка отмены идемпотентна. SystemSession/Client не меняются.
PR113 в эту ветку не merge/rebase; проверка общего результата принадлежит ROOT.
