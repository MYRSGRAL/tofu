# L16: массовая TCP-диагностика desktop

База: `4216831e9cb92a0fb7b6b39eb2ed47a3ff1a654e`.

Добавлены независимые команды `check_batch`, `cancel_batch`, `batch_page` и
состояние `batch`. Проверяется весь выбранный источник, включая недоступные
профили; фильтр страницы серверов не ограничивает набор. До 10 000 целей,
параллелизм 8, канал 32, публичная страница 32 строки. Один producer вызывает
существующий `check_tcp_batch`; задач на отдельные профили нет.

DTO содержит публичный endpoint, ID/revision источника и профиля, network epoch,
статусы и время серии. Credentials и исходные URI не передаются. Медиана
публикуется только для трёх успешных проб. История не сохраняется.

Отмена использует существующий приоритетный TCP watch. Stop/Shutdown прерывают
producer до ожидания существующих fetch/scheduler acknowledgements. Общий
fetch-контракт не изменён. Смена источника/revision/сети очищает устаревшие
результаты. Смена поколения отмены сохраняет уже обработанные строки.

## Проверки

- 34 backend-теста прошли: существующие проверки и fixture на 250 различных
  профилей, ошибки TCP, страницы 32/26, отсутствие credentials, прямая отмена.
- 19 unit-тестов прошли, включая 10 000 результатов с ограниченными страницами.
- Отдельный тест смены выбранного источника проверяет очистку public results.
- Настоящий Qt, 420×760, offscreen/software: два отдельных запуска. В первом
  QML-кнопка проверяет 250 loopback-целей и показывает `250 / 250`; во втором
  loopback-listener с backlog 0 задерживает соединения, QML-кнопка отмены
  завершает серию с видимым `0 / 250 · отменено`. Оба запуска штатно закрыты.
- `cargo fmt -p yonik-desktop` выполнен.
- Desktop clippy `--all-targets --features gui --no-deps -- -D warnings` прошёл.
- Общий clippy остановлен существующим `collapsible_match` в
  `crates/yonik-engine/src/lib.rs:165`; файл не изменён.
- `ruff` отсутствует в окружении; Python fixture проверен исполнением.

Локальные логи и PNG: `local/evidence/batch-diagnostics/`. Финальная Qt-приёмка:
`qt-acceptance/{many,slow}/result.png`, `trace.jsonl`, `stderr.log`.
Бинарник сохранён вне TTL: `local/artifacts/batch-diagnostics/yonik-desktop`.
Сгенерированные evidence и бинарник не включены в коммит.

Продолжение приёмки и bulk API: [acceptance.md](acceptance.md).

Итог интеграции и сквозные runtime gates: [integrated.md](integrated.md).

Combined merge актуального main: [main-merge.md](main-merge.md).

Исправление review P2 для historical favorites: [unavailable-favorite.md](unavailable-favorite.md).

## Границы первоначального среза

Нет нового end-to-end batch-прогона на 10 000 сетевых целей и нового batch-теста
с реальным переходом network namespace/epoch. Для них остаются отдельные gates;
проверка 10 000 DTO не подменяет сетевой нагрузочный прогон. Поколение сети
проверяется перед публикацией и в основном select; существующие unit-тесты
network fencing проходят, но это не полная batch-приёмка смены сети.

Подготовка целей читает страницы и resolve профилей последовательно в actor;
отмена проверяется между чтениями. Задержка подготовки на 10 000 профилей здесь
не измерена. Изменения proxy writer #71 в эту ветку не включались, его contract
не менялся. Android, системная установка, пользовательские подписки/БД/keyring,
Docker и deployment не использовались.
