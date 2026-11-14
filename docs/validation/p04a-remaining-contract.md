# P04A: оставшийся контракт, source-bound аудит

База: `40881657d0f534d34160c68d0d19cb9bf62d81e2`. **P04A OPEN**.
Новые performance runs не выполнялись. Пересчёт исходных трёх measured repeats
и SHA-256 входных evidence/исходников: [audit.json](p04a-remaining/audit.json).
Это привязка текущего исходника, а не утверждение, что старый ELF собран из него.
Исходные manifest сохраняют собственные source/binary binding и HEAD.

## Что уже доказано и не повторяется

- [Импорт/compile](p04-import-compile/README.md): 1000/10000 профилей,
  10/100/1000 executable AND CIDR, оба compiler backend, три measured repeats.
  Это внутренний compiler fixture, не приём 1000 правил UI.
- [Snapshots](p04-subscription-snapshots/README.md): 20 подписок, bounded
  retention по четырём раундам, реальные snapshot commit и compile.
- [Resource bundle](p04-bound-resource-bundle.md): 100k domains/50k CIDR,
  native membership и binding; [external compiler](p04-external-rule-compiler.md)
  измерил размеры файлов. Принятые PR109 VM 50k CIDR остаются prior component;
  здесь их окружение и raw logs заново не проверялись.
- [Cooperative cancellation](p04-p05-cooperative-cancellation.md) покрывает
  parser/stage/commit, но не даёт отмену resource compiler.

## Точный remaining gap

`tools/perf_import_compile.py::require_success` требует `mixed_domain_refused`.
Example проверяет лишь `.is_err()`: любой отказ удовлетворяет этому старому
контролю. Поэтому его PASS не является положительным proof mixed AND.

`resource_bundle::compile_bound_policy` вызывает
`routing::compile_policy_resources`: обычный вход не имеет app attribution;
`routing::validate_policy` отклоняет executables без tun sources.
Names отклоняет CIDR через RequiredRealResolution, IpOnly отклоняет domains
через DomainsInIpOnly. Existing `resource_bindings::
native_resources_preserve_unsupported_gates_and_small_config` фиксирует эти gates.
Evaluator AND сам по себе не является compiler/native proof.
API compile_bound_policy не принимает cancellation callback/token.

Не измерены на этом mixed compiler path: время/peak RSS, cancellation ACK,
отсутствие публикации после отмены, восстановление памяти после нескольких
циклов, размеры output при 10x personal rules. Старый child SIGKILL этого
не доказывает. Старые RSS и дисковый budget не заменяют recovery RSS.

## Минимальное решение root до production edit

1. Определить, доступен ли platform-independent **compile-only** mixed путь
   с verified resolution/process provenance; он не выдаёт native authority.
   Если runtime provenance невозможно представить без N5, оставить этот
   критерий OPEN. Не добавлять фиктивный Platform capability и не ослаблять gates.
2. Уточнить AND на исходном домене, реальном destination CIDR и executable;
   provenance должен связывать имя, адрес, порт и revision конкретного flow.
   Порт здесь проверяет provenance, а не вводит новый port selector в Conditions.
3. Добавить cooperative checkpoint/admission к actual bound compile/serialize
   с terminal Cancelled и отсутствием частичного bundle; повторный вызов после
   отмены должен завершаться. Конкретный DTO/enum/API утверждает root.
4. Сохранить UI 256 rules/64 group и существующие wire/config bounds.
   1000 personal app-rules требуют утверждённого bounded grouping либо явного
   refusal; отказ записывается как REFUSED, никогда как PASS1000.

## RED fixture для утверждения контракта

Это спецификация RED, **не новый выполненный Rust/native тест**.
Synthetic offline input: app `/opt/fixture/app-0`, domain `a.example.invalid`,
CIDR `203.0.113.0/24`, destination `203.0.113.7:443`, action Block,
fallback Direct. Одна и та же policy/revision и sealed resource manifest.
Ожидается successful bound compilation обоих backend с сохранённым AND.
Текущий ordinary bound path возвращает Unsupported(ProcessAttribution);
системный IpOnly также не принимает domains. Новый positive fixture должен
быть RED на этих отказах, а не принимать их как требуемый результат.

Обязательные независимые negative controls после решения root:

| Мутация | Ожидаемый результат |
| --- | --- |
| Другой app при том же destination | Direct |
| Другое имя при том же IP | Direct |
| CIDR miss при том же имени/app | Direct |
| Provenance от другого port или revision | Refusal до публикации |
| Перестановка двух CIDR refs при прежнем ordered digest | Refusal |
| Cancel до admission / отключённый checkpoint | Cancelled без bundle / RED |
| Искусственно уменьшенный output budget | Refusal, не truncated success |
| 10x personal rules | Измеренный bounded output либо явный refusal |

После разблокирования: 1 warmup + 3 measured runs, counts 10/100/1000,
normal 1000 servers/20 subscriptions/100k domains/50k CIDR; stress 10000 servers
и 10x personal changes. CIDR-адреса не разворачиваются. Собирать wall/CPU,
peak RSS, serialized bytes, native count, post-cancel/post-success RSS по циклам
и sample variance. Бюджеты инженерные; до actual measurement они не достигнуты.

Full native P04B/GAPP/TUN остаётся OPEN и зависит от N5. Config-check pinned
ELF подтверждает формат, но не mint authority, Apply или process attribution.

## Решение root и продолжение

Предложение выше сохранено как история исходного gap. Root утвердил compile-only
data artifact без flow receipt/N5 prerequisite; activation остаётся отдельным gate.
Исполняемый RED: [p04a-mixed-red](p04a-mixed-red/README.md). Реализация и targeted
GREEN evidence: [p04a-unadmitted](p04a-unadmitted/README.md).
