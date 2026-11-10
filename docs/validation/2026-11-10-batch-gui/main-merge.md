# PR76: combined merge origin/main

Первый родитель: `cb885615737277f9d01e42e2196bc28b624c0687`.
Второй родитель / проверенный origin/main:
`db8226ef585b8251fe02ca3dbbe0f7ac759fc636`.

Выполнен обычный merge, без rebase/reset/amend. Единственный текстовый конфликт
находился в объявлениях модулей repository.rs: сохранены оба `mod backup` и
`mod batch_targets`. Backend объединился автоматически. Общие source API,
backup и resolve_profile не менялись вручную.

Проверено соответствие fetch.rs, system_client.rs, backup.rs и application.cpp
версии origin/main. batch_diagnostics.rs и repository/batch_targets.rs совпадают
с принятым cb88561. Относительно origin/main backend содержит только batch paths;
Proxy retirement в priority path остаётся до обработки Stop/Shutdown и await.
Новых корректировок Proxy leases/refresh/scheduler semantics не понадобилось.

## Meaningful combined regression

- Desktop unit: 22 PASS, включая новый Proxy session reconnect на тот же endpoint,
  отзыв поколения до сетевого I/O и Stop во время SOCKS-handshake.
- Backend: 34 PASS, 1 upstream native Proxy gate оставлен ignored намеренно.
- Rust subscription_proxy: 12 PASS (DNS pinning, запрет fallback, redirects,
  отмена и замена endpoint), собственные loopback fixtures.
- Repository bulk: 4 PASS на merged backup70, включая CAS/AEAD/equivalence/cancel.
- ActualHost 10k: PASS. Все 10 000 уникальных profile IDs совпали со всеми
  result IDs; три успешные TCP-пробы на профиль, все страницы и DTO проверены.
  Ровно 30 000 TCP EOF, по 1 875 на каждый из 16 собственных loopback listeners.
  Check_batch и чтение всех результатов: 6 016 мс debug.
- Во время подготовки 10k: Cancel 144 мс, Stop 151 мс, Shutdown 149 мс; после
  ACK счётчик остаётся 30 000, новые TCP не запускаются.
- RTNETLINK epoch: PASS. В отдельном user/net namespace только lo; выполнены
  реальные route add TEST-NET, host inode 4026531833, test inode 4026533529.
  Monitor ID `54fd505830b19fc2787750617a7b59d5`, epochs `1→2→3`.
  Completed и in-flight страницы инвалидируются; при второй смене completed=32,
  accepted=434 (384 от первой серии). Старые строки не восстанавливаются.
  Отмена активной серии — 5 мс. Оба gates вместе: 44,14 с.

Native/VM/tray/installed-helper gates других PR не повторялись. Новый Qt binary
для этого merge не строился: Qt user path exact cb88561 остаётся прежним evidence,
а новая combined-проверка использует настоящий DesktopHost и JSON GUI protocol.
Полного GUI load 10k здесь не заявляется.

Combined Rust check: desktop clippy --all-targets --no-deps -- -D warnings;
fmt --check desktop+linux. Итоги и бинарные SHA256 в локальном evidence.

`git diff --check` относительно родителя ветки видит upstream CRLF/пустые строки
в license inventory; оригинальные лицензионные тексты не переписывались.
Проверка собственного diff относительно origin/main чистая.

## Evidence / review

Логи: `local/evidence/batch-main-merge/` вне TTL — backend.log, proxy.log,
bulk.log, gates.log, clippy.log, fmt.log, provenance.json и cleanup.log.
Точный merge SHA и SHA256 acceptance binary связаны в provenance после commit;
бинарник сохранён вне build-каталога. Сборка использует собственный transient
root flock, with-rust-slot, CARGO_BUILD_JOBS=1 и глобальный лимит 2 slots.

Full combined review назначает root. Merge PR, watch и делегирование не выполнялись.
