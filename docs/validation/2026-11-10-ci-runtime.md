# Desktop CI: длина пути UNIX-сокета

Падение desktop на `9cbe2de` воспроизведено с установленным бинарником `85b2a8e`:
ошибка возникает до загрузки QML. После ребрендинга runtime-путь в CI стал
`/home/runner/work/_temp/yonik-runtime` (37 байт). Вместе с `/tofu-` и SHA-256
полный путь составляет 107 байт; Qt отказывается создавать сервер.

## Воспроизведение

Синтетический smoke с отдельным data-dir и явным shutdown:

| Runtime, байт | Полный путь сокета, байт | Exit |
| --- | --- | --- |
| 25 | 95 | 0 |
| 34 | 104 | 0 |
| 35 | 105 | 0 |
| 36 | 106 | 0 |
| 37 | 107 | 1 |
| 38 | 108 | 1 |
| 40 | 110 | 1 |

Результаты получены локально с Qt 6.11.2; нельзя приписывать им выполнение
на runner с другой Qt. Исходное CI показывает тот же мгновенный exit1.

## Исправление и проверки

Workflow создаёт короткий приватный runtime через `mktemp -d /tmp/yonik-qt-XXXXXX`.
Для cleanup передаётся именно output шага создания; унаследованный
`XDG_RUNTIME_DIR` не используется как цель удаления. После job выполняется
`rmdir`, удаляющий только пустой созданный каталог. Legacy socket/lock prefix
и пользовательские данные приложения не меняются.

- profiles, partial, tcp-basic: PASS с коротким runtime (22 байта, mode0700).
- После сценариев runtime пуст; cleanup подтверждён.
- YAML разобран, все desktop run-блоки прошли bash -n; diff check чист.
- Независимый read-only review: существенных регрессий нет; замечание о выборе
  cleanup-пути устранено использованием output шага вместо inherited env.
- Результат нового GitHub CI ожидается. Это отдельная обязательная проверка.

Evidence вне git: `local/validation/whole-ux/ci-runtime-*.json` и
`local/validation/whole-ux/ci-short-runtime/`.
