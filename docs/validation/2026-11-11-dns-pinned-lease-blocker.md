# DNS pinned cores: положительные exclusions и границы native lifecycle

База: `5452555100ee10569ff5f8af9cedd7df01968ba9`. Полный G-DNS **OPEN**.
Это native fixture/report slice. Production compiler, DnsPolicy, shared API,
helper/session/worker/private ABI и host network settings не менялись.
Compiler по-прежнему возвращает `Unsupported(DnsContract)` для exclusions и
`require_mapping_lease`; успешные native controls не объявляются продуктовой интеграцией.

## Exclusions: A и AAAA

`tools/dns_pinned_controls.py` проверяет SHA-256 обоих бинарников до запуска:
Xray `cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1`,
sing-adapter `4c31db07cb394479b58cc9aace8c8bef5da05a73db6dacca4a019aeb8b14e16d`.
Новые pins, Go и зависимости не добавлялись.

Все четыре сочетания core × A/AAAA прошли:

- Exact `exact.excluded.test`, suffix `suffix.excluded.test` и его дочернее имя
  получают реальные `192.0.2.10` / `2001:db8::10`.
- Каждый запрос подтверждён **фактической** записью собственного UDP upstream:
  имя, qtype, адрес, TTL. Одного ожидаемого JSON или адреса ответа недостаточно.
- Дочернее имя exact-исключения, похожий суффикс и обычное имя получают fake-IP
  разрешённого семейства; этих запросов нет в upstream transcript.
- Четыре native мутанта с удалёнными exclusion rules отклоняются: вместо real
  приходит fake. Timeout/parse failure не принимается за успешный ответ.

Для Xray real server имеет exact/suffix matches, а fake server — явный
`regexp:.*`. Первая проба без этого fallback match отвергнута oracle: Xray
выбирал первый real DNS и для имён вне исключений. Неуспешное evidence сохранено.
Это уточнение fixture-конфигурации, а не изменение compiler.

## Reset используемого интерфейса

Отдельный `--reset-only` запускает оба cores в собственном user/net/mount/pid
namespace. Только его `lo` переводится down/up; down подтверждается `ip -j link`.
До reset real DNS отвечает `.10`, после reset тот же upstream отвечает `.11`.
Два фактических upstream records подтверждают свежий real lookup; новое
fake-соединение достигает M01. Старый fake-IP продолжает достигать M00 в том же
PID/config/netns. Это отрицательный native witness отсутствия автоматического
сброса mapping от link event, а не проверка production NetworkHost.

У fixture нет production Stamp/custody/context: `production_stamp=null`,
`production_epoch_fence=NOT_TESTED`. Совпадение fake-IP после restart само по себе
**не является нарушением epoch fence**. Старый `fakeip_epoch_gap` дополнительно
показывает пустую новую сессию и возможное повторное назначение адреса; это только
native allocation behavior. Для доказательства production нарушения потребовались
бы старый Stamp и попытка его допуска в текущем доверенном контексте.

## Реальное истечение TTL

Полный запуск использует wire TTL, monotonic time и действующий native процесс.
Xray TTL=1 с проверен после 1,1 с с единственным allocation в pool `/15`,
poolSize=65535: старый fake-IP остаётся M00 без исчерпания pool.
Длинный sing-adapter case использует его фактический TTL=600 с, pool `/15` и
два allocations. Wrapper ограничен 750 с, логирует остаток каждые 30 с,
проверяет живость процесса, завершает process group при timeout/отмене.
После ожидания проверяются старый mapping и свежий DNS lookup/соединение.
Точная длительность и результат записаны в приложенном evidence.

В длинном case есть также событие dummy-link down; это отдельный слабый control,
который не подменяет описанный выше reset используемого `lo`.
Никакие fixture clocks или сокращённый sing TTL не выдаются за production proof.
Сохранение mapping после TTL не означает автоматически устаревший real RRset:
marker-назначение здесь статично. Это проверка lifetime native mapping.

## Узкий blocker и проверяемая развязка

`tools/dns_pinned_lease_blocker.py` сохраняет отдельный контрпример малого
валидного `/30` pool: ещё действующий адрес переиспользуется другим именем
(Xray TTL=1 с, sing TTL=600 с). Его нельзя переносить на срок исчерпания default
pool, а также использовать как причину не проверять exclusions или reset.

