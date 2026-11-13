# P11: закрытые причины scheduler Fetch

База: accepted main `79f589a23d900af4ccf184413438b21985b877a0`.
Ветка: `t3/p11-scheduler-fetch-causes`.

ScheduleFailure::service сохраняет тип FetchError до legacy mapping. Пять
специальных вариантов получают отдельные причины HwidRequired/HwidLimit/
HwidRejected/HwidRedirect/UnsupportedProxy; все остальные — единственную
SchedulerServiceFetchFailed. Коды и message_key взяты из прежнего каталога.
Stage SchedulerService, slots error/source, обязательная generation, params {}.
Import, native, unsafe, public API, зависимости, pins и локализации не изменены.

Targeted RED: настоящий SubscriptionService::refresh через loopback HTTP 503
возвращает FetchError::HttpStatus; проверка падает на отсутствующем companion.
GREEN: тот же producer проходит service mapping → snapshot → production strict
V2. Все 21 enum проверены на точный legacy key/code/reason и params. Stale/null
scheduler generation и чужой slot отвергаются. Матрица Rust сверяется с JSON;
независимый Python strict V2 принимает все 21 сохранённых экспорта.

Evidence вне TTL: `/home/myrs/projects/yonik/local/validation/p11-fetch/`:
логи RED/GREEN, source snapshot/diff/hashes, бинарник library tests, snapshots,
exports и SHA256SUMS. Прерванный compile и первоначальный compile failure fixture
сохранены отдельно; RED подтверждается только red-targeted.log.

GUI fault injection каждого enum, TLS/errno и полный P11 acceptance не заявляются.
Cross-UID scope не выполнялся; прежняя ветка и published ref сохранены.

Проверки: desktop library 95 PASS / 1 ignored; Python V1/V2 15 PASS;
21 actual exports Python strict V2 PASS; targeted rustfmt и diff-check PASS.
Clippy MSRV 1.85.1 --all-targets -- -D warnings: PASS.
