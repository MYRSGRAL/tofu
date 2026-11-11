# P-04: нижний слой external rule-set compiler

Историческое состояние encoder/custody. Продолжение: [bound resource bundle](p04-bound-resource-bundle.md).

База: `f3c937d717a97345dee3bc5bb79127fe4c96ae1e`.
Этот slice не закрывает P-04 и не включает caller paths.

`yonik_engine::external_rules::compile_records` принимает seekable immutable
источник и явный writer. Формат строк: `exact:<canonical-domain>`,
`suffix:<canonical-domain>` либо canonical CIDR, каждая запись завершается LF.
Используются существующие `DomainName` и `Cidr`; regex не поддерживается.
Два линейных прохода с ограниченной строкой (300 байт), без AST всего набора.
Первый проход валидирует весь источник и размер результата до записи writer.
Второй записывает protobuf geosite/geoip или sing-box source v3.
Host обязан закрепить immutable источник на оба прохода и публиковать writer
только после успешного результата: ошибки seek/I/O не гарантируют откат writer.

Формат Xray соответствует pinned
[geodat.proto v26.10.31](https://github.com/XTLS/Xray-core/blob/v26.10.31/common/geodata/geodat.proto),
поля GeoSiteList/GeoIPList и canonical CIDR. Тег `YONIK`: pinned loader приводит
искомый код к верхнему регистру. Для sing-box source поддерживается конвертация
закреплённой CLI `rule-set compile` в binary SRS.

`yonik_linux::rule_set_custody` содержит bounded typed reference и шифрование
XChaCha20-Poly1305 с SHA-256 тела. AAD связывает digest, byte size, record count,
format/version. Неверное тело не вызывает запись ciphertext; открытие читает
не больше заявленного размера + overhead + 1 и проверяет AEAD и digest.
Record count здесь является authenticated metadata, а не самостоятельным
парсингом native artifact: его должен связать host с результатом compiler.
Plaintext возвращается в Zeroizing; никаких persistent plaintext ресурсов
сам модуль не создаёт. Ключ передаётся trusted host; окружение ключом не управляет.

Заявленные ограничения: 1 500 000 записей, 128 MiB source/artifact на один вызов.
Это лимиты отказа, не измеренные RSS или показатели 10x acceptance. Custody
использует bounded whole-buffer AEAD и не заявляет 64 MiB builder RSS.
Inline selector budget, 64 KiB IPC, 4 MiB config cap не изменены.

## Минимальный контракт с root до изменения private IPC

Передавать manifest максимум 16 ресурсов: SHA-256, размер plaintext, число
записей, kind/format/version; без путей и rule bodies. Trusted host связывает
bundle digest с config digest и упорядоченным manifest. FD/lease передаётся
отдельно: read-only immutable snapshot, verified size/type/digest до Prepare;
lease удерживается до Stop. Ошибка подготовки сохраняет предыдущую generation.
Необходимы явные отдельные reservations для ciphertext, staging, native artifact,
active и rollback; не включать их скрыто в 4 MiB config.

Не реализованы: атомарный filesystem store/current/rollback, FD ABI, session
lease accounting, resource bundle attachment к существующему Policy/evaluator,
сохранение AND/OR при этом attachment и production `.srs` subprocess custody.
Нельзя добавлять refs только в JSON после legacy compile: это обошло бы
DNS provenance и process-attribution gates. Policy/DnsPolicy здесь не дублируются.
Helper/system transport остаётся отдельной обязательной зависимостью.

## Проверки

`cargo +1.85.1 test -p yonik-engine --test external_rules` проверяет golden DAT,
150 000 distinct records, canonical input/count/size refusal до первой записи.
Ignored `pinned_native_150k` отдельно запускается с `--ignored --nocapture`:
реальный Xray config-check, sing-box source→SRS и binary config-check.
Это проверки формата, не положительный isolated-netns membership gate.
Custody unit test проверяет roundtrip, отсутствие plaintext в ciphertext,
corruption, неверный digest/size и bounded-ref refusal.

Локальные логи и source hashes: `/home/myrs/projects/yonik/local/evidence/external-rules/`.
48 performance / 20 snapshot прогонов не повторялись.

Фактически полученные размеры для синтетического набора:

| Ресурс | Записей | Байт |
| --- | ---: | ---: |
| Xray domains.dat | 100000 | 2388902 |
| Xray cidrs.dat | 50000 | 500011 |
| sing-box domains source | 100000 | 3388913 |
| sing-box CIDR source | 50000 | 1550323 |

Native RED: lowercase tag дал Xray `failed to check code YONIK ... EOF`.
После uppercase tag оба config-check прошли; повреждённые DAT/SRS отвергаются.
Эти результаты не доказывают membership, DNS provenance или app attribution.
