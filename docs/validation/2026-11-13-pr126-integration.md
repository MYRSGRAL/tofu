# Интеграционный native candidate #126

Исходная ветка `t3/e5a27eb0`, head `a57a96208cd3b9815ff65111139415ee757ca980`.
Обычные merge без конфликтов и ручных правок:

- `639820e55325258a8bbf4293fe2ba9fb06e0572c`: родители исходный head и #158 `8ca7f3f83097b040b1566aa2c6a918b78653b1eb`.
- `68c95af2607af45d073da107cf23aa5e121b9172`: родители первый merge и main `5fd6153839a8c55f67ba31ddbe9ae2d403328a1f`.

Оба remerge diff пусты. Adapters (mode и blob), Go-файлы, module manifests,
patches и builder совпадают с #158 и #167. Rust/UI (`crates/`, `bench/`,
корневые Cargo manifests) совпадают с принятым main. Pins, ABI 260 и
capabilities новых изменений не получили.

#167 `ce356b452675036fab3388ef64ea03f8f00910ce` остаётся full pinned builder
свидетельством Go source #158. Его исходные manifests сохранены без изменения.
Это не сборка объединённого Rust candidate. Полный native issuer, foreign UID,
физический TUN и финальный package admission этим merge не подтверждаются.
Ветки #156/#158/#167 не изменялись; история #126 не переписывалась.

Лёгкие проверки: 18 builder unittest, 6 diagnostic export unittest,
`cargo +1.85.1 fmt --all -- --check`, `cargo +1.97.0 fmt --all -- --check`,
`rustfmt` 1.85.1 для host fixture, Ruff 0.17.4 check/format трёх затронутых
Python-файлов — PASS. Новые тяжёлые Go/Rust сборки не запускались.
Fresh CI точного pushed head требуется отдельно, PR остаётся draft.

Полные tracked SHA256, mode/blob всех исходных refs и candidate, точные родители,
source binding и копии manifests #167 вне TTL:
`/home/myrs/projects/yonik/local/evidence/pr126-integration-20261113`.
Transient/build-каталоги для этой интеграции не создавались.
