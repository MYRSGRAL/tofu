# P04(plan) / FR-07: reconciliation accepted evidence

Accepted source: `8ea826a04a745c8916e14a4329588c75f66b683e`.
Это **P04 из docs/PLAN.md** (URI/JSON/Base64/ручной ввод), не performance P-04.
R2 functional coverage audit предоставлен root как исходная предпосылка;
здесь проверяются только внешние данные и переносимость source binding.
Никакие binaries, GUI, engines, Rust/Go или runtime-сценарии не запускались.
Действующая toolkit v6 campaign не опрашивалась и не изменялась.

## Результат

Пять canonical bundles найдены: **286 файлов / 785630388 bytes** захешированы
без копирования/раскрытия содержимого. **93 declared log/binary/core hash checks
совпали**, дополнительных mismatch нет. Retained manifest binary hashes,
source-manifest hash, accept log и две lifecycle trace hashes проверены отдельно.
Все пять source refs являются ancestors accepted head.

Из **2039 source-manifest entries** (есть повторы между bundles) **1943**
побайтно совпадают с accepted blobs на том же пути; **96** отличаются.
Это различия исходников, не повреждение evidence. Все пути, обе SHA256,
сравнение с историческим ref и абсолютные evidence paths — в `ledger.json`.
Whole-tree/new-binary equivalence не заявляется.

Корпус robustness: framing всех **12288** inputs проверен. Повторена только
генерация synthetic mutation bytes из трёх accepted fixture files и seed
`0x921420261112`, без вызова production parser. Полный digest совпал:
`401c7f9299a57f4d42414d760c0d1de33471e6087d544b0879ef5ac7231ba51f`.
Шесть фиксированных sample digests сохранены в ledger, payload не выводится.
Runner из старого absolute worktree path сопоставлен retained copy и accepted
path `crates/yonik-import/tests/evidence/run_robustness.py`: byte-exact SHA.

## Requirement → witness → граница продвижения

| Требование | Проверенные данные | Что допустимо учитывать |
| --- | --- | --- |
| P04(plan), FR-07: JSON/Base64/URI, bounded ввод, original/error preservation | `p04-robust-9214`: 14/14 source entries exact; corpus/ELF/runner hashes; 5 tests PASS в campaign.log, включая явный negative redaction test | Узкий bounded robustness witness на accepted parser source. Не универсальная fuzz guarantee и не новая execution |
| FR-07: text/file → review/approve, encrypted save, original, отказ unsafe path/операций | `fr07-desktop-import`: 1516/1567 source entries exact; ELF + manifest binding; 84 logs hashes; old-reader result согласован с manifest | Исторический local-import/format4 witness. `local_import.rs`, bridge ABI и tests exact; старый Main.qml/backend не выдаются за текущий GUI/backend run |
| FR-07: шесть ручных протоколов, TLS/WS/gRPC, validation/clear | `fr07-manual`: 5/8 entries exact; два retained ELF + два core hashes; QtTest 8 PASS, backend denials PASS, native matrix test PASS | ManualEntry.qml, manual_native.rs, manual_qml test и manual_compile.rs exact; 12 cases/24 config checks — historical result плюс matrix-test log, не повторный запуск |
| FR-07: imported profile → новый GUI → Connect/Stop, lifecycle, local remote-operation denial | `fr07-integrated`: 407/449 entries exact; retained ELF, denial log и lifecycle traces verified; 1 valid + 1 wrong-peer case, обоим доступны direct endpoints, closed/reaped | Интегрированный historical witness с реальным negative peer и UI Stop по сохранённым trace/result. Parser/QML/harness equivalence отдельно от изменившихся backend/bridge |
| FR-07: native transport identity, TLS+WS/gRPC | `pr127-oracle`: acceptance.rs SHA exact, binary/accept.log/two core hashes verified, 12 synthetic compiled JSON + 12 native logs hashed | 12 исторических config checks. На каждом case assertion oracle отвергает 4 identity mutations: 48 controls следуют из exact harness и 12 завершённых PASS; отдельных 48 строк лога нет |

