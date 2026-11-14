# P25 USED_INPUT_BYTES

`_p25_used_input_bytes.retain_used_input_bytes(record, record_sha256,
observed, observed_sha256, output)` — post-capture API без subprocess, discovery,
сетевых запросов и SDK inventory. Первые четыре аргумента — retained bytes
`build-capture-record.json`, его независимый SHA256, `build-cxx-observed.json`
и его независимый SHA256. Pins должен получить trusted caller из сохранённого
capture manifest / independently verified evidence, а не вычислить из недоверенных
входов и объявить доверенными. Схемы взяты из `_p25_final_profile.enrich`.

`output` — абсолютный путь нового каталога внутри workspace без TTL. Родитель
должен существовать. Каталог создаётся exclusive с mode 0700; повторный вызов
в тот же каталог отвергается. API не меняет capture, controller или admission.
Объекты называются SHA256; `manifest.json` хранит ID, origin, exact SHA и размер.
Это вход для последующего origin+SHA attribution join; автоматического подключения
к `attribute_capture` нет.

Читаются только header/sdk-unit IDs из normalized dependency rows успешных
observed units, совпадающих с record units/argv. Наличие в files/input_origins
само по себе не означает использования. SDK origins ограничены trusted code
profile `/usr/include/qt6`; caller и JSON не могут передать дополнительные roots.
Тесты подменяют константу только для synthetic temporary fixtures.
Все компоненты открываются NOFOLLOW, файл обязан быть regular. Проверяются SHA,
fstat до/после, размер и повторный pathname walk/parent identity. Hardlinks сами
по себе не запрещены. Устойчивость проверки ограничена наблюдаемыми состояниями
filesystem; API не заменяет snapshot или защищённый capture producer.

Лимиты: 16 MiB на файл, 256 MiB реально прочитанных bytes (включая повторные
origins), 4096 origins/files, 10000 dependency rows; JSON record 1 MiB и observed
8 MiB. Потоковое чтение блоками 64 KiB; dedup сохраняет разные origins, каждый
повторно проверяется. Сложность линейная по metadata и прочитанным bytes.
Объекты, manifest и каталог проходят fsync; при ошибке нет успешного возврата.
CaptureError означает REJECTED, включая missing file, wrong hash, symlink,
mutation, exclusive-output и fsync failure. Незавершённый каталог может остаться
для диагностики, но без завершённого manifest; повтор требует нового output.

Отсутствие actual-used SDK rows/origin/SHA даёт `MISSING`. Generated dependencies
получают отдельный `generated-retained-bytes:<id>`: API не читает произвольные
build/source paths. Их bytes должны быть отдельно сохранены и проверены.
Records с уже заполненными generated/primary assertions здесь отвергаются:
для них необходим отдельный provenance join. Forwarding/qconfig bytes сохраняются
полностью, но generator identity не выводится из текста или имени.

`RETAINED` означает только сохранение выбранных bytes, никогда PASS.
`legal_status=NOTPROVEN`, `full_source_closure=OPEN`,
`generator_identity=MISSING`; final acceptance/admission всегда false.
File/package/license identity, upstream patches/config, generator/input/output и
final artifact binding остаются независимыми обязательствами. OR/WITH election
не выполняется. Несовпадение tar c27/49e остаётся NOTPROVEN, не превращается в
универсальный license gate. Исторические 142/23/195 refs не меняются.

Проверки только synthetic fixtures:

```sh
python -m unittest discover -s tools/tests -p 'test_p25_used_input_bytes.py'
python -O -m unittest discover -s tools/tests -p 'test_p25_used_input_bytes.py'
```

Реальные actual-used inputs остаются MISSING до нового build/capture; эти тесты
не являются доказательством состава Qt или полного Linux payload.
