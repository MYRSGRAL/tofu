# Private bootstrap v3: аутентифицированное наблюдение ядра

База: `540b0eb3c8d9e2592c09445c7ad5fe225a705a38` (PR159).
Native ABI260, зависимости, unsafe и capabilities не меняются.

## Закрытый контракт

Отдельный offer `YNKANC03`, challenge `YNKNS003`, report `YNKPR003`.
Каждый frame имеет ровно 128 bytes. Challenge сохраняет nonce 8..40,
digest 40..72, session 72..88, generation BE 88..96, invocation 96..112;
112..128 должны быть нулевыми. Report копирует все binding bytes 8..112,
содержит три errno i32 BE в 112..124 и нулевые 124..128.
Report не содержит FD. SCM credentials проверяются на каждом ответе.

Helper в v3 ожидает ровно один report перед namespace frame, затем closed-ACK.
Ошибки размера, credentials, binding, reserved, версии, порядка и срока
терминальны; автоматического fallback нет. Явные worker v1/v2 сохранены.
Пробы выполняются trusted worker до receive_payload, только через существующий
`yonik_nsfs::kernel_probe::observe()`.

Допустимые наблюдения: unshare 1/22, setns 1/9, clone3 1/22/38.
Ноль и неизвестные errno отвергаются. Эти множества не являются permit policy.
Legacy clone и per-flag enforcement остаются недоказанными;
`ProductionPermit::mint()` и verified bootstrap закрыты. Run payload после
v3 observation не отправляется, пока этот отдельный production gate закрыт.

Report opaque, хранится с net/pid custody, без mount FD. RunAuthorization
связывает session/generation/network epoch с этой custody. Общий owner/network
revoke синхронно закрывает FD и удаляет report; deadline делает то же самое.
Повторное связывание отвергается, продления существующего report нет.

## Проверки и границы

Тесты actual private SEQPACKET/SCM_CREDENTIALS проверяют поля challenge,
missing/duplicate/late/unknown report, PID/UID, short/long frame, лишний FD,
число открытых FD, expiry и revoke. Отдельно проверяются все разрешённые errno
и изменение каждого байта report. Проверка общего RunAuthorization revoke
использует test-only observation и реальное удержание namespace FD.

Privileged installed-systemd и cross-UID VM campaign не запускались;
не считать host socket tests доказательством installed unit enforcement.
