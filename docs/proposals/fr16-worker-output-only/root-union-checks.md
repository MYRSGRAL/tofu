# PR211: проверка ordinary union с root213

Source merge: `11557271eb57ede0e9aa527278a2271fd6c1de6c`.
Parents: frozen FR16 `4d98b6ef2e05b822085b55b07d4ca7028adc3394` и root
`0bb3e86e22d7bf6709c90453e9f8619b60a59ee4`. Root213 ещё не accepted.
R1 `1c7a98b4` scoped CLEAN относится только к 4d98, не к новому union.

Проверено без новой Rust build/test и без runtime:

- Ordinary merge без конфликтов. Из 535 изменённых относительно общего base
  путей 534 побайтно совпали с соответствующим parent. Единственный combined
  файл — backend; карта FR16 blobs и hash полного списка в `root-union.json`.
- `schedule_replacement` побайтно совпадает с root213; INFO policy function
  побайтно совпадает с 4d98. Все FR16 helper/client/worker/carrier files сохранены.
- `cargo +1.85.1 fmt --all -- --check` и `git diff --check` PASS.
- `python -m unittest tools.tests.test_p11_acceptance_closure tools.tests.test_p11_factory_coverage -v`:
  5 PASS. `python tools/check_translations.py`: 388 inline RU/EN пар, 108 ключей,
  17 QML файлов, PASS. Python source не менялся; ruff в текущем PATH отсутствует.
- `python -m unittest tools.tests.test_p11_integrated_candidate -v`:
  4 PASS / 1 ожидаемый freshness FAIL / 1 local-evidence ERROR.
  Freshness FAIL вызван новым backend blob: старый reviewed binding сохранён,
  новый записан в `pending_source_updates`, review пустой и pending=true.
  Gate/его тесты не менялись; это **не GREEN** до независимого root review.
  ERROR — отсутствует historical
  `local/validation/desktop-reviewed-integration/source-binding.json`
  (и связанный producer log) в этом worktree; доступной копии по прежнему
  canonical local path также не найдено. Evidence не подделывалось и не заменялось.

Historical `source`, `source_blobs`, `producer_evidence`, прежние reviewed updates
и их history не переписаны. Metadata pending entry не участвует в checked_blobs
и не обходит freshness gate. Для GREEN нужны root review объединённого backend
и доступ к прежним exact-hash producer artifacts; старый runtime не переносится
на новый source. PLAN promotion из root213 остаётся условным на его CI/merge.

Логи этого шага и полная карта parent blobs отдельно:
`/home/myrs/projects/yonik/local/orchestration/evidence/fr16-pr211-root-union/`.
Historical ELF/source/log 4d98 в `fr16-output-6476` не изменялись.
`new_runtime_pass=false`: VM/worker/engine/host install/crossUID не выполнялись.
Следующий этап — только предложение `worker-acceptance-next.md`, не USER grant.
