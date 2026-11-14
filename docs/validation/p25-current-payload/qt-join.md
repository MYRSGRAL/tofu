# Ограниченный Qt/header/generated join после c005b53

Это delta к неизменному ledger package142, а не новые байты package142.
[statement-qt-delta.json](statement-qt-delta.json) содержит old/current statement
hashes и 195 Qt primary refs, проверенных **внутри actual release-source.tar**
против сохранённых SHA. UI RU/EN статически проверены парно: controller payload
с cores/source archives и отдельный legacy external-provider variant. GUI не запускался.

| Источник | Идентификаторы / текст | Actual source offer и attribution |
| --- | --- | --- |
| Qt base/declarative/svg 6.12.0, `composition/qt-sources.json` | LICENSES/LGPL-3.0-only.txt `da7eabb7bafdf7d3ae5e9f223aa5bdc1eece45ac569dc21b3b037520b4464768`; GPL-3.0-only `8ceb4b9ee5adedde47b31e975c1d90c73ad27b6b165a1dcd80c7c545eb65b903`; BSD-3-Clause `9f0490f18656c6f2435bd14f603ef0c96434d1825615363dce43abb42ed1dcce` | Все эти primary bytes сохранены в `release-source.tar:src/packaging/licenses/composition/texts/<hash>`. Их наличие не выбирает условия конкретного файла |
| Qt base/declarative primary exception | Qt-GPL-exception-1.0 `40678d338ce53cd93f8b22b281a2ecbcaa3ee65ce60b25ffb0c462b0530846b2` | Primary текст в том же source offer. Его применимость к фактически включённому tool output не установлена; автоматический WITH не добавлен |
| CXX-Qt Rust/native bridge | Сохранённые MIT/Apache-2.0 тексты и SPDX declarations в Rust inventory; `vendor/cxx-qt-{MIT,Apache-2.0}.txt` | 662 Rust/Go primary refs проверены исходным reconciler; inventory build/runtime union не превращён в доказательство каждого embedded фрагмента |
| Desktop generated pipeline | `crates/yonik-desktop/build.rs` использует `CxxQtBuilder::new_qml_module`, bridge.rs, application.cpp, resources.qrc, notices.qrc; Qt Quick/Widgets/Network/DBus | Build script и собственные QML/bridge sources находятся в release-source. Нет нового compile/link-map replay; historical eec7 maps не перенесены на package142 |
| SDK headers, moc/rcc/QML generated output | Нужны file-level copyright/SPDX notices фактически retained header/template/generated sections, generator/version и применимые output exceptions | Точная attribution chain header/generated→package142 ELF не содержится в этом join. В actual outer package нет отдельного Qt SDK source archive; source offer содержит primary texts/исторические manifests, не доказанную полную соответствующую Qt source closure |

**Точный оставшийся gap для root:** отсутствует проверенная привязка конкретных
Qt SDK header/generated fragments финального ELF к их file-level primary
license expressions/copyright и build/source inputs. Поэтому нельзя выбрать
LGPL/GPL/exception, установить точный mandatory text set или утверждать исполнение
corresponding-source obligations на основании одного aggregate qt-sources.json.
Это source-attribution gap, не доказанное отсутствие всех 169 текстов:
169 hashes вне standalone notices index **есть внутри actual release-source.tar**.
До установления file-level применимости не предлагается новое legal OR/WITH
решение или добавление всего historical Qt inventory в notices.

Минимальный следующий root шаг остаётся один: final-payload compliance reconciliation
перед A5, с retained build/header/generated attribution конкретного freeze.
Statement fix уже внесён в source; новый package не построен. Новые формулировки
не исправляют старые COLLECTED bytes задним числом. Historical package78 SCOPE
остаётся byte-identical; ссылка на shipped SCOPE объяснена в SOURCE-OBLIGATIONS.
Новых требований SLSA/OS reproducibility, checksum RE и whole-OS audit нет.

Проверки этого delta: 195 Qt primary refs byte/hash verified внутри actual
source offer; static paired RU/EN PASS; reconciler replay byte-identical старому
ledger; compile-notices tests; Ruff format/check. Старое замечание README об
отсутствии Ruff относится к c005b53; для delta использован разрешённый абсолютный
`/home/myrs/projects/yonik/local/orchestration/tools/ruff-0.17.4/ruff`.
Независимый R2 frozen-c005 review назначает root; этот delta не заявляет его результат.
