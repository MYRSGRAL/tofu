# Принятые Linux-срезы: 12 ноября 2026

Актуальная принятая delta до main `d74e618`: [реестр 14 ноября](2026-11-14-linux-accepted-delta.md). Полные статусы не повышены.

База этого реестра — main `cceede520842a17e4ef548807f63f64b2be19226`.
Это обновление доказанных срезов, не полный acceptance seal. Все строки
ограничены своими frozen source/runtime evidence; последующее слияние само по
себе не доказывает повторение runtime campaign на новом бинарнике.

| Направление | Принятое evidence | Точный оставшийся gate |
| --- | --- | --- |
| Импорт P04/FR07 | PR134: [product text/file](2026-11-12-desktop-local-import.md). PR138: [12 288 deterministic mutations](2026-11-12-import-robustness.md), не coverage-guided fuzzing. | Manual stack #135/#139/#140 и local lifecycle #137 ещё вне этой main-базы; интеграция и короткий witness итогового source. Полный cross-product всех transport/UDP не установлен требованиями FR07. |
| Хранение P06/P07 | [Реальный Secret Service](2026-11-11-real-secret-service.md), [format4/verified backup3](2026-11-12-p07-linux-v2.md). | Actual product GUI format4 с production key custody и требуемые recovery boundaries; synthetic persistence key этих проверок не заменяет. |
| Политики P07 | PR131: [closed selectors, disabled preservation, fail-closed enabled](2026-11-12-p07-linux-v2.md), [PolicyApply integration](2026-11-12-p07-policy-apply-integration.md). | Production process/application и mixed destination enforcement с actual Apply. |
| Пресеты P17 | PR116 объединяет product wiring с принятыми P05/P07/P11: [integration](2026-11-12-pr116-final-integration.md), [parent sync](2026-11-12-p17-parent-main-sync.md). | Mixed domain/CIDR и полная Linux native matrix остаются; system_tun=false не повышен. |
| Structured errors P11 | PR128/132/136/142: [actual original Main copy](2026-11-12-p11-production-ui.md), [interval](2026-11-12-p11-interval-production-ui.md), [owner exhaustion](2026-11-12-p11-owner-exhaustion.md). | Известные потери cause в subscription/probe/maintenance; incoming interval transaction #143 и maintenance A/B #141 отдельно. Каждый enum не требует новой GUI campaign. |
| Native cancellation | PR129/130: [LocalSession cleanup](2026-11-12-native-cancel-matrix.md), [health cancellation](2026-11-12-health-probe-cancellation.md). | Полный production system lifecycle, allocator-pool баланс и долговременный plateau не следуют из component terminal cleanup. |
| Tray FR19 | PR133: [реальный private Plasma](2026-11-12-fr19-private-plasma.md), оба core × RU/EN, peer-sensitive traffic control. | Физические G-UI/G-POWER отложены без PASS. Сохранение KWin отличать от полного compositor restart. |
| Broker/native | PR112 safe broker принят; PR122 и PR126 остаются отдельными незавершёнными слоями. | Canonical production issuer/BoundTun, kernel admission, foreign-UID и attach→DNS→route→dial matrix. Закрытый gate не является положительной приёмкой. |
| Arch package | PR119 уже принят: [production custody](2026-11-11-arch-production-custody.md), [source provenance](2026-11-11-arch-source-6cf65a4.md). | Итоговый package/helper/cores/notices из конечного source freeze и соответствующая isolated upgrade/rollback/uninstall приёмка. |

## Сохраняемые условия полного завершения

Полный перечень FR/gates/D/A/T/performance остаётся в
[индексе критериев](2026-11-11-linux-acceptance-index.md) и исходных specs.
Окончательный seal требует requirement-by-requirement сверки итогового дерева,
прямых evidence нужного scope и независимых review. Этот реестр не закрывает
Linux по числу PR или зелёных checks. P01/P02/P18 остаются тремя ранее готовыми
пунктами; повышения остальных полных статусов здесь нет.

Два внешних bounded unsafe proposals — fixed kernel observations и три TUN ioctl —
ожидают решения пользователя. До него новый unsafe не применяется; результат
observation сам по себе не даёт production admission. Android и физические
G-UI/G-POWER остаются отложенными. Публикация release не производится этим PR.
