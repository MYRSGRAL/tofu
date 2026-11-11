# P07: предложение ROOT — сохранение неприменимого селектора

Статус: **reviewable proposal, production отложен решением ROOT**.
AndroidPackage ниже — частный вариант для обсуждения, не выбранная реализация
всего переносимого selector-контракта. Android runtime остаётся отложенным.
Единый bounded closed selector/Draft version/migration/fail-closed контракт ROOT
согласует после native broker/wire handoff; engine/private ABI принадлежат соседним
Go/anchor задачам. Настоящая записка не разрешает изменения Conditions или IPC.
Основание: routing-and-tray §7, data-and-contracts §§1/7, A-08.
Это отдельная эволюция модели с участием engine/wire владельца; её нельзя безопасно
реализовать только добавлением serde-поля в desktop RuleInput.

## Закрытая схема

Draft V2 сохраняет V1 поля и добавляет в каждое правило обязательный массив
`platform_sources` (0–16 элементов). Единственный начальный вариант:

```json
{"kind":"android_package","package":"org.example.app"}
```

Enum `PlatformSource::AndroidPackage { package: AndroidPackageId }`,
`deny_unknown_fields` на DTO и варианте, без `serde_json::Value`, flatten,
Other/raw/native JSON, PID, запуска Exec, UID/cgroup и executable mapping.
Package: ASCII, 3–253 байта, не менее двух разделённых точкой непустых сегментов;
сегмент начинается ASCII-буквой, далее ASCII буквы/цифры/underscore, регистр
сохраняется (не доменная IDNA-нормализация). Значение — идентификатор пакета,
не путь. Максимум 256 rules и 4096 platform_sources на Draft; общий существующий
лимит 262144 байта raw и 200000 байт селекторов включает новые значения.
Дубликат в одном правиле отклоняется; ID/order/enabled/action/default неизменны.
Это явные предлагаемые ограничения формата, а не новые продуктовые capabilities.

На Linux каждый AndroidPackage неприменим. Никакого превращения в executable.
Включённое правило с непустым platform_sources делает всю политику непригодной
для activation: неизвестная OR-ветка не удаляется даже при наличии executable.
Взаимосвязь будущих поддержанных platform_sources с executable потребует отдельного
контракта; эта версия не исполняет их ни на одной платформе.

## Все пути преобразования

Предпочтительный вариант — типизированный enum также в общей Conditions:
`platform_sources: Vec<PlatformSource>`. Это сохраняет disabled поля и в Policy,
не заставляет конвертеры выбрасывать данные. Расширение только после согласования.

| Граница | Требуемое поведение |
| --- | --- |
| Draft parse/storage validation | Проверяет закрытую схему, сохраняет enabled и disabled неподдерживаемые правила с ID/order. Не вызывает activation validator при чтении/сохранении |
| Draft::policy() | Переносит typed platform_sources без потерь; запрещено default/empty при конвертации V2 |
| Shared activation validator | До capability/engine generation возвращает явный UnsupportedSelector с ID для любого enabled platform_source; disabled не блокирует, поля сохранены |
| Domain evaluate_destination / compatibility evaluate | При enabled unsupported отказывает до выбора user/LAN/default; новый PolicyFault::UnsupportedSelector. Compatibility evaluate возвращает Block; не интерпретировать отсутствие поля как match-all |
| Explain | До evaluator выдаёт explicit unsupported с ID; не fixture/Decided и не application_witness_missing. Disabled не создаёт отказ, не меняет stored Draft |
| Diagnostics | Disabled не предупреждает; enabled unsupported даёт явную неполноту/неприменимость, не Shadowed как доказательство активируемости |
| Engine policy_input / SystemPolicy parse/serialize | Закрытая V2 схема сохраняет поле, общая проверка запрещает enabled unsupported. Нельзя получить SystemPolicy V1 из V2 с потерей поля, включая disabled |
| Оба native compiler entrypoints | Общий validator плюс собственный fail-closed guard до output; enabled unsupported всегда ошибка. Disabled не генерируется в native routing, исходная immutable typed Policy сохраняет данные |
| Prepare/requirements | Нельзя превратить AndroidPackage в attribution=false и активировать: SystemPolicy проверяется раньше. Wire version mismatch — explicit refusal без fallback в V1 |

## V1 compatibility / migration / старое приложение

- Чтение V1: platform_sources внутренне пуст; сохранять V1 до первого реального
  добавления platform_source. Простое открытие/переключение UI не переписывает БД.
- Добавление первого селектора: единственная CAS-транзакция encrypted Draft
  `version:2` со всеми прежними rules/IDs/order/default и новым полем. CAS failure
  оставляет исходный blob/revision. Никакой отдельной неатомарной миграции БД.
- V2 без platform_sources у rule не принимать как V1; неизвестный future kind,
  version или поля оставляют **весь исходный encrypted blob** read-only, без
  partial parse/пересохранения оставшихся правил. Нужен явный diagnostic.
- Старый V1 app уже отвергает version!=1 и unknown fields. Текущий backend.rs:2797–2818
  при parse failure устанавливает routing_revision=None; save_routing требует Some.
  Поэтому ожидается read-only сохранение V2 blob, но это **надо доказать** fixture
  запуска старого приложения, попытками edit/order/default/delete и reopen,
  побайтовым сравнением blob/revision. Только ошибка parse недостаточна.
- Старый app не сможет объяснить значение V2, но не должен подменять сохранённую
  политику пустым Draft или применить default. До enabling feature требуется
  подтвердить отказ activation/Explain в старом приложении при unreadable Draft;
  если старый бинарник допускает fallback, включение V2 блокируется до определения
  безопасной минимальной версии/миграционной стратегии. Нельзя исправить старый
  установленный бинарник новым serde-полем.

## Scope на согласование

1. Domain: policy types/closed selector validator, evaluation fail-closed guard,
   diagnostics и unit/integration tables.
2. Desktop: editor storage validation отдельно от activation; DTO conversion и
   Explain; backend только loading/read-only/mutation guards и их fixtures.
   Main только routing controls для отображения неприменимости, без layout.
3. Engine owner: policy_input, policy_wire/SystemPolicy V2 и оба compiler guards,
   requirements/Prepare rejection tests. IPC version propagation — отдельное
   согласование с владельцем, без изменения RPC envelope по умолчанию.
4. Старый app fixture: native/offscreen/private bus, synthetic encrypted DB;
   никаких пользовательских ключей, host settings, workers/engine запусков.

RED/GREEN до изменения каждого guard: включённый selector с совпадающим обычным
условием и разные действия должен получить explicit refusal, включая все
конвертеры и Explain; disabled — exact stored roundtrip и допустимый обычный
маршрут. Матрица V1→V2/CAS/reopen/old-reader preservation, unknown kind/field/version,
лимиты 16/17 и 256/257, диагностическая неполнота. Bounded 10/100 правил.

ROOT должен согласовать этот scope и владельца engine/wire до production работы.
Текущий PR124 реализует только actual Qt/persistence proof и эту записку.

## Проверка существующего V1 reader

В actual Qt fixture проверено read-only поведение текущего V1 reader на заданном
синтетическом V2 blob с enabled #41 и disabled #42: два запуска, пять попыток
мутации и Explain в каждом, ciphertext и revision побайтово неизменны, repository
после закрытия возвращает исходные bytes. См. 2026-11-11-p07-qt-persistence.md.
Это не реализация typed V2, не доказательство V1→V2 migration и не проверка
активации всеми converters/compiler/SystemPolicy. Обязательный leaf сохранения
неприменимого platform selector остаётся открытым, P07 не complete.
