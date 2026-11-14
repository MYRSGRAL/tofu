# A4 candidate: изолированные package file/DB transactions

Dependent leaf от frozen PR179 `2599fe70ca93ebd14baaa8932e9de69fc5d66a94`.
PR179/175/171/147 refs и records не изменены. Дескриптор сохраняет
`final_linux_acceptance=false`; полный A4 и native Linux seal **не закрыты**.

## Проверенный результат

Использованы исходные архивы без перепаковки и сборок:

- old accepted #119: `0.1.0.r20261110-2`, SHA256
  `e53cb758ca1b9a2167a80c90ca50b1ab5eb4f6b67c5c8ec01c2bf0ccb856e8b0`;
- NONFINAL `linux-controller-rehearsal-14229843-msrv85-3`: `0.1.0.r20261111-1`,
  SHA256 `229a71a54cf2d0e6e6b83396476acb3ea54138ff0b988b93110ce6b7a3f5c740`.

`vercmp old new` = -1. Install old → upgrade candidate → rollback old → uninstall
реально выполнены pacman с exit 0 в own-root user/mount namespace, root UID
отображён только на собственный host UID. Зависимости не установлены (`-dd`).
Каждый root/dbpath/log/cache/gpgdir/hookdir/config направлен внутрь собственного
transient task. Bind mounts, host pacman transactions, systemctl, service
activation, GUI, foreign UID, VM, Rust/Go builds и Docker не применялись.
Архивы локальные accepted SHA-pinned, package signature отсутствует/не проверяется
(`SigLevel=Never`): этот run не является signature acceptance.

На трёх установленных состояниях каждый regular usr member сверяется с архивом:
bytes SHA256, mode, host UID/GID (own UID/GID, namespace root). Снимки содержат
полный actual file layout; rollback возвращает old payload. Uninstall удаляет
GUI ELF. Synthetic data sentinel с mode 0600 и own UID сохранился на всех стадиях.
Это проверка synthetic file custody, не encrypted production userdata.

## Конкретные незакрытые gates

1. Candidate не имеет `.MTREE`. Pacman `-Qkk` возвращает exit 0 с текстом
   `yonik: no mtree file`; это **не** положительный candidate Qkk integrity gate.
   Прямой archive→installed hash/mode oracle прошёл независимо. Production/recipe
   не менялись. Предложение root: определить обязательность MTREE/size для candidate
   package spec и добавить генерацию metadata отдельным reviewed исправлением.
2. В минимальном disposable root отсутствуют установленные dependency packages и
   system-wide libalpm hooks; пакет не содержит собственного `.INSTALL`. Поэтому
   real system hooks, sysusers account и service manager effects не проверены.
   Требуется отдельно принятый same-UID rootfs с настоящими cached/signed dependency
   closure и hooks. PID1/cgroup-dependent hooks потребуют отдельного допустимого
   installed-system стенда; fake manager/executables не применялись. Ни exit 0
   pacman, ни namespace root не подтверждают эти gates.
3. Доступный #119 fixture привязан к guest `/home/builder`, runuser, nft/ip и VM;
   напрямую запускать его в разрешённом own same-UID scope нельзя. Его accepted
   old package и snapshot oracle переиспользованы, старый VM route не повторялся.
4. Реальный #147 Secret Service/source witness сохранён и проверен: 47 artifacts
   совпали с anchored final manifest. Он относится к source
   `cceede520842a17e4ef548807f63f64b2be19226`, candidate к
   `14229843f9a11bf98ec4f5992df00f3247102ea2`. Сохранённая VM/keyring уже очищена;
   её profile/item нельзя считать входом этого run. Нужна свежая own private-bus
   production GUI fixture именно installed candidate с actual dependencies.
   Profile/original/revision/item/data continuity на candidate **NOT RUN**.
   Witness не подменён новым fake Secret Service и не пересобирался.

## Evidence и проверки

`docs/evidence/a4-candidate-lifecycle/`: commands.jsonl хранит argv/PID/exit/output
транзакций; snapshots.json — полные file hashes/metadata; result.json — scope и
частичный PASS; pacman.log — actual transactions. Повтор после добавления GID
assertion также прошёл; commands.jsonl сохраняет оба запуска.
`inputs-preservation.json` проверяет 13/46/770 файлов трёх registry inventories
после запуска и refs 179/175/171/147. Повторный provenance gate PASS со scope
local-TCB-provenance-preflight, final=false (`provenance.log`).

Transient root удерживался под BSD flock до cleanup; все синхронные дочерние
процессы дождались завершения, дерево удалено. Последний cleanup.json фиксирует
отсутствие task. Единственные результаты сохранены вне TTL в этом leaf; бинарники
и source archive остались в прежнем fixed registry. Никакие прежние FAIL records
не переписывались. Harness-only Ruff и diff check выполнены; фактический route
служит интеграционной проверкой harness, product tests не запускались.

NEXT: root принимает частичный file/DB proof и решает MTREE finding; для закрытия
A4 нужны разрешённые real dependency/hook/GUI custody inputs выше. Final seal
остаётся false. PR119 и PR147 не объявляются final Linux acceptance.
