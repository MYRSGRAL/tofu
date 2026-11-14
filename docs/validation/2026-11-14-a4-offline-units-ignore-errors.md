# A4: единственный units-only read с ignore-errors

Продолжение PR181 после `1a4e9c4d27ccc741d9bf369bd20bd2efd74b7b00`.
Root decision `a4-offline-units-ignore-errors-root.json`, SHA256
`b4dd33c6b41f71e6ec25418480ddae23448742c15652530f662e9029b7494f10`,
разрешил ровно одно чтение существующим pinned Btrfs decoder. Оно выполнено;
новых попыток, boot, decoder, mounts, repair или package transactions не было.
Source142/package MTREE4 и прежние records/userdata не менялись.

## Actual результат

`btrfs restore -i -S --path-regex <units-only> <own-root.raw> <fresh-output>`
вернул **exit0**, но все 14 извлечённых vendor unit-файлов имеют ошибку
`ERROR: zstd frame incomplete` и `ERROR: copying data … failed`; каждый файл
нулевой. Для `cloud-init.target` дополнительно записан directory search error.
Все 14 помечены **RED_TOOL_ERROR**, ни один не используется как definition.
Это actual подтверждение: exit0 с ignore-errors не означает integrity/read PASS.

Per-file исходные пути, размеры, ошибки и результаты сохранены в
[accounting](../evidence/a4-offline-ignore-errors-142/run1/per-file-and-graph.json),
PID/exit/time и sanitized stderr — в
[read-result](../evidence/a4-offline-ignore-errors-142/run1/read-result.json).

Недоступны определения всех девяти required узлов:

- `sshd.service`;
- `cloud-init-network.service`, `cloud-init-main.service`;
- `cloud-config.service`, `cloud-final.service`;
- `network.target`, `network-online.target`;
- `time-sync.target`, `systemd-time-wait-sync.service`.

Также failed: `cloud-config.target`, `cloud-init-local.service`,
`cloud-init.target`, `sshdgenkeys.service`, `systemd-timesyncd.service`.
Нулевые файлы не доказывают отсутствие этих units в guest.

Actual partial graph содержит **0 пригодных определений, 0 рёбер**;
полнота **UNKNOWN**. Отсутствие transitive списка при пустом графе не означает
проверенную dependency closure. Отдельно прочитаны пять symlink masks
`/etc/systemd/system/{yonik-helper.service,yonik-worker.service,yonik-worker.socket,yonik-worker-check.service,yonik-worker-check.socket}`
→ `/dev/null`. Эти masks подтверждают только on-disk ссылки, не job state/PID1.

Причинный invariant bootstrap не доказан: actual After/Requires/Wants и
соответствующие job states отсутствуют. Предыдущие sanitized journal/serial
наблюдения остаются отдельным evidence; они не превращают NTP/time-sync в
установленную причину. Никакого bootstrap fix или `--no-block` из результата
не выводится. Live package daemon-reload gate остаётся неподтверждённым,
**fullA4=false, finalLinux=false, production factory CLOSED**.

## Inputs, guard и cleanup

[Summary](../evidence/a4-offline-ignore-errors-142/run1/summary.json) содержит
точные SHA source142/package/base/overlay/decoder. Существующие pinned
`qemu-img` и `btrfs` сверены с прошлым inventory. Под directory flock с первого
использования выполнены metadata/GPT bounded checks и sparse own root-partition
copy; источник открывался readonly. Selector ограничен units/drop-ins, без
whole guest, private keys, userdata или journal extraction. Сохранён одноразовый
O_EXCL authority marker вне TTL; разрешённый restore вызван ровно один раз.

Input SHA и dev/ino/size/mtime проверены отдельно от exit code: base и overlay
совпадают до/после. Reader reaped, references=[], own transient root удалён;
[cleanup](../evidence/a4-offline-ignore-errors-142/run1/cleanup.json).
Никакие старые raw inputs или user state не удалены.

Private evidence вне TTL:
`local/validation/a4-offline-bootstrap/6affd300/` — 86 500 regular-file bytes,
каталоги0700/files0600, symlinks не chmod-followed. Сохранены exact executed
sources, root decision, commands/PIDs/exits, все per-path ошибки, failed files
и raw manifest. [Receipt](../evidence/a4-offline-ignore-errors-142/run1/private-receipt.json)
привязывает manifest SHA; executed reader совпадает с новым tool source.
Public evidence не содержит bootcmd nonce, private payload или command-line
полей journal.

## Проверки и bounded остаток

34 A4 tests и Ruff PASS. Новый набор проверяет false exit0 с failed required
unit, сохранение рёбер OTHER valid unit при missing transitive node, zero и
truncated units, unattributed errors и независимый source SHA check с mutation
RED. RED до появления модуля и GREEN сохранены в evidence.

Текущий точный blocker — pinned existing decoder не прочитал compressed bytes
14 конкретных unit paths даже с ignore-errors. Это не доказательство причины
zstd ошибки или неисправности guest time-sync. Для causal investigation нужен
отдельно reviewed readonly способ получить actual required/transitive unit
bytes и job/journal correlation. Новый decoder/fallback/boot сейчас не выбран
и не запущен; разрешённый единичный read исчерпан.
