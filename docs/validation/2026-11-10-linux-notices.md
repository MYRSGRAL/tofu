# Linux notices — 2026-11-10

База f8975948406c920c4d78439348001394647d17dd, включая metadata #50.
Scope: P25 Linux, без Android и без релиза. План/requirements/platforms прочитаны.

Вход воспроизводится командами из [README](../../packaging/licenses/README.md).
[Реальный report](../../packaging/licenses/linux-inventory.json) содержит 225
packages выбранной Linux source closure с gui, хеши рецепта и первичных текстов.
Выбранный граф не приравнивается к runtime binary. Все unknown перечислены в README.

Targeted RED: тесты до реализации завершаются ошибкой отсутствующего модуля;
затем отрицательный тест features выявил общую mutable ссылку на ARTIFACTS.
После deepcopy 7/7 GREEN. Проверяются missing_license, missing_notice,
изменённый текст лицензии, неверные features, удалённый package и dev-only edge.

Реальный `--output` и `--check`: exit 1, ровно восемь missing_license:
clang-format 0.3.0, codespan-reporting 0.11.1, cxx-qt-build/gen/lib/macro/
cxx-qt 0.10.0, qt-build-utils 0.10.0. В локальных распакованных crates нет
обнаруживаемого первичного текста; SPDX не подставляет текст автоматически.
Это открытые findings, а не успешная release-проверка.

Первичные лицензии pinned cores получены отдельно с официальных raw GitHub URLs
и сохранены с SHA-256 в upstream/sources.json; новые зависимости не скачивались.
Проверка не делает юридического вывода о совместимости/полноте source obligations.

Rust исходники не менялись, build/check/test/clippy не запускались: проверяется
Python-инструмент над Cargo metadata, сборка Rust не нужна. Rust fmt и Python
ruff format/check проверены. Target не создавался; собственный transient root
удалён после проверки. Никакие сетевые настройки/keyring/установка не затронуты.

## Исправления независимого review

license_file ограничен package root; для локальных workspace crates разрешён
корневой LICENSE/LICENCE/COPYING. Абсолютные пути и выход за разрешённые корни
отклоняются. Чтение выполняется FD-relative с NOFOLLOW для каждого компонента,
только regular files, до 4 MiB на файл. Пустые/пробельные LICENSE и NOTICE
отклоняются, исходные принятые байты не нормализуются.

Непроверенный report больше не определяет ни путь исходника, ни digest bundled
text при I/O: используются только заново построенные ожидаемые записи metadata.
Bundled text записывается относительно NOFOLLOW directory FD с O_EXCL; перед
публикацией сверяется digest прочитанных байтов. Symlink texts/report не следует.

Отрицательные тесты дали RED: absolute/parent/symlink source, внешнее чтение
через sha256 report, пробельные тексты и symlink bundle directory. GREEN:
12 целевых tests; полный штатный tools/tests discovery 137 PASS, Ruff format/check
PASS. Добавлен CI loader для существующей tests/tools матрицы. Первичные
лицензионные тексты не менялись.

Реальный повтор inventory/check: 225 packages, прежние 8 missing_license,
exit 1 ожидаем. Это открытый gap P25, не успешная приёмка лицензий. ELF/Qt/Go
composition по-прежнему unknown. Логи и повторный report вне TTL:
/home/myrs/projects/yonik/local/validation/pr68-path-fix/.
