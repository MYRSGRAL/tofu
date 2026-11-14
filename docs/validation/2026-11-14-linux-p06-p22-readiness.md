# Готовность исходных P06 и P22

Принятый source anchor: `3247e6c32a5ed83ec5d812dfc1f70695d3f2b3b4`.
Решение изменяет две строки текущего PLAN: P06 и P22 становятся «Готов».
Всего 26 строк: 5 готовы (P01/P02/P06/P18/P22), 19 частично выполнены,
2 отложены (Android P03/P26). Исторический реестр 256 исходных строк и
срезы с прежним счётом 3/21/2 не переписываются.

Готовность относится ко всему исходному критерию каждой из этих двух строк.
Она не означает исполнение нового whole-tree binary после каждого merge.
Final A5, остальные FR/сети/поставка и полная Linux-приёмка остаются открытыми.
Физические G-UI/G-POWER остаются deferred, NOT PASS.

## P06: production key custody при восстановлении

Норматив: [storage contract](../specs/data-and-contracts.md),
[план приёмки](../specs/validation-and-roadmap.md). Исторические
[recovery](2026-11-10-storage-recovery.md),
[backup](2026-11-10-storage-backup.md),
[настоящий Secret Service](2026-11-11-real-secret-service.md) и format4
проверки дополнены [actual Restore NEW](2026-11-14-p06-real-restore.md).
Отчёт нового witness сам по себе закрывает один gap; готовность строки
получена соединением этих доказательств, а не расширением вывода одного отчёта.

Witness выполнялся на production source
`8ea826a04a745c8916e14a4329588c75f66b683e`, без production-правок.
PR #198 head `e2c11a1544e4b53585b853c180d7ac3b3914937e` принят merge
`99093d5f0efdf63e9e9c6a119f8d87dcbfeebe9b` после 10 SUCCESS и независимого
custody review. Последующие #201/#202 не меняют production storage path.

Normal Host Backup 101 → Restore NEW 102 → fresh normal Host reopen подтвердили
original/metadata/revision и тот же единственный private Secret Service item,
без ephemeral injection/fallback/создания нового ключа. Missing-source 103
отклонён без output и изменения live DB. Matching completion и Closed ACK,
source/ELF binding, cleanup описаны в witness и его
[manifest](../evidence/p06-real-restore-20261114/manifest.json).

Независимый source/evidence review: `b32b8a89` — CLEAN в custody gap;
criterion join `cbc64ba7` — исходный P06 Ready после CI и merge.
Условия выполнены. Hardware PAM, OS picker, login/lock-unlock и политика
автоподключения после reboot относятся к отдельным платформенным/финальным gates;
в эту строку им не присваивается PASS.

## P22: batch GUI и привязка результатов

Норматив: [проверки серверов](../specs/validation-and-roadmap.md),
исходная строка P22. Требуемое runtime evidence не заменяется чтением кода:
сохраняются [TCP series](2026-11-09-tcp-series.md),
[bounded batch](2026-11-09-tcp-batch.md),
[интегрированный producer](2026-11-10-batch-gui/integrated.md) и
[actual GUI](2026-11-14-p07-production-gui.md).

Frozen feature `6bf810b14f2159ad79f72c9fca81d27c12d506d6` объединён с
accepted main как `6d3b53acc929d2467be1f415d6403691f19f2018` (#177).
Исторические executable/source bindings сохранены. Strict CA positive/negative,
SOCKS refusal, concurrent batch/import/log pressure, CancelBatch socket cleanup
и Stop core cleanup относятся именно к описанным executable. Отдельный
[passive hidden proof](2026-11-14-p07-hidden-consumer.md) не расширяется до
утверждения о нулевой внутренней работе или physical power.

На source anchor этого решения проверены: bounded 10 000 targets,
TCP parallelism 1–32 и backpressure, GUI concurrency 8/timeout 5 s,
каналы по 16, страницы по 32, endpoint/source/revision CAS и network stamp,
отмена и отказ поздним результатам по generation/source/revision/network.
Между feature, merge и anchor batch producer/DTO, TCP API и batch QML не менялись.
Изменения общего backend сохраняют batch guards/network consumer; изменение
network publisher ограничено `cfg(test)`. Проверка semantic transfer не
переименовывает старый ELF в сборку anchor.

Независимый criterion audit `984f43cc` и normative join `f5ecbf3f` не выявили
потерянного обязательного producer-инварианта. Поэтому исходный P22 принят
на связанном историческом/component evidence. Короткий current GUI
Cancel/network-epoch witness остаётся способом подтвердить отдельный
current merged-runtime PASS при final A5; такого нового PASS здесь нет.

## Не повышенные статусы

P04 ждёт CI/merge #197; P11 — свежесть итоговой интеграции #196/#199 и её gate.
P21 имеет функциональные retained tray proofs, но полный T-04/physical вывод
этим решением не закрывается. Ни одна из этих строк здесь не повышена.
N5 issuer, Runtime16, production callers/factory, native DNS/G-APP и финальная
поставка остаются самостоятельными незакрытыми условиями Linux-приёмки.
