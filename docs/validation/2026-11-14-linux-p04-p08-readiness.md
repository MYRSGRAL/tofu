# Готовность исходных P04 и P08

Source anchor: `b25ed1184d917a4ba9465b000b4bec080cfcaa3e`.
После независимых criterion joins, exact-head CI и merge P04/P08 становятся
«Готов». Теперь 7 готовы (P01/P02/P04/P06/P08/P18/P22), 17 частично выполнены,
2 отложены (Android P03/P26), знаменатель 26. Исторические срезы не переписаны.
Это исходные критерии плана, не полный Linux PASS и не current whole-tree runtime.

## P04 / FR-07

Основания: [контракт импорта](../specs/data-and-contracts.md),
[reconciliation пяти bundles](p04-fr07-accepted-evidence/README.md),
[ledger](p04-fr07-accepted-evidence/ledger.json).
Покрыты текст/файл, URI/JSON/Base64, bounded распознавание/нормализация,
ручная форма, безопасные ошибки с original, review/approve и сохранение.
Проверены 93 declared hash checks и 12 288 воспроизводимых robustness inputs.
Исторические source bounds, 96 source deltas и утрата `observed.jsonl` сохранены:
независимая перепроверка actual write bytes по утраченному trace невозможна.
Остальные lifecycle/results и source review подтверждают original/save invariant;
новый runtime на всём принятом дереве этим не приписывается.

Независимые проверки: R2 `e320b07c` — source coverage;
R1 `2e40f9fe` — evidence join с одним fail-closed P2;
R1 `6c9de8fc` — исправление P2 CLEAN, 8 tests PASS;
R1 `1559f877` — exact main union CLEAN;
criterion join `cbc64ba7` — Ready после CI/merge.
PR #197 head `bf032301a7654fb56da16a890a2d8dcf2667b10b` получил
10 SUCCESS и принят merge `eb6eff382f59c8dd1d9203a8bd7891d6f39442e9`.
Условия выполнены. Compiler performance P04A, native activation и full Linux
не входят в этот вывод.

## P08

Основания: [compiler evidence](2026-11-09-engine-compiler.md),
[positive SS2022 witness](p08-ss2022/README.md),
[capabilities contract](../specs/data-and-contracts.md),
[граница config-check / Linux-сети](../specs/validation-and-roadmap.md).
Сохраняется точный поддерживаемый набор шести протоколов и ранее проверенных
сочетаний transport/security/options/rules. Положительный импорт → compile →
точные method/key JSON для `2022-blake3-aes-128-gcm` прошёл оба pinned checker:
Xray 26.10.31 и sing-box 1.14.4, exit 0. Отрицательные key/chacha gates сохранены.

R1 `98e4079b` подтвердил source/ELF/config-check evidence CLEAN.
Rust closure неизменен; whole-head freeze не заявлен: Python entrypoint добавлен
во время сборки и отдельно привязан к final source hashes.
Criterion join R2 `682a3f25` — Ready после CI/merge без нового установленного
конкретного пробела исходного критерия. PR #203 head
`777d50bfbce18959a272c1087446d627a1ef1534` получил 10 SUCCESS и принят merge
`5e4c72850b4f3932243a312a001eac20053ef5f0`.

Явные Unsupported для требующих real-resolution mixed domain/CIDR,
неподдерживаемых DNS contracts и остальных capability gates сохраняются.
Config-check не предоставляет native permit. P13/P14/P15, D-01…D-07,
реальный трафик/handshake, runtime DNS, attribution и N5 остаются отдельно.

## Остаток

P11 ждёт итоговой source freshness и gate. P21 functional tray evidence
не превращается в полный physical T-04 PASS. Final A5, packaging/source closure,
worker FR16, Linux network gates и все остальные partial строки остаются открытыми.
Android и physical G-UI/G-POWER deferred, NOT PASS.