Минимальная развязка для строгого mapping lease требует:

1. Native mapping admission проверяет deadline и доверенный текущий контекст
   session/network/policy; просроченное или неизвестное значение не допускается
   к dial до свежего разрешения.
2. При смене контекста ingress закрывается до инвалидации mapping; очистка только
   DNS answer cache или restart с повторным использованием IP не доказывает fence.
3. Адрес не назначается другому имени, пока старый consumer может законно его
   предъявить; bounded pool exhaustion должен давать отказ, а не alias.

Проверяемый acceptance: положительное свежее соединение; тот же старый Stamp
после TTL/смены контекста не достигает marker; новый Stamp допускается; повторный
allocation не обходит проверку. Этих production hooks fixture не предоставляет.
Новый Rust DNS/packet engine и неавторизованный shared contract не добавлялись.

## Проверки и evidence

14 Python unit tests (включая cleanup wrapper), Ruff format/check и MSRV 1.85.1 fmt check.
RED старого marker oracle воспроизводит потерю фоновой ошибки; новый oracle
падает при ошибке/зависшем cleanup. Дополнительные тесты запрещают ложный PASS
при timeout, мёртвом процессе, неограниченном TTL и отсутствии upstream witness.
Rust source/Cargo.lock неизменны; полный clippy не повторялся.
Новый cache не добавлен, 1x/10x performance и полный D-01…D-07 не заявляются.

Санитизированное actual evidence и hashes лежат рядом с отчётом в
`dns-pinned-controls-evidence.json`. Полные configs/logs и исходники запуска вне TTL:

- `/home/myrs/projects/yonik/local/validation/dns-pinned-controls-v2-5452555/`
- `/home/myrs/projects/yonik/local/validation/dns-pinned-controls-final-exclusions/`
- `/home/myrs/projects/yonik/local/validation/dns-pinned-controls-final-reset/`
- `/home/myrs/projects/yonik/local/validation/dns-pinned-controls-checks/`
- `/home/myrs/projects/yonik/local/validation/dns-pinned-lease-5452555/`

Бинарники остаются в постоянных `local/engines/xray-26.10.31/xray` и
`local/artifacts/sing-box-external-tun-versioned-v1.14.4/sing-box` основного repo.
Функции долгого запуска сохранены snapshot; их AST сверяется с публикуемым
исходником, чтобы форматирование и добавление reset-only не требовали повторного
600-секундного ожидания. Исходная неуспешная проба не перезаписана.
Собственный Cargo transient предыдущего шага очищен (`cargo clean`: 0 files),
новая Rust-сборка не запускалась. После завершения проверяются native cleanup
и отсутствие собственных fixture-процессов. Все namespace уничтожаются wrapper.

Команды воспроизведения (бинарники должны соответствовать SHA выше):

```sh
python tools/dns_pinned_controls.py "$XRAY" "$SING_ADAPTER" --output "$EVIDENCE/full"
python tools/dns_pinned_controls.py "$XRAY" "$SING_ADAPTER" --exclusions-only --output "$EVIDENCE/exclusions"
python tools/dns_pinned_controls.py "$XRAY" "$SING_ADAPTER" --reset-only --output "$EVIDENCE/reset"
python -m unittest tools/tests/test_dns_pinned_controls.py tools/tests/test_dns_pinned_lease_blocker.py tools/tests/test_dns_fixture.py tools/tests/test_engine_smoke.py
```

Каждый output должен быть новым каталогом: native config создаётся через O_EXCL.
Переменные путей выше задаёт запускающий; ни root, ни host interface changes не нужны.
Запуск полного case занимает чуть больше 600 секунд. Реальные timeout,
thread failure и несовпадение witness завершают его ненулевым кодом.
`None` старой connect-пробы в малом-pool evidence не используется как доказательство
protocol refusal: он может означать timeout, которому PASS не присваивается.

Actual sing-adapter результат: wire TTL=600 с, monotonic elapsed=600.200273 с; старый mapping → M00, свежий lookup/соединение → M00. Native run завершён exit 0; оба ядра reaped, namespace уничтожен. Это сохранение mapping после TTL, не доказательство устаревшего real RRset или нарушения production epoch fence.
