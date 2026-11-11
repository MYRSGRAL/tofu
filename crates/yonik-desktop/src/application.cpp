#include "application.h"
#include "ui_smoke.h"
#include "yonik-desktop/src/bridge.cxxqt.h"

#include <QApplication>
#include <QClipboard>
#include <QCloseEvent>
#include <QCryptographicHash>
#include <QDir>
#include <QDBusConnection>
#include <QDBusServiceWatcher>
#include <QFileInfo>
#include <QFile>
#include <QIcon>
#include <QJsonDocument>
#include <QJsonArray>
#include <QJsonObject>
#include <QImage>
#include <QPainter>
#include <QPixmap>
#include <QMenu>
#include <QLocalServer>
#include <QLocalSocket>
#include <QLockFile>
#include <QPointer>
#include <QQmlApplicationEngine>
#include <QQuickWindow>
#include <QSystemTrayIcon>
#include <QStandardPaths>
#include <QTimer>
#include <QUrl>
#include <QWindow>
#include <array>
#include <memory>
#include <functional>
#include <unistd.h>

namespace yonik {
void copy_diagnostic_metadata(const QString& value)
{
    QGuiApplication::clipboard()->setText(value, QClipboard::Clipboard);
}

QIcon statusIcon(const QString& status)
{
    static const QIcon brand(QStringLiteral(":/icons/yonik.svg"));
    const bool connected = status == QStringLiteral("Connected");
    const bool failed = status == QStringLiteral("Error") || status == QStringLiteral("CleanupError");
    const bool limited = status == QStringLiteral("Limited");
    const bool disconnected = status == QStringLiteral("Disconnected");
    const std::size_t variant = connected ? 0 : failed ? 1 : limited ? 2 : disconnected ? 3 : 4;
    // Все вызовы идут из GUI thread; повторный snapshot не перерисовывает семь размеров.
    static std::array<QIcon, 5> cache;
    if (!cache[variant].isNull()) return cache[variant];
    QIcon icon;
    for (const int size : {16, 22, 24, 32, 48, 64, 128}) {
        QPixmap pixels(size, size);
        pixels.fill(Qt::transparent);
        QPainter painter(&pixels);
        painter.setRenderHint(QPainter::Antialiasing);
        painter.scale(size / 32.0, size / 32.0);
        painter.drawPixmap(QRect(0, 0, 32, 32), brand.pixmap(size, size));
        // Статус остаётся различимым, а основной знак сохраняет узнаваемость в трее.
        painter.translate(18, 18);
        painter.scale(14.0 / 32.0, 14.0 / 32.0);
        painter.setPen(Qt::NoPen);
        painter.setBrush(connected ? QColor("#376b51") : failed ? QColor("#a83835")
                         : limited ? QColor("#8f630b") : QColor("#58645d"));
        painter.drawRoundedRect(QRectF(2, 2, 28, 28), 6, 6);
        painter.setPen(QPen(Qt::white, 3, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        if (connected) {
            painter.drawLine(QPointF(8, 16), QPointF(14, 22));
            painter.drawLine(QPointF(14, 22), QPointF(24, 10));
        } else if (failed) {
            painter.drawLine(QPointF(10, 10), QPointF(22, 22));
            painter.drawLine(QPointF(10, 22), QPointF(22, 10));
        } else if (limited) {
            painter.drawLine(QPointF(16, 8), QPointF(16, 18));
            painter.drawPoint(QPointF(16, 24));
        } else if (status == QStringLiteral("Disconnected")) {
            painter.drawLine(QPointF(9, 16), QPointF(23, 16));
        } else {
            for (int x : {9, 16, 23}) painter.drawPoint(QPointF(x, 16));
        }
        painter.end();
        icon.addPixmap(pixels);
    }
    cache[variant] = icon;
    return icon;
}

void captureWindow(QWindow* window, const QString& path, std::function<void(bool)> completed)
{
    qWarning("yonik smoke: capturing own window after a frame");
    auto* quick = qobject_cast<QQuickWindow*>(window);
    if (!quick) {
        qWarning("yonik smoke: root is not QQuickWindow");
        completed(false);
        return;
    }
    const auto connection = std::make_shared<QMetaObject::Connection>();
    const auto captured = std::make_shared<bool>(false);
    *connection = QObject::connect(quick, &QQuickWindow::frameSwapped, quick,
        [quick, path, connection, captured, completed = std::move(completed)] {
            if (*captured) return;
            *captured = true;
            QObject::disconnect(*connection);
            const auto image = quick->grabWindow();
            const bool saved = !image.isNull() && image.save(path, "PNG");
            if (!saved) qWarning("yonik smoke: own-window capture failed");
            completed(saved);
        }, Qt::QueuedConnection);
    quick->update();
}

class CloseHandler final : public QObject {
public:
    CloseHandler(QWindow& window, DesktopBridge& bridge, const bool& cleanupConfirmed)
        : window_(window), bridge_(bridge), cleanupConfirmed_(cleanupConfirmed) {}

protected:
    bool eventFilter(QObject* object, QEvent* event) override
    {
        if (event->type() == QEvent::Quit && !cleanupConfirmed_) {
            event->ignore();
            bridge_.shutdown();
            return true;
        }
        if (object == &window_ && event->type() == QEvent::Close) {
            static_cast<QCloseEvent*>(event)->ignore();
            if (QSystemTrayIcon::isSystemTrayAvailable()) {
                window_.hide();
            } else {
                // Без трея окно остаётся единственным доступным управлением сессией.
                window_.show();
                window_.raise();
                window_.requestActivate();
            }
            return true;
        }
        return false;
    }

private:
    QWindow& window_;
    DesktopBridge& bridge_;
    const bool& cleanupConfirmed_;
};

int run_application(const QString& dataDir, bool smokeTest, const QString& screenshot,
                    const QString& script, const QString& trace)
{
    // Qt хранит argv до разрушения QApplication.
    int argc = 1;
    char name[] = "yonik-desktop";
    char* argv[] = {name, nullptr};
    // Независимая от desktop-темы палитра; явный выбор пользователя сохраняется.
    if (qEnvironmentVariableIsEmpty("QT_QUICK_CONTROLS_STYLE")) {
        qputenv("QT_QUICK_CONTROLS_STYLE", QByteArrayLiteral("Basic"));
    }
    QApplication application(argc, argv);
    application.setApplicationName(QStringLiteral("yonik"));
    application.setDesktopFileName(QStringLiteral("cloud.highpriority.yonik.local"));
    application.setWindowIcon(QIcon(QStringLiteral(":/icons/yonik.svg")));
    application.setOrganizationDomain(QStringLiteral("highpriority.cloud"));
    const auto runtime = QStandardPaths::writableLocation(QStandardPaths::RuntimeLocation);
    const QFileInfo runtimeInfo(runtime);
    if (runtime.isEmpty() || !runtimeInfo.isDir() || runtimeInfo.ownerId() != geteuid()
        || (runtimeInfo.permissions() & (QFile::ReadGroup | QFile::WriteGroup | QFile::ExeGroup
            | QFile::ReadOther | QFile::WriteOther | QFile::ExeOther))) {
        return 1;
    }
    const auto key = QString::fromLatin1(QCryptographicHash::hash(
        dataDir.toUtf8(), QCryptographicHash::Sha256).toHex());
    // Общий lock с прежними версиями защищает тот же каталог данных.
    const auto socketPath = QDir(runtime).filePath(QStringLiteral("tofu-") + key);
    QLockFile instanceLock(socketPath + QStringLiteral(".lock"));
    instanceLock.setStaleLockTime(0);
    if (!instanceLock.tryLock(0)) {
        const QFileInfo socketInfo(socketPath);
        if (!socketInfo.exists() || socketInfo.ownerId() != geteuid()) {
            return 1;
        }
        QLocalSocket connection;
        connection.connectToServer(socketPath);
        if (!connection.waitForConnected(1000)) {
            return 1;
        }
        connection.write("show\n", 5);
        if (!connection.waitForBytesWritten(1000) || !connection.waitForReadyRead(1000)) {
            return 1;
        }
        return connection.read(4) == QByteArrayLiteral("ok\n") ? 0 : 1;
    }
    // Lock принадлежит текущему процессу; удаляем только оставшийся сокет того же UID.
    const QFileInfo socketInfo(socketPath);
    if (socketInfo.exists()
        && (socketInfo.ownerId() != geteuid() || !QLocalServer::removeServer(socketPath))) {
        return 1;
    }
    QLocalServer server;
    server.setMaxPendingConnections(8);
    server.setSocketOptions(QLocalServer::UserAccessOption);
    if (!server.listen(socketPath)) {
        return 1;
    }
    QJsonArray steps;
    if (smokeTest && !script.isEmpty()) {
        QFile scriptFile(script);
        if (!scriptFile.open(QIODevice::ReadOnly) || scriptFile.size() > 65536) {
            return 2;
        }
        const auto document = QJsonDocument::fromJson(scriptFile.readAll());
        if (!document.isObject() || !document.object().value(QStringLiteral("steps")).isArray()) {
            return 2;
        }
        steps = document.object().value(QStringLiteral("steps")).toArray();
        if (steps.size() > 256) {
            return 2;
        }
    }
    QFile traceFile(trace);
    if (smokeTest && !trace.isEmpty() && !traceFile.open(QIODevice::WriteOnly | QIODevice::Truncate)) {
        return 2;
    }
    qsizetype stepIndex = 0;
    bool smokeFailed = false;
    bool stepDispatchPending = false;
    QQmlApplicationEngine engine;
    engine.load(QUrl(QStringLiteral("qrc:/qt/qml/cloud/highpriority/yonik/qml/Main.qml")));
    if (engine.rootObjects().isEmpty()) {
        return 1;
    }
    auto* window = qobject_cast<QWindow*>(engine.rootObjects().first());
    auto* bridge = engine.rootObjects().first()->findChild<DesktopBridge*>();
    if (!window || !bridge) {
        return 1;
    }
    // Все Qt-объекты живут в GUI-потоке и уничтожаются до QApplication через RAII.
    application.setQuitOnLastWindowClosed(false);
    QMenu menu;
    auto* showAction = menu.addAction(QStringLiteral("Открыть yonik"));
    auto* connectAction = menu.addAction(QStringLiteral("Подключить"));
    auto* stopAction = menu.addAction(QStringLiteral("Отключить"));
    auto* quitAction = menu.addAction(QStringLiteral("Выйти и отключиться"));
    auto* statusAction = new QAction(QStringLiteral("Отключено"), &menu);
    menu.insertAction(showAction, statusAction);
    statusAction->setEnabled(false);
    QSystemTrayIcon tray(statusIcon(QStringLiteral("Disconnected")));
    const auto updateTrayLabels = [window, bridge, &tray, statusAction, showAction, connectAction, stopAction, quitAction] {
        const bool english = window->property("language").toString() == QStringLiteral("en");
        const auto snapshot = QJsonDocument::fromJson(bridge->getSnapshot_json().toUtf8()).object();
        const auto state = snapshot.value(QStringLiteral("status")).toString();
        QString stateText;
        if (state == QStringLiteral("Connected")) stateText = english ? QStringLiteral("Connected") : QStringLiteral("Подключено");
        else if (state == QStringLiteral("Limited")) stateText = english ? QStringLiteral("Connection limited") : QStringLiteral("Соединение ограничено");
        else if (state == QStringLiteral("Error") || state == QStringLiteral("CleanupError")) stateText = english ? QStringLiteral("Error") : QStringLiteral("Ошибка");
        else if (state.isEmpty() || state == QStringLiteral("Disconnected")) stateText = english ? QStringLiteral("Disconnected") : QStringLiteral("Отключено");
        else stateText = english ? QStringLiteral("Connecting / stopping") : QStringLiteral("Подключение / остановка");
        statusAction->setText(stateText);
        const auto name = snapshot.value(QStringLiteral("active_name")).toString().left(80).toHtmlEscaped();
        const auto engineName = snapshot.value(QStringLiteral("active_engine")).toString().toHtmlEscaped();
        tray.setToolTip(QStringLiteral("<b>yonik</b><br/>") + stateText.toHtmlEscaped()
            + (name.isEmpty() ? QString() : QStringLiteral("<br/>") + name + QStringLiteral(" · ") + engineName));
        tray.setIcon(statusIcon(state));
        showAction->setText(english ? QStringLiteral("Open yonik") : QStringLiteral("Открыть yonik"));
        connectAction->setText(english ? QStringLiteral("Connect") : QStringLiteral("Подключить"));
        stopAction->setText(english ? QStringLiteral("Disconnect") : QStringLiteral("Отключить"));
        quitAction->setText(english ? QStringLiteral("Quit and Disconnect") : QStringLiteral("Выйти и отключиться"));
    };
    QObject::connect(&menu, &QMenu::aboutToShow, window, updateTrayLabels);
    updateTrayLabels();
    tray.setContextMenu(&menu);
    QObject::connect(showAction, &QAction::triggered, window, [window] {
        window->show();
        window->raise();
        window->requestActivate();
    });
    QDBusServiceWatcher trayWatcher(QStringLiteral("org.kde.StatusNotifierWatcher"),
        QDBusConnection::sessionBus(), QDBusServiceWatcher::WatchForUnregistration);
    QObject::connect(&trayWatcher, &QDBusServiceWatcher::serviceUnregistered, window,
        [showAction](const QString&) { showAction->trigger(); });
    QObject::connect(&server, &QLocalServer::newConnection, window, [&server, showAction] {
        while (server.hasPendingConnections()) {
            auto* socket = server.nextPendingConnection();
            if (!socket) {
                continue;
            }
            socket->setReadBufferSize(6);
            auto request = std::make_shared<QByteArray>();
            QObject::connect(socket, &QLocalSocket::readyRead, socket, [socket, request, showAction] {
                request->append(socket->read(6 - request->size()));
                if (*request == QByteArrayLiteral("show\n")) {
                    showAction->trigger();
                    socket->write("ok\n", 3);
                    socket->disconnectFromServer();
                } else if (request->size() >= 5) {
                    socket->abort();
                }
            });
            QObject::connect(socket, &QLocalSocket::disconnected, socket, &QObject::deleteLater);
            QTimer::singleShot(1500, socket, [socket] { socket->abort(); socket->deleteLater(); });
        }
    });
    QObject::connect(&tray, &QSystemTrayIcon::activated, window,
        [showAction](QSystemTrayIcon::ActivationReason reason) {
            if (reason == QSystemTrayIcon::Trigger || reason == QSystemTrayIcon::DoubleClick) {
                showAction->trigger();
            }
        });
    QObject::connect(quitAction, &QAction::triggered, bridge, [bridge] {
        bridge->shutdown();
    });
    QObject::connect(connectAction, &QAction::triggered, bridge, [bridge] {
        bridge->dispatch(QStringLiteral("{\"command\":\"connect\"}"));
    });
    QObject::connect(stopAction, &QAction::triggered, bridge, [bridge] {
        bridge->dispatch(QStringLiteral("{\"command\":\"stop\"}"));
    });
    connectAction->setEnabled(false);
    stopAction->setEnabled(false);
    bool cleanupConfirmed = false;
    const auto advanceSmokeStep = [&, bridge, window] {
        if (!smokeTest || stepIndex >= steps.size() || smokeFailed || stepDispatchPending) return;
        const auto snapshot = QJsonDocument::fromJson(bridge->getSnapshot_json().toUtf8()).object();
        if (snapshot.value(QStringLiteral("closed")).toBool()) return;
        const auto step = steps.at(stepIndex).toObject();
        const auto when = step.value(QStringLiteral("when")).toObject();
        const auto tcp = snapshot.value(QStringLiteral("tcp_result")).toObject();
        for (auto it = when.begin(); it != when.end(); ++it) {
            QJsonValue actual = snapshot.value(it.key());
            if (it.key() == QStringLiteral("sources_count")) actual = snapshot.value(QStringLiteral("sources")).toArray().size();
            if (it.key() == QStringLiteral("profiles_count")) actual = snapshot.value(QStringLiteral("profiles")).toArray().size();
            if (it.key() == QStringLiteral("pending")) actual = snapshot.value(QStringLiteral("pending")).isObject();
            if (it.key() == QStringLiteral("tcp_status")) actual = tcp.value(QStringLiteral("status"));
            if (it.key() == QStringLiteral("tcp_samples_count")) actual = tcp.value(QStringLiteral("samples_ms")).toArray().size();
            if (it.key() == QStringLiteral("tcp_samples_requested")) actual = tcp.value(QStringLiteral("samples_requested"));
            if (actual != it.value()) return;
        }
        if (!smoke::matches(qobject_cast<QQuickWindow*>(window), step.value(QStringLiteral("ui_when")).toObject())) return;
        const bool ui = step.value(QStringLiteral("ui")).isObject();
        if (ui == step.value(QStringLiteral("command")).isObject()) {
            smokeFailed = true;
            bridge->shutdown();
            return;
        }
        stepDispatchPending = true;
        ++stepIndex;
        const auto perform = [&, bridge, window, step, ui] {
            if (ui && step.value(QStringLiteral("ui")).toObject().value(QStringLiteral("action")).toString() == QStringLiteral("quit")) {
                quitAction->trigger();
            } else if (ui) {
                if (!smoke::perform(qobject_cast<QQuickWindow*>(window), step.value(QStringLiteral("ui")).toObject())) {
                    qWarning("yonik smoke: UI action failed at step %lld", static_cast<long long>(stepIndex));
                    smokeFailed = true;
                    bridge->shutdown();
                }
            } else {
                bridge->dispatch(QString::fromUtf8(QJsonDocument(step.value(QStringLiteral("command")).toObject()).toJson(QJsonDocument::Compact)));
            }
            stepDispatchPending = false;
        };
        if ((step.value(QStringLiteral("capture")).toBool() || step.contains(QStringLiteral("capture_name"))) && !screenshot.isEmpty()) {
            const auto path = smoke::capturePath(screenshot, step);
            if (path.isEmpty()) { smokeFailed = true; bridge->shutdown(); return; }
            captureWindow(window, path, [&, bridge, perform](bool saved) {
                if (!saved) { smokeFailed = true; bridge->shutdown(); return; }
                QTimer::singleShot(500, bridge, perform);
            });
        } else {
            QTimer::singleShot(0, bridge, perform);
        }
    };
    QTimer smokeSteps;
    if (smokeTest && !steps.isEmpty()) {
        smokeSteps.setInterval(25);
        QObject::connect(&smokeSteps, &QTimer::timeout, &smokeSteps, advanceSmokeStep);
        smokeSteps.start();
    }
    QObject::connect(bridge, &DesktopBridge::snapshot_jsonChanged, &smokeSteps,
        [&, bridge, window, smokeTest, screenshot] {
            const auto snapshot = QJsonDocument::fromJson(bridge->getSnapshot_json().toUtf8()).object();
            connectAction->setEnabled(snapshot.value(QStringLiteral("can_connect")).toBool());
            stopAction->setEnabled(snapshot.value(QStringLiteral("can_stop")).toBool());
            updateTrayLabels();
            // После неудачного выхода диагностика должна быть доступна даже из трея.
            if (snapshot.value(QStringLiteral("error")).toString() == QStringLiteral("cleanup_failed")
                && !snapshot.value(QStringLiteral("closed")).toBool()) {
                showAction->trigger();
            }
            QJsonObject metadata;
            for (const auto* key : {"status", "ready", "busy", "error", "revision", "selected_profile", "closed", "tcp_busy", "can_stop"}) {
                metadata.insert(QLatin1String(key), snapshot.value(QLatin1String(key)));
            }
            const auto sources = snapshot.value(QStringLiteral("sources")).toArray().size();
            const auto profiles = snapshot.value(QStringLiteral("profiles")).toArray().size();
            const bool pending = snapshot.value(QStringLiteral("pending")).isObject();
            metadata.insert(QStringLiteral("source_count"), sources);
            metadata.insert(QStringLiteral("profile_count"), profiles);
            metadata.insert(QStringLiteral("pending"), pending);
            metadata.insert(QStringLiteral("step"), stepIndex);
            metadata.insert(QStringLiteral("window_visible"), window->isVisible());
            metadata.insert(QStringLiteral("tray_available"), QSystemTrayIcon::isSystemTrayAvailable());
            const auto tcpResult = snapshot.value(QStringLiteral("tcp_result")).toObject();
            const auto tcpStatus = tcpResult.value(QStringLiteral("status"));
            const auto tcpSamples = tcpResult.value(QStringLiteral("samples_ms")).toArray().size();
            const auto tcpRequested = tcpResult.value(QStringLiteral("samples_requested"));
            metadata.insert(QStringLiteral("tcp_status"), tcpStatus);
            metadata.insert(QStringLiteral("tcp_samples_count"), tcpSamples);
            metadata.insert(QStringLiteral("tcp_samples_requested"), tcpRequested);
            metadata.insert(QStringLiteral("probe_present"), snapshot.value(QStringLiteral("probe_result")).isObject());
            if (smokeTest && traceFile.isOpen()) {
                const auto line = QJsonDocument(metadata).toJson(QJsonDocument::Compact) + '\n';
                if (traceFile.pos() + line.size() > 500000 || traceFile.write(line) != line.size()) {
                    smokeFailed = true;
                    bridge->shutdown();
                }
                traceFile.flush();
            }
            advanceSmokeStep();
            if (smokeTest && !screenshot.isEmpty() && !window->property("smokeCaptured").toBool()
                && script.isEmpty() && snapshot.contains(QStringLiteral("sources"))) {
                window->setProperty("smokeCaptured", true);
                QTimer::singleShot(250, window, [&, window, screenshot] {
                    captureWindow(window, screenshot, [&](bool saved) {
                        if (!saved) {
                            smokeFailed = true;
                            bridge->shutdown();
                        }
                    });
                });
            }
            if (snapshot.value(QStringLiteral("closed")).toBool()) {
                cleanupConfirmed = true;
                smokeFailed = smokeFailed || (smokeTest && (stepIndex < steps.size() || stepDispatchPending));
                application.quit();
            }
        });
    if (smokeTest) {
        // Видимость меняется без backend snapshot: trace фиксирует реальный close/relaunch.
        QObject::connect(window, &QWindow::visibleChanged, bridge, [bridge](bool) {
            Q_EMIT bridge->snapshot_jsonChanged();
        });
    }
    CloseHandler closeHandler(*window, *bridge, cleanupConfirmed);
    window->installEventFilter(&closeHandler);
    application.installEventFilter(&closeHandler);
    // Qt автоматически регистрирует иконку при последующем появлении tray-сервиса.
    tray.show();
    if (smokeTest) {
        // Окно включает 30-секундный deadline системного RPC и подтверждение Stop.
        QTimer::singleShot(45000, bridge, [&, bridge] {
            smokeFailed = smokeFailed || stepIndex < steps.size() || stepDispatchPending;
            if (smokeFailed) qWarning("yonik smoke: incomplete script at step %lld", static_cast<long long>(stepIndex));
            bridge->shutdown();
        });
    }
    const auto result = application.exec();
    return smokeTest && smokeFailed ? 2 : result;
}
}
