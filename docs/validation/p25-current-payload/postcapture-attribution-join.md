# Pure post-capture attribution attachment

Новый private API `tools/_p25_attribution_join.py:attribute_capture()` исполняет
join, который раньше существовал только как `capture-consumption.json` contract.
`p25_capture_build_closure.capture()` проверяет supplied record/refs, но не
сопоставляет captured origins с отдельным Qt candidate dataset. Его реализация,
producer/wrapper и historical bytes здесь не изменены.

## Конкретный API

```python
from _p25_attribution_join import attribute_capture

attachment = attribute_capture(
    manifest=retained_bytes["build-capture-manifest.json"],
    manifest_sha256=trusted_manifest_sha256,
    record=retained_bytes["build-capture-record.json"],
    report=retained_bytes["build-capture-report.json"],
    observed=retained_bytes["build-cxx-observed.json"],
    candidates=reviewed_candidate_bytes,
    candidates_sha256=trusted_candidate_sha256,
    source_fragments=fragment_bytes_by_retained_copy_path,
    primary_texts=primary_bytes_by_sha256,
)
```

Это функция над bytes, без CLI, filesystem/network discovery, unpack, query,
compiler или build. Caller читает только конечный whitelist с NOFOLLOW/caps и
сохраняет attachment отдельно от producer report. Pins manifest/candidates
нужны из независимо проверенного controller context и reviewed dataset; хеш
самоподставленного input не создаёт authority.

Например, manifest pin берётся из root-verified controller
`collected.json:artifact_sha256["build-capture-manifest.json"]`; candidate pin —
из независимо закреплённого candidate `manifest.json` entry для
`candidate-join.json`. Primary bytes передаются по SHA из уже доступного,
проверенного source offer/primary store. Source fragment map использует точные
`source_fragment.retained_copy` paths existing candidate rows. Прочитать весь
SDK или registry для этого API невозможно: таких arguments/operations нет.

## Что признаётся used input

Проверяются retained input pins, schema, source/profile/ELF consistency и
source/build roots. Для каждого report unit нужны совпадения:

- record unit и report unit по file/depfile/argv;
- единственный observed successful compile с тем же argv и normalized depfile;
- verified compiler identity в capture whitelist с observed compiler SHA;
- verified normalized depfile identity в record/report.

Только dependencies таких units попадают в `inputs`. Unit должен присутствовать
в собственном dependency наборе. Tool/ELF/query inventory не допускается в code
dependencies. При missing/conflicting compile evidence unit отражается в `units`
с точными issues, но никакие его claimed dependencies не повышаются до used.
Неиспользованные SDK file rows и candidate inventory не создают attribution rows.

SDK origins восстанавливаются только через observed `input_origins`. Source/
build origins — из зафиксированного root + relative path. Это metadata join,
а не новый filesystem resolution. Нет fallback по basename, Qt version, SPDX
identifier или одному hash при другом origin.

## Row classifications

| Classification | Значение |
| --- | --- |
| `MATCHED` | Origin + exact SHA совпали с единственным candidate; retained source-fragment bytes проверены по SHA и их SPDX совпал с candidate; primary bytes/refs доступны и согласованы |
| `MISSING` | Известен конкретный отсутствующий компонент: verified hash/origin, source fragment, primary bytes/ref attestation или file-level license declaration |
| `CONFLICTING` | Несовместимые identities/aliases, duplicate candidate, изменённые fragment/primary bytes, несовпадение SPDX или primary ref SHA |
| `UNKNOWN` | Actual used origin отсутствует в reviewed Qt candidate set; лицензия не угадывается |

Source-fragment match — ровно соответствие сохранённого installed source fragment,
не полная corresponding-source closure SDK. `candidate_options` сохраняет **все**
переданные candidate alternatives, не выбирает OR и не добавляет WITH. Primary
identifiers проверяются на наличие в исходном SPDX text как tokens; это не legal
SPDX parser. Generated outputs без такого exact candidate остаются UNKNOWN,
им не присваиваются лицензия generator crate или Qt output exception.

`bytes_verified` пересчитывается в этом вызове. Поле
`candidate_offer_refs_attested` обозначает только attestation из pinned reviewed
candidate dataset; архив и publisher signature здесь заново не проверяются.
File-level declarations отсутствуют у qconfig/forwarders: даже при совпавших
source bytes результат MISSING с `missing:file_level_license_declaration`.

Внешний статус всегда `ATTRIBUTION_REPORT`, с counts по классификациям. Source
capsule остаётся отдельным `MISSING`; его описание из candidate не выдаётся за
доказанную final SDK version или новое обязательство. `legal_status=NOTPROVEN`,
`final_linux_acceptance=false`, `production_admission_gate=false`. Ни capture
status, ни historical package142 не переписываются.

## Bounded schema и failures

Schema 1; неизвестные top-level/file/unit/dependency/candidate-row fields и
повторные JSON keys отвергаются. Record ≤1 MiB, manifest ≤1 MiB, observed ≤8 MiB,
report ≤32 MiB, candidates ≤4 MiB. Lists/maps ≤4096 items; подтверждённых dependency
rows суммарно ≤10000. Supplied blobs ≤1 MiB каждый; source fragments ≤1 MiB
суммарно, primary bytes ≤8 MiB. Output ≤8 MiB. Output хранит refs/notice excerpts,
не копирует все source/license bytes.

Нарушение pin/schema/caps, invalid paths/refs или omitted record units вызывает
`p25_capture_build_closure.CaptureError`: из непроверенного envelope used set не
выводится. Attribution gaps внутри согласованного envelope остаются row-level
MISSING/CONFLICTING/UNKNOWN. Отсутствие обязательного envelope input не подменяется
пустым report. SHA и list bounds используют runtime checks и работают под `-O`.

## Проверки и оставшийся input

Stdlib unittest fixtures проверяют positive origin+SHA/primary/source join,
ignored inventory, qconfig без выдуманного SPDX, missing/conflicting/unknown,
manifest/input/candidate pins, duplicate keys, byte caps, path/ELF/source/profile
consistency и missing/conflicting compiler evidence. Отдельный fixture вызывает
настоящий `p25_capture_build_closure.capture()` на синтетических files и затем
передаёт его полный output в новый API. Компилятор/Qt tools не исполняются.

Следующий factual input — четыре retained envelope files обязательной будущей
final build с trusted manifest pin. Только они выберут действительно used
candidate rows. Для source capsule/generator declarations сохраняется конкретный
input из `qt-file-attribution-candidate/candidate-join.json:single_missing_input`;
его отсутствие не скрывается положительным source-fragment match. Дополнительная
audit rebuild не нужна. Root review и legal OR/WITH decisions этим helper не
заменяются.
