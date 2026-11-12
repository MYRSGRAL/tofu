# Named DNS downstream: компонентная Linux проверка

Новый TestNamedDownstream в обоих native patches использует настоящий core,
controlled TCP nameserver, штатные resolver/router/outbound и TCP/UDP echo peers.
DNS reply удерживается барьером до изменения authority. Никакого mock dialer:
положительный результат требует echo, счётчик TCP accept/UDP receive измеряет egress.

Xray: IPOnDemand, default blackhole, разрешённый freedom только для 127.0.0.1/32.
Sing-box: resolve action, route по 127.0.0.1/32, последующий reject.
Это существующая семантика native configs; compiler/configRoot/ABI не менялись.

Для TCP и UDP проверяются Known, cancel, revoke, epoch, Unknown, Unavailable,
stale во время DNS await и после прогрева cache реальным положительным flow.
Cache cases дополнительно запрещают новый nameserver exchange. Changed-answer
возвращает 127.0.0.2; wildcard peers обнаруживают ошибочный loopback egress.
Независимый negative oracle меняет разрешённый CIDR на 127.0.0.2/32 и требует
падения Known TCP/UDP echo у обоих cores.

Evidence вне TTL: /home/myrs/projects/yonik/local/evidence/dns-downstream-e5a27.
Там находятся исходники тестов, команды check.sh/oracle.py, race/vet/build логи,
бинарники и проверка применения patches к закреплённым исходникам.

Границы доказательства: authority создаётся закрытыми test-only fixtures
ClosedDNSFixture/ClosedSocketFixture. Это не authenticated wire issuance.
Отрицательные случаи измеряют нулевой peer egress в ограниченном окне, а не
системный socket syscall counter. Coalesced concurrent downstream callers и
изоляция независимых соседних flows этим новым тестом ещё не покрыты.
Набор не закрывает весь named DNS acceptance contract. Production issuer,
foreign UID admission, mixed-policy Rust activation остаются заблокированными.

Свежие проверки: race named 30 cases/core, DNS regressions, vet и сборки обоих
cores, builder 18, Ruff check/format, clean patch/hash verification — PASS.
Обязательные Rust clippy 1.85.1 и cargo fmt в этом checkpoint не выполнены:
wrapper не получил общий slot за 924 секунды; собственное ожидание отменено
до запуска Cargo. Rust source не менялся. Полную приёмку продолжать отдельно.
