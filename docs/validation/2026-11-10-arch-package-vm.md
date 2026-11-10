# P24: реальный пакет в disposable Arch VM

Проверенная база/HEAD: `d695c1624a2a9aae920d9658fdcf8c848e94f1e3`.
Ветка: `t3/arch-package-vm-validation`. Production-код и рецепт не менялись.
Исходный checkout `/home/myrs/projects/yonik` использовался только для чтения evidence/engine artifacts.

Пакет `yonik 0.1.0.r20261110-1`, source
`e33fa1908dfb3deb5434506341674de0b3a827b4`, SHA-256
`2b8110af8d8eb335ff2897edde87f84474dd65ac87bfe9522b0d91f93dd5bb7e`.
Новые исправления main в этот пакет не входят. Текущий archive validator прошёл.

Официальный образ https://mirror.pkgbuild.com/images/latest/Arch-Linux-x86_64-cloudimg.qcow2:
SHA-256 `794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003`;
SHA256 metadata и detached PGP signature проверены заново в собственном GNUPGHOME,
primary key `1B9A16984A4E8CB448712D2AE0B78BF4326C6F8F`.
QEMU/KVM UEFI, 2 CPU / 2048 MiB, SSH только loopback. Arch обновлён через
официальные репозитории; после обновления выполнен poweroff/boot на Linux
`7.2.15-arch1-1`. Qt `6.12.0`, systemd `262.2`; полный pacman inventory в dbus.log.

| Gate | Результат и границы |
| --- | --- |
| Установка оригинального архива | PASS: pacman -U, pacman -Qkk: 32 files / 0 altered |
| Отсутствие autostart | PASS: helper inactive/static, процессов /usr/lib/yonik нет |
| Владельцы/права/sysusers | PASS: archive validator, Qkk, root:root 755 ELF / 644 integration; yonik-worker nologin |
| Units | PASS: systemd-analyze verify; явный start/stop helper; ExecStartPre внутри unit exit0 |
| D-Bus/polkit | PASS: busctl introspect активирует helper; Capabilities отвечает; pkaction зарегистрирован, tester pkcheck exit1 |
| Qt из /usr/bin, чистый HOME | PASS: 142 steps, 41 events, 3 TCP EOF, 7 HWID requests; offscreen/software, tester |
| Update/rollback | PASS для пакетного перехода -1 → -2 → -1; см. ниже |
| Сохранность данных/чужой сети | PASS: SHA sentinel DB/key совпадает, dummy foreign0 идентичен, nft inet foreign_p24 и rule pref12345 сохранены |
| Uninstall после остановки | PASS: pacman -R, ELF удалены, процессов нет; sysuser остаётся штатно |
| Engines | PASS только версии/hashes/root ownership; validation registry внутри helper НЕ доказана |
| Отказ без registry | НЕ ПРОВЕРЕН: запуск helper без registry допустим; это не проверка отказа session API |
| Clean VM build / generic x86-64 | НЕ ПРОВЕРЕН; выполнена установка существующего архива |
| System transport | НЕ ПРИНЯТ: Capabilities system_tun=false; известный actual PR49 gate FAIL не переопределяется |

Вторая версия создана только внутри VM: распаковка оригинала через bsdtar,
замена `.PKGINFO pkgver` на `0.1.0.r20261110-2`, добавление
`/usr/share/yonik-p24-revision` с текстом о test-only revision, удаление устаревшего
`.MTREE`, перепаковка `.PKGINFO .BUILDINFO usr` root:root через bsdtar/zstd -T2.
ELF не менялись. Это известная тестовая ревизия, а не независимая новая сборка;
`.BUILDINFO` унаследован от оригинала. SHA-256 второго пакета
`aa37539e88a05987617a1602e846299b126492fc5e638cda0d40b5b13398f303`.
После upgrade pacman показывает -2 и marker существует, после downgrade показывает
-1, marker исчезает, Qkk оригинала проходит. Reinstall не выдаётся за upgrade.

Engine artifacts сверены с route-fix/artifacts.json и прежним provenance:
Xray 26.10.31 `cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1`;
sing-box 1.14.4 external-TUN adapter
`4c31db07cb394479b58cc9aace8c8bef5da05a73db6dacca4a019aeb8b14e16d`.
Это существующий adapter artifact, не новый официальный upstream ELF.
Они установлены исключительно в VM, root:root 0500, manifest 0644/schema1.
Standalone --check-worker-fence вне unit вернул 1; внутри unit ExecStartPre успешен.
Отдельный вызов не используется как доказательство registry validation.

Ошибки стенда сохранены: после обновления до reboot отсутствовал dummy module;
загрузка подписи spirv-tools один раз завершилась timeout, один повтор успешен.
Первая introspection использовала неверный object path Network1 и pkcheck чужой
PID; исправленный прогон dbus.log использует Network и собственный PID tester.
service.sh диагностический, без set -e: его exit0 не означает успех каждой команды.
Дефект production по этим наблюдениям не заявлен.

Evidence вне TTL, gitignored:
`/home/myrs/.t3/worktrees/yonik/t3-75e9b8dc-0380c71554d101f3/local/validation/p24-vm/`.
Там scripts, install{,2,3}.log, service/lifecycle/dbus.log, image/signature metadata,
QEMU exit records и preserved.tar (второй пакет, sentinel hashes, smoke screenshots/trace,
registry; без ключей и дисков), SHA-256
`73efc69984c2abaa879b5718a9aee069f20042ea02a8999e493e36dd3def9ac3`.
Для воспроизведения последовательность: download+SHA/PGP → prepare/boot → transfer →
install (после Syu reboot перед сетевыми sentinels) → service → lifecycle → dbus → export/poweroff.
Scripts являются evidence конкретного прогона, не универсальным runner.

Очистка: обе загрузки VM завершены штатным poweroff, QEMU exit0; собственные
VM/SSH процессы проверены перед удалением transient. Сборок Cargo не было,
cargo clean неприменим. Lock корня transient удерживался с первого использования.
Удалён только собственный `/var/tmp/codex-myrs/transient/yonik-p24-vm`.
Host packages/helper/network/keyring не менялись, детей/Docker/deploy не было.

P24 остаётся частичным: clean build, registry/session acceptance и вся утверждённая
матрица не подтверждены этим ограниченным пакетным прогоном.
