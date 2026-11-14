# Linux: принятый прирост d74 → 302f8bf

Срез accepted main `302f8bf1d618e054083430925d168d4f022d6748`.
Предыдущий [delta](2026-11-14-linux-accepted-delta.md) — PR176
`77131bfb9e6f64334f7d999ee4f9abb1f241cc9a`, parent accepted `d74e61884385cd20f659112182142b05cacb958e`.
Это новый слой evidence, старые incoming/OPEN остаются историческими, не удаляются.
Полный план: **3 ready / 21 partial / 2 deferred из 26**. Повышений полных строк нет.

[JSON criteria ledger](2026-11-14-linux-criteria-302f8bf.json) сохраняет 256 исходных
строк индекса (P01…P26, FR, G/D/A/T, performance P-01…P-08 и L-leaves), точные blobs
нормативных документов, ancestry каждого head/merge и source-bound ссылки reports.
P01 плана и performance P-01 — разные пространства ID. `promotion=false` означает
отсутствие нового полного повышения; исторический текст строки не является текущим
incoming-статусом. Полный исходный [индекс](2026-11-11-linux-acceptance-index.md)
остаётся знаменателем, включая storage/security/recovery и текстовые инварианты.
`docs/specs/index.md` отсутствует: entrypoint — [README](../specs/README.md).

## Точный принятый прирост

Git ancestry проверена к d74 и 302f; snapshot `gh pr view` прочитан однократно для
каждого перечисленного PR. CI ниже относится к exact head, не к будущему main.
GitHub reviews пусты: независимые CLEAN ниже — root queue handoff, а не GitHub approval.
Канонический источник review receipts:
`/home/myrs/projects/yonik/local/orchestration/linux-remaining-task-queue.md`.
Это локальный путь, не переносимый artifact; JSON фиксирует Git/CI отдельно.

| PR / exact head → merge (полные SHA в JSON) | Принятый scope и ограничение |
| --- | --- |
| #171 ca801bc → 3e64220, включает #168 | N2/N3 canonical one-shot authorization, authenticated old3, queue/FD custody. Полный issuer не создан. |
| #166 87a51f9 → 1422984 | A4 source/build/ELF/compliance controller; не final package. #179 2599fe7 → 9f02440 фиксирует rehearsal failure до build, не новый PASS. |
| #155 1cf6aba → 1e83e6e | FR07 manual→save→new GUI→Connect/Stop bounded witness; synthetic key сохранён. |
| #169 88c398a → 73778db; #173 dac33e7 → 14a2e52 | LocalSocks resource/alternating-core witnesses; не полный native P-05. Старый #173 66df cleanup-incoming устарел как текущий статус. |
| #174 fa39b23 → 465a4dd | Integrated P11 14 boundaries. Старый c51bcf3 incoming сменён принятым head; полный P11 не закрыт. |
| #175 8576f53 → 0a2c0a4 | Actual Rust owned ATTACH/private Plain consumer обоих cores; восемь controls. Не production admission. |
| #180 b3a2838 → 7e909c7 | Private N5a lifetime/revocation, exact authorized binding и ACK fence; не full factory. |
| #182 b0b3aaa → a29ea61 | Supported-profile inputs и UNKNOWN implementation/load. Принятие manifest не attestation. |
| #183 5004508 → 36cc6a9 | Disk rotation restart oracle/process isolation; не весь FR16. |
| #177 6d3b53a → 4088165 | Controlled P-07 concurrent core/checks/import/logs, Stop/Cancel, bounded queues и hidden GUI consumer. Exact 10 SUCCESS; GUI/hidden CLEAN + integration R2 60a9 CLEAN. Не физический Wayland/energy proof. |
| #184 ea81c30 → 0a5d820 | Private move-only VerifiedTransport, прежние credentials/peer/pidfd/starttime/ACK. Exact **15**, не 10 SUCCESS. Real-userns fixture ignored в CI после uid_map EPERM; mint по-прежнему отказывает до receive. |
| #185 727e418 → cce6c4e | Controlled P-03, mixed-policy/model/Loader-frame/backend gate, 3×100 offscreen/software; exact10 SUCCESS + integrated bf54 CLEAN + union/preset 576897 CLEAN. Не physical Plasma/GPU/network Apply/long uptime. |
| #186 2dbfc04 → 0fc3442 | Compile-only unadmitted mixed AND/cancel/resources/pinned codec proof; exact10 SUCCESS + R1 6c931bf0 CLEAN. Не integrated P04A, 1000-rule UI, runtime attribution или Apply. |
| #189 71cf169 → 302f8bf | Disk consumer и host boundary ниже; exact10 SUCCESS + consumer 10b CLEAN + host 346 CLEAN + binding 575 CLEAN. Не native authority. |

## FR16: критерии отдельно от безопасных кодов

Источники: [consumer](2026-11-14-fr16-consumer-acceptance.md),
[host](2026-11-14-fr16-host-disk-boundary.md),
[первичный desktop slice](2026-11-11-fr16-desktop-log-slice.md).
Новый disk/host scope принят, полный нормативный FR16 не повышается автоматически.

