# P05: Linux actual refresh acceptance

База `b9e5071784065c1fcb4813608d195421a146f80c`; первый срез
`9fcccfaa5f2acf8dd80cea34a99d874dc4fa046f`. Продолжение добавляет закрытый
DesktopHost callback source_change и native fixture для обоих закреплённых ядер.
Production fetch/subscriptions/repository/SystemSession/Client не меняются.

Контракт: data-and-contracts §§1–3; validation-and-roadmap, FR-06; PLAN P05.
Пустой ответ не означает удаление: рабочий snapshot не заменяется автоматически.
Partial публикует accepted/rejected и требует подтверждения именно этого отчёта.
Endpoint/credentials меняют fingerprint, display name не является identity.

| Requirement | Existing proof | Новый actual gate |
| --- | --- | --- |
| Смена credentials источника, стабильный source ID | subscription_refresh_contract::source_token_rotation_preserves_profile_id_and_favorite | source_change callback с expected generation и matching ACK; actual Host manual/scheduled refresh после ротации через оба native SOCKS |
| Rename / новые серверные credentials | accepted repository identity baseline | Native Host: rename сохраняет profile ID, смена VLESS credential создаёт новый ID; source ID сохраняется |
| Full / empty / partial / corrupt | subscriptions partial/empty tests | Service и scheduler плюс Host manual/scheduled, оба ядра; до approve старый snapshot, partial counts, error сохраняет revision=1 |
| HTTP fail / 304 / size | subscription_refresh_contract wire errors; conditional 304 | Обе native матрицы после rotation: HTTP503, unsolicited 304 без старых validators, size header rejection; положительный conditional304 отдельно в scoped Host/Service tests |
| Redirect / origin boundary | fetch HWID/origin tests | Собственный HTTP upstream: same-origin и cross-origin redirect; query-token не переносится, cross destination проверяет отсутствие Authorization; policy локального исключения явная |
| TLS verification | FetchHost transport | Своя CA только через SSL_CERT_FILE дочернего процесса; trusted HTTPS проходит, другая собственная CA отвергается; TLS fallback отсутствует |
| Proxy refusal без DIRECT / DNS fallback | accepted PR96 policy/DNS path | Реальный SOCKS каждого native core → закрытый namespace endpoint; DIRECT connect fence защищает HTTP/TLS/cross/DNS upstream и refused port; два положительных controls; getaddrinfo=0 |
| Отмена после response до commit | accepted PR95 cooperative/CAS/commit boundary | Actual Service Pending/Review — точный барьер полного response+parse; cancel перед commit оставляет revision=1; Host SourceChange отменяет pending review и не публикует его |
| Stale repository CAS | accepted repository identity baseline / PR95 | Второй Service меняет generation после Pending, commit/finish_review отвергается; Host approve устаревшего report также не меняет snapshot |
| После commit admission истинный результат | accepted PR95 locked-write boundary | Native Service admitted: подтверждённый commit=2, последующий cancel не откатывает snapshot; новых искусственных admission hooks нет |
| Pre-edit cancellation / ACK | accepted PR95 и PR113 fences | Callback gate удерживает actual manual/scheduled HTTP, SourceChange ждёт закрытия socket и выдаёт ACK только после cancel; старый profile и snapshot сохраняются |
| Ограниченный 10x dataset | scoped P05 acceptance | 200→2000 записей, <256 KiB, импорт deadline 15 s, страницы по 100, полное множество уникальных IDs; size rejection. Это не RSS benchmark |
| Секреты / export | P11 отдельный writer | Native callback запрашивает existing logs_read/export; canaries отсутствуют в DTO/export, raw URL/body/credentials не выводятся; отдельная negative-export evidence |

## Проверки и границы

`p05_native.rs` запускается только ignored fixture `p05_native.py` в собственных
user/network/PID namespaces с private D-Bus и ephemeral repository key.
Каждый Host владеет настоящей LocalSession. Service/scheduler используют SOCKS
именно этой сессии с явным literal DNS-over-TCP upstream. Core peer конфиги и
CA/ключи — только в приватном временном каталоге; секреты не передаются argv.

Повторно используются read-only артефакты sing-box 1.14.4 и Xray 26.10.31 из
local/engines и committed engine_host/engine_compile/dns_fixture. Go source,
сборка core, pins/dependencies, host keyring, resolver, routes/nft не менялись.

Resolver interposer переиспользует PR96 source; дополнительный connect fence
действует только на test executable, не на core. Controls принудительно вызывают
lookup и fenced TCP connect, затем evidence счётчики обнуляются. Следовательно
нулевые счётчики матрицы проверяют работоспособный механизм наблюдения.

SourceChange RED на исходном backend: invalid_input. GREEN — callback schema,
CAS/ACK и удерживаемый fetch. Первый prototype native fixture отбросил watch
sender и получил Cancelled; исправлено владение fixture, production fetch не
менялся. Первоначальный scoped 10x тест неверно ожидал default page<=100
(default=200); итог использует явный limit. Эти падения не production defects.

Интеграция с PR113 выполняется ROOT: точный hunk и сохраняемые fences описаны
в [source-change-integration](2026-11-11-p05-source-change-integration.md).
Эта ветка не содержит merge/rebase PR113 и не заявляет проверку их общего head.
UI credential editor не добавляется: callback acceptance использует публичный
закрытый JSON API; stored secret URL не выдаётся.

Evidence/source/binary manifests сохраняются вне TTL:
`/home/myrs/projects/yonik/local/validation/p05-native`.
Старый scoped evidence: `/home/myrs/projects/yonik/local/validation/p05-refresh`.

Финальные счётчики и результаты проверок добавляются после завершения runs.
Полный P05 в PLAN не переключается: интеграция ROOT и review остаются отдельными.

## Итог фактических запусков

- sing-box 1.14.4 и Xray 26.10.31: каждый 30 Host + 32 Service/scheduler
  сочетания, все прошли (124 суммарно). Native tests: sing-box 134,01 s, Xray 77,65 s.
- Оба core: DNS positive control=1, DIRECT positive control=1;
  system getaddrinfo=0, hidden DIRECT connect=0 после сброса controls.
- Собственные upstream: 480 DNS-over-TCP queries, 232 HTTP target requests,
  8 cross-origin requests, 8 trusted TLS requests.
- 60 export/DTO страниц: canary/raw URI отсутствуют; отдельный
  `negative-exports.json`. Сырые exports и тела не сохраняются.
- Callback/scoped acceptance: 3 passed, 43 existing tests filtered.
  Отдельный backend suite: 42 passed, 1 native test ignored; его actual заменяет
  новая более широкая native matrix. Повтор accepted PR95/96 целиком не заявлен.
- Rust 1.85.1: locked workspace clippy all-targets `-D warnings`, fmt check,
  git diff check прошли. Ruff 0.16.6 check/format для новых Python fixtures прошли.
- Private D-Bus предупреждает об inotify недоступного /root каталога; свои bus,
  Host и core процессы завершаются штатно. Host bus/keyring не используются.

Результаты: `results.json`, `counts.json`, `negative-exports.json`,
`final-manifest.sha256`, `callback-final-manifest.sha256`, logs и бинарники
находятся в evidence вне TTL. После завершения процессов target очищается cargo
clean, собственный transient root удаляется; host caches не очищаются.

Runtime criteria данного среза закрыты actual Linux gates. Остаётся внешняя
интеграция с PR113 и review ROOT на общем head; она не является средовым blocker
для выполненных матриц и не подменяется утверждением о полной готовности P05.
