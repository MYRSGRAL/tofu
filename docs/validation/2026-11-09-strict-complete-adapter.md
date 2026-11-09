# Первый native adapter: полный A/AAAA в sing-box

## Граница изменения

Проверка исходников закреплённых ядер показала, что необходимые гарантии не
появляются от одного общего hook. Этот этап исправляет конкретную потерю ошибки
семейства в sing-box v1.14.4, commit `c005f35300ce76427dc285481ec5695f5b179cd2`.
Производственный Go diff — 13 добавленных строк и 1 удалённая; остальное — тесты.
Основная policy/application часть tofu остаётся Rust, data-plane — native engine.

Route action `resolve` получает opt-in `strict_complete`. Его значение передаётся
в DNSQueryOptions. Оба пути объединения A/AAAA — явный transport/client и новый
DNS rule engine — возвращают ошибку, если обязательное семейство не удалось.
Успешный NODATA не равен ошибке. При IPv4Only/IPv6Only не добавляется второе семейство.

В legacy DNS ошибка строгого объединения помечается отдельной причиной, чтобы
не скрываться переходом к следующему resolver rule. Это не запрет внутренних
speculative `evaluate` запросов DNS DSL: проверяется итог обязательных семейств.
Default/false сохраняет upstream семантику. Односемейный address-limit fallback
также сохраняется: ревью выявило его регрессию, RED-тест воспроизвёл отказ,
после исправления тест прошёл для IPv4Only и IPv6Only.

Первичные источники: [client v1.14.4](https://github.com/SagerNet/sing-box/blob/v1.14.4/dns/client.go),
[router v1.14.4](https://github.com/SagerNet/sing-box/blob/v1.14.4/dns/router.go).
[Патч и команды сборки](../../adapters/sing-box/v1.14.4/README.md).

## Сборка и manifest

`tools/build_native_adapter.py` проверяет source HEAD/tag, чистоту дерева и hash
канонического patch, применяет его через git apply, собирает с -mod=readonly,
-trimpath и -buildvcs=false. go.mod/go.sum проверяются до/после сборки. Go cache,
module cache, tmp и логи находятся в собственном transient task root под flock
его inode. Toolchain локальный, GOTOOLCHAIN=local; скрытые GOFLAGS/GOENV отключены.
HOME не переназначается. Упавший/частичный запуск не перезапускается вслепую.

Бинарник и manifest сохраняются вне TTL. Manifest включает source commit/tag,
patch/binary SHA256, Go version/GOOS/GOARCH/GOEXPERIMENT, точные tags/args и hashes
зависимостей. Наличие feature не означает прохождение полного strict DNS gate;
`strict_dns_gate=false`. Это metadata воспроизводимой процедуры, не подписанная
аттестация и не доказательство побитово одинаковой повторной сборки.

На целевой машине собран Linux/amd64 c Go 1.27.1-X:nodwarf5. Фактический patch SHA256:
`32043478abfe5acc73666b302dbb9185acafb112f384bee7ae7d679e0c38c8c7`.
Binary SHA256:
`94815afaac6324f5d7973e0da40101a9d5dc8fc3e225361e6eec54e583e6a397`.
Артефакт: `local/artifacts/sing-box-strict-v1.14.4/`; он не заменяет официальный
engine, используемый desktop. Android/другие архитектуры этим Go build не проверены.

Ревью builder выявило ложное заявление feature для произвольного --patch и
сохранение потомков после timeout. Оба случая воспроизведены RED-тестами.
Теперь feature связан с каноническим patch; каждая команда владеет отдельной
process group, которую завершает и reap до освобождения root lock. Дополнительный
RED-тест воспроизвёл сигнал при восстановлении handlers после Popen; cleanup
охватывает и это окно. Тесты проверяют реального потомка через pidfd и сигнал
в точке восстановления handler. Host processes/сервисы не затрагиваются.

## Нативная матрица

`tools/dns_complete_smoke.py` всегда запускает worker в новых user/net/PID/mount
namespaces, с отдельным /proc. Публичного worker-аргумента обхода launcher нет.
До ip-операций проверяются отдельный netns и единственный lo. Только synthetic
198.51.100.0/24 и 2001:db8::/32, локальный DNS и TCP marker listeners; внешняя сеть,
реальная подписка и системный DNS не используются.

Официальный бинарник проверяется по прежнему SHA256. Adapter manifest связывается
с фактическим бинарником и текущим каноническим patch. Output directory должен
быть новым, чтобы старый успешный result.json не пережил новый неудачный запуск.
Каждый сценарий проходит native config-check; процесс остаётся жив после запросов
и затем останавливается/reap. Два SOCKS запроса проверяют cache-hit и повторную
ошибку; успех требует точного marker, отказ — отсутствия обращений к обоим marker
listeners, а не только SOCKS error.

Пройдены 92 сценария, 184 попытки SOCKS/TCP. Для явного transport и DNS rules, с
cache on/off: оба valid, оба SERVFAIL, ошибка A/AAAA по отдельности, AAAA NXDOMAIN,
NODATA каждой семьи/обоих, IPv4Only/IPv6Only, старший блокирующий AAAA CIDR.
Официальный upstream, patched default и explicit false сохраняют контрольное
поведение; patched strict отказывает partial result. Both-valid cache on требует
ровно одного A и AAAA для двух подключений, cache off — двух каждого семейства.
Timeout как ошибка transport дополнительно покрыт Go unit-тестами с искусственным
DeadlineExceeded; отдельный native packet-drop timeout в эту матрицу не входит.

Пройдены Go tests ./dns ./route ./route/rule ./option, targeted strict race tests,
gofmt, 48 Python unittest и Ruff 0.17.4. Rust/Qt source и зависимости не менялись;
этот этап не повторял GUI/power/Android Rust acceptance из предыдущих проверок.
В CI добавлен отдельный `native_adapter`: закреплённый setup-go/Go 1.27.1,
сборка с manifest, Go unit/race и проверка неизменности зависимостей. Cache/tmp
в RUNNER_TEMP, команды удерживают flock. Shell проверен bash -n, job прошёл
независимое статическое ревью. Нативная namespace-матрица остаётся локальной;
добавление job не означает успешный запуск GitHub Actions при billing lock.
Локальные evidence: `local/validation/strict-dns-adapter/native-first/result.json`
и соседние config/log/observation файлы; build/race/unit logs сохраняются отдельно.

## Что остаётся до G-DNS

StrictComplete не возвращает TTL lease и не закрепляет решение до фактического
dial. Не исправляет stale/disable-expire semantics, CNAME/negative TTL, Fake-IP
выдачу/повтор/restart или system DNS ownership. Не выдаёт DNS_POLICY_UNRESOLVED
в продуктовый controller и не реализует split DNS приватного VPN. Не гарантирует
равнозначную защиту Xray. D-01…D-07 целиком остаются обязательными; compiler
продолжает отклонять смешанную DNS/Fake-IP политику.

Для Xray source analysis отдельно выявил потерю DNS error в routing.Context,
продолжение router/dispatcher после lookup failure, повторный DNS в freedom,
отдельный UDP resolver path и недостаточные TTL исходного LookupIP. Следующий
патч должен переносить error и проверенный набор без нового DNS до dial, сохраняя
Host/SNI; отрицательный TTL и backend limitations нельзя подменить константой.
