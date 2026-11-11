# Диагностика UnsafePath без изменения политики

Дополнение к PR95 после сообщения root: на независимых PR есть CI failures
`UnsafePath` при open subscriptions.sqlite и 10-секундные Harness timeouts,
в том числе в integration target, включающем backend.rs как support.
Сам по себе timeout не доказывает медленный runner и не объясняет UnsafePath.

На исходном head PR95 локальный workspace прошёл: 776 passed, 39 ignored.
CI PR95 job [130234832545](https://github.com/MYRSGRAL/yonik/actions/runs/42932616987/job/130234832545)
прошёл fmt/clippy и упал в `preferences_reopen_applies_engine_port_probe_and_language`:
`tests/backend.rs:105`, `snapshot до deadline: Timeout`. Причина не установлена.
Код этого теста идентичен базе; desktop не входит в ownership этого изменения.
Root отдельно исследует Harness namespace и port/fixture collisions.

В repository добавлена только внутренняя error-path диагностика:

- Все условия secure_path и UnsafePath outcomes сохранены, включая mode/uid/nlink,
  canonicalize, NOFOLLOW и create_new. Deadline/retries не менялись.
- В test/test-support builds для синтетического root непосредственно в temp_dir
  (`yonik-*`/`tofu-*`, ASCII имя) ошибка записывает JSON event
  `yonik_repository_unsafe_path`. В обычной production build reporter отсутствует.
- `branch` обозначает исходную ветку отказа: parent/database/sidecar policy,
  create/stat/canonicalize. `observed` сохраняет Metadata той проверки, которая
  приняла решение; `nodes` отдельно делает свежий lstat parent/DB/WAL/SHM/journal.
  Это позволяет увидеть исчезновение/замену между проверкой и диагностикой.
- Поля mode, uid/gid, nlink, dev/ino, len, ctime, file type и errno не содержат
  body, ключа, URL или decrypted data. Произвольные пользовательские пути скрыты;
  `YONIK_REPOSITORY_TEST_DIAGNOSTICS=1` разрешает для них только redacted metadata.
- `test_ref` берётся из validated hex `YONIK_REPOSITORY_TEST_REF`, иначе GITHUB_SHA.
  Полный fixture path записывается только для указанных синтетических temp roots.

Повторное локальное выполнение actual duplicate target
`cargo +1.85.1 test --locked --offline -p yonik-desktop --test batch_diagnostics`
с reporter: **35 passed, 1 ignored**, включая support::scheduled_delete...
и support::preferences_reopen... . UnsafePath или timeout не воспроизведены.
Это не объяснение причины CI и не доказательство отсутствия race.

Evidence сохранён вне transient build в
`/home/myrs/projects/yonik/local/validation/actual-cancel/`:
`diagnostic-batch.log`, `ci-rust-failure.log` и последующие scoped checks.
Production policy fix не сделан без observed filesystem fact.

Scoped проверки с reporter, Rust 1.85.1: существующий
`symlinks_permissions_and_limits` прошёл и выдал два structured events
(symlink mode `120777`, regular DB mode `100644`, uid=1000, nlink=1,
private parent `040700`). JSON отдельно проверен в `diagnostic-security.json`.
Cancellation regressions 22/22 и workspace all-targets Clippy прошли
(`diagnostic-cancel.log`, `diagnostic-clippy.log`). Эти controls подтверждают
работу reporter и прежние отказы policy, а не воспроизводят CI UnsafePath.
