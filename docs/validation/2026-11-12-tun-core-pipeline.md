# Приватная Linux приёмка TUN → core → outbound

Приёмка запускает реальные constructors обоих pinned cores в собственной
user/net namespace. Существующий закрытый fixture выполняет authenticated
HELLO/FEATURES/ATTACH/ACK с bound TUN. Kernel TCP/UDP clients отправляют packets
через ynkfixture0, настоящий routing и outbound к controlled loopback peers.
Environment только выбирает isolated launcher; production authority не создаёт.

Для cancel, EOF, REVOKE и stop передаётся положительный TCP/UDP payload. TCP peer
затем пишет 32 MiB при остановленном чтении client: core writer удерживает данные.
После terminal writer обязан завершиться с ошибкой, следующий UDP payload не
должен возвращаться. До teardown fixture проверяется отсутствие core TUN read /
ConnectionManager copy goroutines. После cleanup повторные восемь сценариев не
увеличивают FD/goroutine count относительно прогретого baseline. Повторный revoke
проверяется существующим fixture. Это проверка освобождения активного пути;
прямых счётчиков внутренних allocator pools приёмка не добавляет.

Приёмка обнаружила sing-box inbound copy leak: закрытие outbound не будило
заблокированный inbound writer. Inbound TCP/UDP теперь принадлежат существующей
Session через OwnSocket/Release и существующему ConnectionManager cleanup при
stop. Новые публичные hooks и контракты не добавлены. В Xray production delta
не потребовалась.

Оба negative controls отключают закрытие Session tracked resources: та же
приёмка падает на незавершённом writer. Дополнительный sing-box RED до inbound
fix сохраняет stacks заблокированных copy goroutines и FD growth. Финальный
race GREEN обоих cores проходит, Go vet и production builds обоих cores проходят.
Builder: 18 tests; Ruff; MSRV 1.85.1 clippy all-targets -D warnings; cargo fmt.
Parent b94c737995123a36e311749c63ba25ce63f4be97 перенесён обычным merge.

Evidence и binaries вне TTL: /home/myrs/projects/yonik/local/evidence/pipeline-e5a27.
run.sh, build.sh, negative.py и acceptance.json фиксируют команды и provenance;
xray-race.log, sing-final-race.log, *-red.log, sing-diagnostic-green.log,
dns-red.log, dns-green.log содержат результаты. patch-verification.json проверяет
применение к чистым pinned archives и все manifest package/callsite hashes.

Linux Rust issuer остаётся fail-closed, unsafe permission pending. Эта приёмка
не доказывает production ATTACH или OS foreign-UID admission. Android,
физические G-UI/G-POWER и production campaign отложены. Pins/ABI260/capabilities
не изменены. DNS lifecycle исправлен отдельным commit 822fb99.
