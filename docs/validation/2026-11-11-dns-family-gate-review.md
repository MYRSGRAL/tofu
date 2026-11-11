# Достоверность gate отключённого семейства

Проверяемый исходный head: `c170f68ab729de20d1872781b2d4bf0633741a3f`.
Исправлена только Python fixture и её unit-регрессии; compiler и native JSON
не менялись. Исходное evidence автора сохраняется отдельно без перезаписи.

Старый gate принимал timeout/ошибку разбора как отсутствие IPv4, а upstream
всегда отвечал AAAA и падал при A-запросе. Контроль с единственным изменением
`sing-box-FakeIp-v6.json`: `strategy=prefer_ipv6` завершился exit 0.

Теперь upstream возвращает правильное семейство A/AAAA, ошибки всех fixture
потоков учитываются после их завершения. Disabled-family требует реального
структурно корректного ответа: NOERROR без адресов либо явный NXDOMAIN/REFUSED
без адресов. Timeout, SERVFAIL, malformed response и IPv4-адрес отвергаются.
Дополнительно запрещён выход `disabled.test A` в upstream. Потоки имеют
ограниченное ожидание завершения; их ошибки не могут исчезнуть из результата.

Финальная неизменённая fixture прошла четыре actual IPv6 cases обоих pinned
ядер (Real/Fake-IP), включая marker, unknown fake и пустую новую сессию.
Все четыре disabled-ответа: NOERROR, 31 байт, upstream A не наблюдался;
список ошибок пуст. Тот же mutant strategy завершился exit 1 с assertion
«IPv4 адрес получен». Это RED старого oracle → GREEN исправленного oracle,
а не новая реализация DNS compiler.

19 профильных Python-тестов прошли, включая A/AAAA upstream, перехват ошибки
потока и отрицательные ответы/timeout/повреждение. Ruff format/check проходят.
Rust, исходные config-check и полная IPv4 matrix повторно не запускались:
изменений Rust/compiler не было. TTL/epochs, положительные exclusions,
network reset и полный G-DNS остаются открытыми.

Evidence основного workspace: `local/validation/pr80-family-fix/`:
`mutant-old/`, `final-normal/`, `final-mutant/`, `unit.log`, `manifest.json`.
Запуски выполнялись в отдельных user/net namespaces, host network не менялась.
Хеши source fixture и использованных JSON зафиксированы; native binaries
проверены существующим `validated_binary` по закреплённым SHA-256.
