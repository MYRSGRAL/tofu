# yonik — источники и проверенные факты

Проект [yonik](README.md) развивается с открытым исходным кодом (open source). Linux — первая платформа, Android — следующий этап.

Проверено 8 ноября 2026 года. Использованы первичные источники. Документация `latest` может описывать более новые возможности, чем выбранный стабильный выпуск; они не становятся частью compatibility manifest автоматически.

| ID | Источник | Что подтверждает |
| --- | --- | --- |
| S-01 | [Xray-core 26.10.31](https://github.com/XTLS/Xray-core/releases/tag/v26.10.31) | Последний стабильный релиз по /releases/latest на дату проверки |
| S-02 | [sing-box 1.14.4](https://github.com/SagerNet/sing-box/releases/tag/v1.14.4) | Последний стабильный релиз по /releases/latest на дату проверки |
| S-03 | [Xray TUN, закреплённая версия](https://github.com/XTLS/Xray-core/blob/v26.10.31/proxy/tun/README.md) | Собственный TUN, особенности системной сети и мобильного FD |
| S-04 | [sing-box TUN](https://sing-box.sagernet.org/configuration/inbound/tun/) | TUN, исключения, DNS и auto-route; учитывать пометки версий |
| S-05 | [Xray FakeDNS](https://xtls.github.io/en/config/fakedns.html) | Сопоставление fake-IP с доменом и риск устаревшего DNS-кэша |
| S-06 | [sing-box FakeIP server](https://sing-box.sagernet.org/configuration/dns/server/fakeip/) | Нативный FakeIP DNS-server |
| S-07 | [sing-box URLTest](https://sing-box.sagernet.org/configuration/outbound/urltest/) | Default https://www.gstatic.com/generate_204 |
| S-08 | [ru-direct](https://github.com/kyoresuas/ru-direct) | Кандидат наборов доменов/IP и форматов двух ядер; не доказательство совместимости нашего клиента |
| S-09 | [systemd-resolved и VPN](https://systemd.io/RESOLVED-VPNS/) | Раздельная маршрутизация DNS по интерфейсам/доменам |
| S-10 | [Android VPN guide](https://developer.android.com/develop/connectivity/vpn) | Один активный VpnService на профиль, lifecycle и foreground service |
| S-11 | [VpnService API](https://developer.android.com/reference/android/net/VpnService) | prepare/protect, управление сервисом и разрешением |
| S-12 | [VpnService.Builder API](https://developer.android.com/reference/android/net/VpnService.Builder) | Установление TUN и настройка маршрутов/DNS |
| S-13 | [Android API 29: выполнение файлов](https://developer.android.com/about/versions/10/behavior-changes-10#execute-permission) | Ограничения exec из writable app home |
| S-14 | [libbox 1.14.4 PlatformInterface](https://github.com/SagerNet/sing-box/blob/v1.14.4/experimental/libbox/platform.go) | Контракты TUN, сокетов и наблюдения за сетью |
| S-15 | [XTLS/libXray](https://github.com/XTLS/libXray) | Mobile-wrapper, нестабильный API и ограничение независимых Go runtime |
| S-16 | [sing-box WireGuard endpoint](https://sing-box.sagernet.org/configuration/endpoint/wireguard/) | Возможность встроенного WG в выбранном движке |
| S-17 | [Xray WireGuard outbound](https://xtls.github.io/en/config/outbounds/wireguard.html) | Возможность WG-outbound |
| S-18 | [UniFFI](https://mozilla.github.io/uniffi-rs/latest/) | Kotlin bindings и async-интерфейсы для Rust |
| S-19 | [Android: регулярные сетевые обновления](https://developer.android.com/develop/connectivity/minimize-effect-regular-updates) | Причины объединять фоновые обращения и учитывать радио |
| S-20 | [CXX-Qt](https://github.com/KDAB/cxx-qt) | Интеграция Rust и Qt/QML |
| S-21 | [Qt system tray](https://doc.qt.io/qt-6/qsystemtrayicon.html) | Desktop-трей и StatusNotifierItem Linux |
| S-22 | [Slint](https://github.com/slint-ui/slint), [Winit backend](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backend_winit/) | Rust/DSL, Wayland/X11 и лицензирование |
| S-23 | [Slint 1.17](https://slint.dev/blog/slint-1.17-released) | Появление встроенного system tray |
| S-24 | [Iced FAQ](https://book.iced.rs/faq) | Изменения 0.14 и оговорки текстового ввода |
| S-25 | [Tokio](https://docs.rs/tokio/latest/tokio/), [zbus](https://docs.rs/zbus/latest/zbus/) | Возможности runtime и D-Bus-адаптера |
| S-26 | [Xray routing](https://xtls.github.io/en/config/routing.html) | IPOnDemand, порядок правил, совпадение любого адреса, поведение при ошибке DNS |
| S-27 | [sing-box route actions](https://sing-box.sagernet.org/configuration/route/rule_action/) | Действие resolve и выбор resolver; не доказательство полной семантической эквивалентности Xray |
| S-28 | [sing-box route rules](https://sing-box.sagernet.org/configuration/route/rule/) | process_name/process_path и платформенные ограничения; работоспособность конкретного TUN подтверждает G-APP |
| S-29 | [Xray 26.10.31 router config](https://github.com/XTLS/Xray-core/blob/v26.10.31/infra/conf/router.go) | Наличие поля process и его передачи в routing rule у версии-кандидата; не доказательство полноты распознавания |
| S-30 | [Desktop Entry Specification](https://specifications.freedesktop.org/desktop-entry/latest/) | Метаданные Linux-приложений, Exec и D-Bus activation; desktop entry сам по себе не идентифицирует поток |
| S-31 | [Лицензия sing-box 1.14.4](https://github.com/SagerNet/sing-box/blob/v1.14.4/LICENSE) | GPLv3-or-later и отдельное уведомление об имени/ассоциации |
| S-32 | [Лицензия Xray-core 26.10.31](https://github.com/XTLS/Xray-core/blob/v26.10.31/LICENSE) | MPL 2.0; лицензия yonik не переименовывает upstream-лицензию |
| S-33 | [GPLv3: текст и сводка GitHub Choose a License](https://choosealicense.com/licenses/gpl-3.0/) | Условия открытой передачи исходников/изменений, notices и частного использования; не готовый аудит всех зависимостей |

## Граница уверенности

Подтверждены наличие upstream-механизмов и ограничения платформ. Не подтверждены производительность приложения, готовая совместимость Xray/sing-box-конфигов, безошибочный перенос native библиотек в предложенные процессы, работоспособность на пользовательской сети или конкретном телефоне.

Номера релизов в документах — кандидаты для первых прототипов. Версии native wrappers, feature-флаги, toolchain и хэши артефактов должны попасть в воспроизводимый manifest перед реализационными проверками. Документация содержит проектные решения, а не результаты этих будущих проверок.
