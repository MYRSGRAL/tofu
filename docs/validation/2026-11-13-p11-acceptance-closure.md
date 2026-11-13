# P11: конечный acceptance leaf

База: main `5fd6153839a8c55f67ba31ddbe9ae2d403328a1f`.
Машиночитаемая таблица: [p11-acceptance-closure.json](p11-acceptance-closure.json).
В ней сохранён исходный R2 inventory на `cceede520842a17e4ef548807f63f64b2be19226`:
12 production boundaries и отдельно Import148/Startup154. Review task IDs
bda669/684 не являются Git refs. Incoming refs не слиты в эту ветку.

Каждая строка содержит точный sourceSHA, Git blob SHA исходника,
изменение относительно frozen source, publication chain и representative V2
identity (key/code/stage/slots/generation). Эти identity не являются исчерпывающим
списком enum: единица closure — production boundary. Полные contracts проверяются
динамически по JSON, без дублирования списка причин.

Статусы closed обозначают уже принятый source mapping. Incoming обозначает
наличие исправления в опубликованном ref, не принятие в main. Open — итоговый
gate интегрированного head; весь P11 этим leaf не закрывается.

Проверки:

- `python -m unittest tools.tests.test_p11_acceptance_closure tools.tests.test_p11_factory_coverage tools.tests.test_diagnostic_export_v2 tools.tests.test_check_diagnostic_export -v`: 21 PASS.
- `python tools/check_translations.py`: 374 inline-пары, 160 ключей, 17 QML.
- На точных incoming refs 145/149/152 отдельно извлечены catalog, V2 JSON и Main.qml
  через git show; тот же gate RU/EN и всех reason × slot × generation: 6 PASS.
- `git diff --check`: PASS. Ruff отсутствует в PATH; format/lint не заявлены.
- Rust/GUI кампании не повторялись. Новый gate проверяет контракт, но не заменяет
  Rust producer/export tests и integrated-head final gate.

SourceFailure проверяется по реальным Cause-ссылкам production части файла:
все такие причины должны существовать в V2. Эта проверка ловит пропущенные
contract entries, но не доказывает достижимость каждой ветки. Publication/fences
проверены чтением exact source; новые runtime claims не делаются.

Для maintenance rollback primary copy/replacement cause имеет precedence:
ошибка закрытия влияет на drained=false, отдельный errno не изобретается.
Probe Transport и service Storage сохраняют доступную типизированную категорию;
они не являются доказательством конкретной OS-причины. Generic fallback сам по себе
не объявляется багом.

PR160 `8fff27dbe58892a6831e9171530f6dad8c9da900`: FR16 disk rotation отдельно.
Его backend delta добавляет disk guard и shutdown, не новый P11 metadata mapping.
Helper issuer-dependent actual cases остаются за пределами этого независимого leaf.
Probe-v2 proposal не применён. Production/error_context не изменены.
Evidence лежит в docs/evidence/p11-closure вне TTL; тяжёлых сборок не было.
Следующий шаг: отдельный review leaf, затем root final gate на integrated head.
