# Проверка руководств Linux RU/EN

Точная база: `db8226ef585b8251fe02ca3dbbe0f7ac759fc636`.
Изменены только новые руководства и ссылки README; код, packaging, CI,
зависимости, PLAN и общие контракты не менялись. Android отложен.

Проверены main.rs (реальные CLI options), backend.rs (engine pins, import/review,
preferences), preferences.rs, desktop_schedule.rs, Main.qml, engine/options.rs,
requirements/ui-backend/helper-service/helper-worker-runtime specs, PKGBUILD,
packaging/arch/README.md, install_desktop.py и check_arch_package.py.

Важное расхождение источников: ui-backend-contract.md ещё называет все preferences
временными, но текущий backend сохраняет engine/port/language и безопасный URL.
Query/credentials/fragment URL остаётся session-only, при его выборе durable URL
сбрасывается на default. Руководства следуют коду; контракт не изменялся.

## RU/EN parity checklist

| Пользовательский путь / ограничение | RU | EN |
| --- | --- | --- |
| Linux first, Android deferred; local SOCKS, system_tun=false | да | да |
| Сборка locked, slot/flock/external target/jobs=1, сохранение binary и cleanup | да | да |
| Проверенное происхождение, точные engine pins, cleanbuild78/eec7 не latest | да | да |
| Menu install, data-dir, Secret Service failure без замены ключа | да | да |
| FR-06/07 URL, review, JSON/partial, ошибки и последний snapshot | да | да |
| Offline stdin analysis, отсутствие GUI file/manual import, лимиты | да | да |
| ManualProxy domain требует explicit trusted DNS, GUI wiring отсутствует | да | да |
| Background unknown fail closed, ручное обновление | да | да |
| Durable preferences и session-only query URL | да | да |
| Editor draft, Apply disabled | да | да |
| Endpoint diagnostics, Limited/network change, отсутствие global internet claim | да | да |
| Tray close/Quit, CleanupError, instance restore | да | да |
| #82 safeCopy pending, проверка секретов перед отправкой | да | да |
| Package pin отдельно от HEAD, validation limits, VM install/rollback/remove | да | да |
| Сохранение данных/ключа, schema rollback caveat, отсутствие broad workaround | да | да |
| P25 не закрыт: licenses/runtime composition/performance | да | да |

## Фактические проверки

- `python tools/install_desktop.py --help`: exit 0, подтверждены только реальные options.
- `python tools/check_arch_package.py --help`: exit 0.
- `bash -n packaging/arch/PKGBUILD`: exit 0.
- `python -m unittest discover -s tools/tests -p test_install_desktop.py`: 20/20 OK,
  изолированные temporary HOME/XDG fixtures, без установки в пользовательский HOME.
- `python -m unittest discover -s tools/tests -p test_arch_package.py`: 6/6 OK,
  synthetic archives, без установки и исполнения hooks.
- Относительные Markdown-ссылки README и руководств проверены на существование;
  парность fences и `bash -n` всех sh-блоков руководств — успешно.
- В обоих языках совпадают engine paths/hashes и shell-блоки; checklist выше
  проверен по содержанию вручную. `git diff --check` — успешно.

Cargo/GUI здесь не собирались и не запускались: изменение документационное.
Host install, keyring, helper/network effects, package install/uninstall/rollback
и реальные provider requests не выполнялись. Shell syntax не доказывает runtime
сборки или доступность удалённых артефактов. Heavy transient build отсутствует,
поэтому Cargo cleanup не требуется. Existing notices/runtime/performance gates
не перепроверялись и не объявляются пройденными.
