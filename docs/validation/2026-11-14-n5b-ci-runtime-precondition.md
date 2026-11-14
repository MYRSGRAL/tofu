# N5b: предпосылка real-userns runtime gate

Оба Rust job feature head `62971b56ecbbb24bd482055e23f6e5370256ffbc`
завершились отказом `unshare: write failed /proc/self/uid_map: Operation not permitted`
до запуска дочернего verifier fixture. Это отказ предпосылки платформенного
стенда, а не успешный runtime proof и не установленный дефект verifier.
Логи jobs `132249043507` и `132249021467` сохранены root вне TTL.

Только `verified_transport_userns_roundtrip` помечен `ignore` с явной причиной.
Его тело, запуск exact child, все восемь transport/FD controls и assertions
сохранены. Настройки runner, uid mappings, права и production gate не меняются.
Остальные детерминированные проверки продолжают выполняться обычным CI.

На отдельном разрешённом same-UID userns стенде запускать точный gate:

```sh
cargo +1.85.1 test -p yonik-linux --lib -- \
  --ignored --exact worker::role_handoff::tests::verified_transport_userns_roundtrip
```

Обычный CI с ignored fixture не доказывает actual roundtrip. Сохранённые прежние
локальные evidence относятся к feature source и своим runtime prerequisites;
нового runtime на merged head этим изменением не заявляется. Kernel admission,
foreign UID, full factory и runtime новых 16 probes остаются закрытыми.

Включение accepted main `36cc6a9` — обычный merge без ручного разрешения;
он сохраняет исправление disk restart oracle #183. Никаких новых production
изменений сверх уже проверенного N5b это обновление не добавляет.
