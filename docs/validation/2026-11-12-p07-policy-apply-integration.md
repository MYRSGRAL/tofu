# Linux P07: сохранение selectors при PolicyApply

Интеграция P07 с main `3f73b6a` выявила потерю `platform_sources` в промежуточном
inline-конвертере. Connect уже имел отдельный guard; новый System RoutingApply
использует тот же конвертер, поэтому ограничение нельзя полагаться сохранить
только проверкой Connect.

Два RED-контроля подтвердили: enabled selector превращался в правило без этого
условия, disabled selector исчезал при roundtrip. Исправление отклоняет enabled
selector до сериализации с `Unsupported`. Disabled selectors обоих списков
сохраняются через существующий SystemPolicy V2; правила без selectors продолжают
использовать V1 без нового поля. Новый protocol/native ABI не вводится.

Отдельно согласованы incoming fixtures: обязательный пустой `platform_sources`
в Conditions и контроль установленного P07 бюджета 256/257 вместо прежнего 128/129.
Production budget этой интеграцией не менялся.

## Проверка

Rust 1.85.1, `--offline --locked`: 178 PASS, 2 ignored; GUI/test-support Clippy
all-targets `-D warnings`, fmt, factory 2/2 и RU/EN inventory — PASS. Положительный
контроль сохраняет disabled user/preset selectors, отрицательный отклоняет оба
enabled списка; отдельный контроль подтверждает прежний V1 и границу 256/257.
C++ warnings toolchain сохранены в логе; Rust Clippy завершился успешно.

Evidence и retained binaries вне TTL:
`/home/myrs/projects/yonik/local/evidence/pr131-policy-main/manifest.json`.
RED E0063, старый budget assertion и два selector RED сохранены отдельно.
Этот документ добавлен после проверок и отдельно хеширован в manifest.

Новая GUI runtime campaign и production native admission здесь не выполнялись.
Прежние P07 Qt/repository proofs сохраняют свои refs и границы; полный P11,
Android runtime и физическая приёмка этим результатом не подтверждаются.
