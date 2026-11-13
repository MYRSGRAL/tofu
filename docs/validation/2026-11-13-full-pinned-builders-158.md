# Full pinned builder acceptance обоих cores

База #158: `8ca7f3f83097b040b1566aa2c6a918b78653b1eb` (включает #156).
Production-код, зависимости, pins, ABI, caps, TTL, deadlines и retries не изменены.

Один full builder acceptance на ядро завершился PASS. Оба запуска последовательны,
с `GOMAXPROCS=1`, `-p=1`, `CGO_ENABLED=0`, из чистых pinned source checkout.
Builder проверил source HEAD/tag, patch SHA256 и metadata/package/callsite hashes,
неизменность go.mod/go.sum, все предусмотренные им package suites, собрал настоящий
production binary и подтвердил версию. Дополнительный `go mod verify` обоих cores — PASS.
Это полный набор package checks builder, а не `go test ./...` всего upstream.

Холодная компиляция отделена от исполнения: harness перед штатным package check
выполняет `go test` того же пакета с `-run '^$'` (тесты не исполняются), в том же
source/cache и с теми же флагами, с compile budget 900 с. Затем оригинальный
package check выполняется с неизменённым лимитом 180 с. Production build использует
штатный BUILD_TIMEOUT=900 с. Исходный builder не редактировался. Отдельная
72-positive campaign не запускалась; тесты внутри полного package suite исполнены.

| Этап | sing-box | Xray |
|---|---:|---:|
| Первый холодный compile-only | 195,45 с | 167,51 с |
| common/yonikbroker suite | 28,25 с | 19,78 с |
| route / dispatcher suite | 0,77 с | 0,47 с |
| TUN suite | 0,62 с | 14,52 с |
| Production binary build | 425,07 с | 106,39 с |

Эти длительности — wall time команд; они не доказывают отдельный native deadline.
Ни FAIL, ни skip не вводились. Старый cold compile timeout не засчитывается runtime FAIL.

ELF SHA256:

- sing-box: `8a1683227b52a94ef904f8cdfbb676b16f9cf4409e68c046d3a5c7acebe591b5`
- Xray: `68d802801b6be344f6fda5546803f2f38d49ace1cc50c4e37155fd0818d6083d`

Полные команды, toolchain/GOEXPERIMENT, upstream SHA, patch metadata, hashes
модульных архивов, stdout/stderr каждого шага, ELF headers и Go build info:
[full-pinned-builders-158](full-pinned-builders-158/acceptance-context.json).
Harness и его SHA сохранены; command log различает compile-only и package-runtime.
Capability/ABI manifest сохраняет `native_protocol: null`,
`runtime_hooks_connected: false` и исходные adapter_features, файловые capabilities
пусты. Production issuer, физический TUN и package admission не проверялись.
Это не финальный Linux package/admission seal и не bit reproducibility proof.

Бинарники и полные локальные evidence вне TTL:
`/home/myrs/projects/yonik/local/evidence/full-pinned-158-20261113`.
Артефакты на host не устанавливались. Transient root удерживался root flock;
после сохранения результатов его собственные Go caches/source/build удалены.
Rust не собирался, Cargo target не создавался.
