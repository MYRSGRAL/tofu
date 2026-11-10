# Desktop integration SubscriptionScheduler — срез P10

База: `0dc6261fccba9c6ad82ee576b9ae5b8d8af25b97`.
Ветка: `t3/desktop-subscription-scheduler`. P10 целиком не закрывается.

Backend владеет одним bounded SubscriptionScheduler: максимум 20 000 записей
(лимит repository), один active fetch или review. Startup читает id/generation
страницами по 64 без чтения snapshot body; между страницами уступает runtime.
Новые источники планируются на сутки вперёд, существующие при старте — сразу.
Persisted due пока отсутствует: перезапуск не восстанавливает суточный deadline.
После изменений используется точечное чтение generation без полного обхода БД.
Формат, миграции, зависимости и контракт scheduler не менялись.

Один select объединяет команды, события scheduler и его общий абсолютный
монотонный deadline. Idle scheduler не создаёт polling timer, task на источник
или unbounded канал. next_event и finish_review получают часы завершения.
Каждый импорт, включая clean, публикует pending DTO существующего review UI.
До approve snapshot не меняется; discard освобождает staged report.
Ручной refresh сохраняет прежнюю семантику clean import и preferences persistence.

Перед delete, HWID/source change и ручным refresh выполняются cancel и ack.
При parallelism=1 отмена queued id во время чужого fetch может ждать окончания
этого fetch (штатный timeout 30 секунд); его event сохраняется для следующего
прохода UI, не теряется и не утверждается автоматически.
Stop отправляет отмену ручному refresh, session и TCP до первого await, затем
ожидает scheduler cleanup. Shutdown дожидается scheduler перед service.close
и освобождением runtime. При Stop оставшаяся очередь восстанавливается с
суточной отсрочкой. Ошибки очистки публикуются как storage.

## Policy и оставшийся platform-events gate

Qt ApplicationWindow передаёт visibility через существующий dispatch bridge.
Без видимого окна новые автоматические загрузки запрещены. Публичный
DesktopHost::set_subscription_policy принимает явный SubscriptionPolicy от
доверенного provider; JSON-командой задать policy нельзя. Provider обязан
разрешать online только при известной сети, известном metered и power state.
Default/unknown не разрешает dispatch. Policy регулирует допуск новых работ;
уже начатый fetch завершается либо отменяется командой/Shutdown.

Системный provider NetworkManager/UPower и D-Bus события в этом срезе **не
подключены**. Штатный Qt host остаётся на unknown и сам автоматическую сеть не
запускает. Это обязательный остаток до включения функции пользователям.
Тестовый provider использует тот же публичный policy input и реальный FetchHost
на отдельном loopback HTTP. Proxy сохраняет штатный отказ без DIRECT fallback.
Пользовательские БД, сети, подписки, keyring и helper не использовались.
Остаются persisted due, platform events, power/resume/background-window acceptance
и измерения энергии. Нет Android и изменений helper/network/import/applications.

## Проверки

Все Cargo-команды: Rust 1.85.1, jobs=2, target внутри собственного transient
`/var/tmp/codex-myrs/transient/yonik-desktop-scheduler/target`, под flock его корня.

Новые проверки: неизвестная policy и hidden не дают deadline; scheduled clean
fetch требует approve/discard; удерживаемый fetch отменяется при delete,
HWID/source generation change и shutdown; metadata API ограничивает страницы
64 записями, отражает новое поколение и удаление без snapshot.

Первый bootstrap-тест был выполнен уже после реализации: отдельный RED для
него не заявляется. Общий backend-прогон обнаружил реальную регрессию порядка
Stop: ожидание scheduler ack перед TCP cancel позволяло polling connect.
Существующий тест stop_and_shutdown_finish_a_pending_tcp_task_before_acknowledgement
упал, сигналы отмены перенесены перед await. Итог повторной проверки ниже.

Итоговые результаты:

- `cargo +1.85.1 test -p yonik-desktop -p yonik-linux`: 399 passed, 0 failed,
  24 ignored (namespace/VM gates не запускались).
- После последних уточнений cancel/completion clock и сохранения ручного review:
  `cargo +1.85.1 test -p yonik-desktop`: 43 passed, 0 failed, 1 ignored.
- `cargo +1.85.1 clippy --workspace --all-targets -- -D warnings`: exit 0.
- `cargo +1.85.1 fmt --all -- --check`: exit 0.
- GUI build с `--features gui`: exit 0; GCC/Qt headers дают существующие
  C++ diagnostic warnings, Rust clippy warnings отсутствуют.
- `preferences_smoke.py`: exit 0, два запуска настоящего Qt offscreen,
  восстановлены engine/probe_url/probe_status/language. Вспомогательные сценарии
  повреждённых preferences также прошли. Это не приёмка Wayland/tray/power events.
- `git diff --check`: exit 0.

Сырые логи и Qt smoke evidence сохранены вне TTL и не входят в commit:
`/home/myrs/.local/state/codex-evidence/yonik-desktop-scheduler-20261110`.
Автоматическая пересортировка Cargo.lock убрана; зависимости не менялись.
После завершения процессов target очищен штатным cargo clean.
Независимый review и CI PR остаются проверками root/CI.