| Критерий | Что именно доказано / предел |
| --- | --- |
| Разделение App/Engine | Типизированные source, Engine ProcessExited виден в sample export, App исключён. Это не поток исходных сообщений стороннего ядра. |
| Фильтры | Actual read(source=Engine, code=ProcessExited)→LogPage.export; mutation отключения source filter RED. |
| Bounds/rotation/backpressure | Очередь128, 1003→128 enqueued/875 dropped, 5×4MiB, partial-tail recovery, I/O failure terminal без retry; live ring/page bounds из прежнего slice. |
| Export/privacy | Canary SECRET/ESC/NUL/CR подан через reject_native, bytes отброшены; закрытые поля export безопасны в проверенном пути. Это не redaction неизвестного raw config/log input. |
| Host drain/ACK | Actual Actor Stop и mutation ACK при stalled disk; closed snapshot/control.closed отсутствуют до drain; после release приходят. Same watch lease отзывает pending DesktopFetch; lease synthetic, не active native core. |
| Crash/recovery | Owned child SIGKILL после rename chain до create active, restart. Не power-loss и не kill внутри rename syscall. |
| data-and-contracts:93 | «Исходные сообщения стороннего ядра доступны как технические детали…» — **unproven этим evidence**. reject_native только увеличивает rejection counter, исходный текст не сохраняет. Нужен отдельный source-bound witness доступности технических деталей через нормативно выбранный путь. |

Последний пункт не вводит нового требования принимать произвольные raw logs или
универсально редактировать любую строку. Требование доступности native technical
messages и scope closed-code logger различаются. Закрытый export не доказывает
доступность исходного сообщения. Root должен сопоставить существующий technical
path либо отдельно назначить gap; production код здесь не менялся.
Исторический producer/source binding P11 сохранён; reviewed_source_updates #189
не переименовывает старый runtime в запуск нового backend.

## Pending и точные следующие gates

| Gate | Следующее действие и зависимость |
| --- | --- |
| N5 full issuer CLOSED | После отдельного completeness review связать exact deployed systemd/libseccomp/kernel/ABI policy с успешной загрузкой в authenticated pending invocation, затем owner→issuer→factory→owned ATTACH→actual health/revoke. Source semantics/properties/errno сами по себе не attestation. New16 runtime запрещён до отдельного разрешения; новых probes не запускать. |
| N5c #188 47997a | Private common-boundary candidate, review CLEAN не ancestry. Не входит в 302f; дождаться authoritative CI/review и отдельного принятия. Canonical installed runtime/service coverage OPEN. |
| N5 bytes #190 36ced8e | cfa8bc1/f6fb2ea сохранены; static signed implementation/emitter/compat candidate не входит в 302f. Отдельно review/CI и acceptance, actual filter load остаётся UNKNOWN. |
| Native/DNS/G-APP | После issuer — actual bound native TUN двух cores, DNS D01…07, app A01…08, WG/LAN/network transitions, route correctness/cleanup. Component N4 не заменяет full Linux matrix. |
| P-01 #178 d0aa37d | Report MEASUREMENTS_SAVED_PENDING_FINAL_REVIEW. Review final terminal/identity/source/metrics; не перезапускать frozen campaign ради ledger. |
| P-02 #187 51fd15a | Candidate/preflight и measurements остаются отдельно; закончить campaign/review exact execution-source и engine-only pair. Наличие short controls/CI не принимает длинный результат. |
| P-03/P-07 | Controlled scope принят; physical G-UI/G-POWER deferred NOT PASS. Общий FR18 не закрыт. |
| P04A/P04B | Собрать integrated import/compiler normal+10x/10,100,1000 app-rules provenance/cancel/resources witness, учесть variance (compile CV до22.74%). P04B actual routing ждёт N5; data artifact не admission. |
| P-05/P-06/P-08 | Native lifecycle/flow cost/long sleep-network-WG campaign после N5 и fixtures. P-06 требует одинаковых реальных маршрутов engine-only/full-app; logical recovery не deferred только из-за energy части. |
| Toolkit | Equivalent functional Qt/GTK4/Slint workload/source binding, не empty window; historical corrected bench/ui subtree binding не означает новое измерение. Physical решение отдельно. |
| A4 #181 70c148c | Data/keyring custody partial; actualBackup Busy, нет valid backup/profile/revision gate. Сначала typed cause/admission/copy diagnosis, затем final reviewed package/install/upgrade/rollback/uninstall. |
| P11/FR16/storage | Закрыть native technical-details mapping и остальные original P11 RU/EN/export/backup/recovery критерии на integrated source; частичная приёмка не full P11. |
| Финальный A5 | После A1…A4 и native scope сопоставить ВСЕ original rows и текстовые инварианты; не считать merged PR denominator. Android P03/P26/G-ANDROID и physical G-UI/G-POWER deferred, NOT PASS. |

Raw evidence остаётся вне Git: consumer/host `local/evidence/fr16-consumer/` и
`local/evidence/fr16-host/` producer worktree, P03 `local/validation/p03-integrated/`,
P04A paths из checked-in binding.json. Это внешние retained artifacts, доступность
в любом checkout не обещается. Нового runtime/build/probe/re-audit feature bodies
не выполнялось. Проверка этого delta: JSON, Git ancestry/blobs, local Markdown links,
сохранность всех 256 inventory rows, counts и diff; отдельный pure-Python checker рядом.
