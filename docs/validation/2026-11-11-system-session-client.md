# Системный клиент профиля и desktop adapter

База: `5157139600ff9a26fe456f19a6b9796ecbeda195`.

`SystemClient::prepare_profile` сериализует только version/mode/engine/kill_switch/ServerIntent.
Публичный `prepare` защиты сохранён. Общий RPC сохраняет проверку sender, JSON object,
закрытые DTO и Message body ошибок. Debug профиля и handle скрыт.

`SystemSession` сериализуется владельцем через `&mut self`. Он хранит только
ревизию выбора, одноразовый handle и состояние helper. Host обязан вызывать
`revise` при каждом изменении source/profile/generation; старые команды получают
Stale. `revise`, Stop и Unlock уничтожают handle. Apply потребляет его до await.
Потеря owner видна в snapshot; ошибки и отмена мутации оставляют Unknown.
TunnelStarting и VerifiedRules остаются состояниями helper, без вывода о
transport ready или Connected. Failed cleanup не превращается в закрытый туннель.

Adapter предназначен для последующего подключения GUI после L16. Backend/QML,
scheduler и production capability не менялись. При system_tun=false новый Connect
получает Unsupported. Application PreparedPlan требует локальный config digest и
проверку artifact: IPC adapter не подделывает эти доказательства и не создаёт
параллельного reducer подключённости.

Проверки используют только созданную тестами private D-Bus. Строгая fixture
проверяет закрытый ServerIntent при Prepare, затем принимает Apply. Это проверка
wire pipeline, а не запуск production helper, TUN или engine. Synthetic credentials,
malformed intent, malformed helper replies, чужой sender, owner change, deadline,
cancellation, stale revision, one-shot handle и capability gate проверяются
в `crates/yonik-linux/tests/system_client.rs`. Исходник adapter подключён в integration
тест через path: дополнительные зависимости desktop не нужны.

RED: cargo +1.85.1 check -p yonik-desktop — E0599 отсутствует prepare_profile.
Все тяжёлые команды: jobs=1, собственный transient CARGO_TARGET_DIR, BSD flock
на корне и /home/myrs/projects/yonik/local/orchestration/with-rust-slot.

GREEN: `cargo +1.85.1 test -p yonik-linux --test system_client` — 6 passed,
включая финальный повтор после добавления отменённого Apply.
`cargo +1.85.1 clippy -p yonik-linux -p yonik-desktop --all-targets -- -D warnings`
— успешно. `cargo +1.85.1 fmt --all -- --check` и `git diff --check` — успешно.


## Повтор Prepare при сохранённой блокировке

Локальная проверка adapter ошибочно считала `VerifiedRules`/`RecoveryRequired` доказательством активной системной сессии. Она удалена: admission, наличие сессии и worker uncertainty по-прежнему проверяет helper; capability gate и revision/one-shot fencing сохранены. Private-bus regression воспроизвела `Err(Busy)` вместо успешного Prepare (RED). GREEN: семь integration tests, включая оба retained-policy состояния и успешный Stop → Prepare без Unlock; scoped Linux all-targets clippy и fmt прошли. Состояние после Stop в fixture — VerifiedRules, RecoveryRequired отдельно приходит через State. Это проверка adapter/RPC, не production TUN/GUI и не доказательство разрешения helper при реально активной сессии. Логи: `local/validation/pr77-retained-policy/` основного workspace.
