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