Область достаточна для **узкого evidence promotion P04(plan)/FR-07** совместно
с R2 implementation audit: внешние данные теперь воспроизводимо reconciled,
вместо общей отметки «bundle не проверен». Она не закрывает полный Linux,
physical UI/power, packaging, всю performance P-04 и не вводит обязательный
protocol×transport×UDP cross-product или новый parser/fuzz проект.
Существующий acceptance index/PLAN здесь не переписывался: решение promotion
остаётся root с учётом явно указанных source differences.

## GUI и backend: разные source boundaries

Parser diff `bf97e32e639ab3b01ca2feb119c7d103441af3f4 → accepted` пуст.
Все 10 файлов `yonik-import/src` совпадают также с ref pr127-oracle.
Это не равенство desktop/engine деревьев.

- Старый desktop bundle: Main.qml/backend.rs/backend_startup.rs/build.rs изменены.
- Более поздний integrated bundle: **Main.qml, ManualEntry.qml, bridge.rs,
  local_import.rs, manual/lifecycle harnesses** exact accepted blobs. Все QML
  файлы, перечисленные в этом manifest, exact.
- Integrated **backend.rs, backend_startup.rs, bridge_impl.rs, desktop_schedule.rs,
  maintenance_job.rs, diagnostics/error_context/presets и build.rs** отличаются;
  часть engine/Linux modules тоже отличается. Byte-identical mapping для них
  отсутствует, semantic transfer новым исполнением не доказывался.
- В manual bundle различаются Main.qml/build.rs/manual_entry_smoke.py; более
  поздний integrated manifest связывает текущий smoke/QML, но не делает старый
  manual binary новым accepted binary.

Два исторических ref обозначают base, не полный immutable build tree:
robustness source manifest включает новый robustness.rs поверх report base 3763803;
integrated source-binding включает Python manual_encrypted_connect.py overlay
поверх bf97e32. Эти два файла exact accepted, но не совпадают с blob base ref.
Desktop 1567/1567 и manual 8/8 source entries совпадают со своими recorded heads.
Ни один mixed base/harness binding не объявляется полным exact binary build tree.

## Recorded-only / точный недостающий материал

Все пять **каталогов** и заявленные manifest/log/binary paths найдены.
Digest `actual_writes_sha256` обоих integrated cases записан как
`fc55b75ab345cf6cdd42a7f1701e84e05f3548e96c654156fd27293f06d0a198`.
Producer хешировал `<task_root>/observed.jsonl`, затем transient root был очищен.
В canonical bundle файл
`/home/myrs/projects/yonik/local/evidence/fr07-integrated/observed.jsonl`
**отсутствует**. Поэтому digest actual writes подтверждён только согласованными
result/report, а не повторным хешированием оригинальных write bytes. Поиск
ограничен canonical bundle/report; другие пользовательские диски не сканировались.
Это точная граница воспроизводимости, а не повод запускать binary заново или
сохранять/показывать реальные configs. DB hashes/old-reader backup hashes тоже
остаются recorded attestations: содержимое DB/config здесь не читалось.

## Воспроизведение data-only

Из repository root, только stdlib Python и git:

```sh
python -B docs/validation/p04-fr07-accepted-evidence/reconcile.py --output /tmp/p04-fr07-ledger.json
cmp docs/validation/p04-fr07-accepted-evidence/ledger.json /tmp/p04-fr07-ledger.json
python -B -m unittest discover -s docs/validation/p04-fr07-accepted-evidence -p test_reconcile.py -v
ruff check docs/validation/p04-fr07-accepted-evidence
ruff format --check docs/validation/p04-fr07-accepted-evidence
```

Exit 0 означает успешную data integrity сверку; source deltas остаются в ledger
и не преобразуются в PASS нового runtime. Два новых negative tests отклоняют
truncated/oversized framing и обнаруживают однобайтовую подмену при прежнем числе
records. Они проверяют validator, не повторяют старые Rust/GUI negative controls.
Исторические probes/traffic/config checks в ledger всегда помечены как historical.
