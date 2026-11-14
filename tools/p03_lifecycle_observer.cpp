// Private P-03: исходный qrc, реальные controls и CloseHandler. Не подменяет tray.
#include <QQmlApplicationEngine>
#include <QQuickWindow>
#include <QQuickItem>
#include <QJsonDocument>
#include <QJsonObject>
#include <QFile>
#include <QDir>
#include <QLocalSocket>
#include <QStandardPaths>
#include <QCryptographicHash>
#include <QTimer>
#include <QElapsedTimer>
#include <QPointer>
#include <QMetaMethod>
#include <QSystemTrayIcon>
#include <QDBusConnection>
#include <QDBusAbstractAdaptor>
#include <dlfcn.h>
#include <cstdlib>

class PrivateWatcher final : public QDBusAbstractAdaptor {
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.kde.StatusNotifierWatcher")
    Q_PROPERTY(bool IsStatusNotifierHostRegistered READ registered)
public:
    explicit PrivateWatcher(QObject* parent) : QDBusAbstractAdaptor(parent) {}
    bool registered() const { return true; }
public slots:
    void RegisterStatusNotifierItem(const QString&) {}
    void RegisterStatusNotifierHost(const QString&) {}
};
class P03Observer final : public QObject {
    Q_OBJECT
    QQuickWindow* window;
    QObject* bridge;
    QPointer<QObject> editor;
    QList<QPointer<QObject>> ownedObjects;
    QElapsedTimer timer;
    int notifications = 0;
    bool failed = false;
    int cycle = 0;
    QJsonObject initial;
    static QObject* find(QQuickItem* item, const QString& name) {
        if (item->objectName() == name) return item;
        for (auto* child : item->childItems()) if (auto* result = find(child, name)) return result;
        return nullptr;
    }
    void record(const char* phase, QJsonObject value) {
        value["phase"] = phase;
        value["cycle"] = cycle;
        value["classification"] = cycle < 10 ? "warmup" : "measured";
        QFile memory("/proc/self/smaps_rollup");
        if (memory.open(QIODevice::ReadOnly)) value["smaps_rollup"] = QString::fromUtf8(memory.readAll());
        value["fd"] = QDir("/proc/self/fd").entryList(QDir::AllEntries | QDir::NoDotAndDotDot).size();
        value["threads"] = QDir("/proc/self/task").entryList(QDir::Dirs | QDir::NoDotAndDotDot).size();
        qInfo("P03 %s", QJsonDocument(value).toJson(QJsonDocument::Compact).constData());
    }
    void stop(const char* reason) {
        if (failed) return;
        failed = true;
        record("blocked", {{"reason", reason}});
        QCoreApplication::exit(3);
    }
    bool click(const char* name) {
        auto* item = find(window->contentItem(), QString::fromLatin1(name));
        if (!item || !item->property("visible").toBool() || !item->property("enabled").toBool()) { stop(name); return false; }
        if (!QMetaObject::invokeMethod(item, "clicked", Qt::DirectConnection)) { stop("clicked unavailable"); return false; }
        return true;
    }
    void openRules() {
        notifications = 0;
        if (!click("settingsButton")) return;
        QTimer::singleShot(0, this, [this] {
            if (!click("settingsRoutingButton")) return;
            QTimer::singleShot(200, this, [this] { modeCanary(); });
        });
    }
    void modeCanary() {
        editor = find(window->contentItem(), "routingEditor");
        if (!editor || editor->property("advanced").toBool()) { stop("initial simple editor"); return; }
        ownedObjects.clear();
        int connections = 0;
        for (auto* object : editor->findChildren<QObject*>()) {
            ownedObjects.append(object);
            if (QByteArray(object->metaObject()->className()).contains("Connections")) connections++;
        }
        record("editor_objects", {{"descendants", ownedObjects.size()}, {"connections_objects", connections}});
        const auto signal = editor->metaObject()->indexOfSignal("advancedChanged()");
        // Считаем реальные notify доставленные один раз; отрицательный контроль отключает наблюдение.
        if (!qEnvironmentVariableIsSet("P03_DISCONNECT"))
            QObject::connect(editor, editor->metaObject()->method(signal), this, metaObject()->method(metaObject()->indexOfSlot("modeChanged()")));
        if (qEnvironmentVariableIsSet("P03_DUPLICATE"))
            QObject::connect(editor, editor->metaObject()->method(signal), this, metaObject()->method(metaObject()->indexOfSlot("modeChanged()")));
        timer.start();
        if (!click("routingModeButton")) return;
        QTimer::singleShot(0, this, [this] {
            if (!editor || !editor->property("advanced").toBool() || notifications != 1) { stop("advanced notify canary"); return; }
            record("advanced_completed", {{"dispatch_to_state_ms", double(timer.nsecsElapsed()) / 1e6}, {"notify", notifications}});
            timer.restart();
            if (!click("routingModeButton")) return;
            QTimer::singleShot(0, this, [this] {
                if (!editor || editor->property("advanced").toBool() || notifications != 2) { stop("simple notify canary"); return; }
                record("simple_completed", {{"dispatch_to_state_ms", double(timer.nsecsElapsed()) / 1e6}, {"notify", notifications}});
                const auto after = QJsonDocument::fromJson(bridge->property("snapshot_json").toString().toUtf8()).object();
                const bool policyEqual = after["routing"] == initial["routing"] && after["routing_revision"] == initial["routing_revision"] && after["revision"] == initial["revision"];
                record("policy_check", {{"equal", policyEqual}, {"routing", after["routing"]}, {"revision", after["revision"]}, {"routing_revision", after["routing_revision"]}});
                if (!policyEqual) { stop("policy changed"); return; }
                if (!qEnvironmentVariableIsSet("P03_RETAIN_EDITOR") && !click("homeButton")) return;
                QTimer::singleShot(0, this, [this] {
                    int survivors = 0;
                    for (const auto& object : ownedObjects) if (object) survivors++;
                    record("editor_release", {{"destroyed", editor.isNull()}, {"observed_descendants_surviving", survivors}});
                    if (survivors) { stop("editor descendants retained"); return; }
                    if (editor) { stop("Loader retained editor"); return; }
                    if (!click("homeButton")) return;
                    timer.restart();
                    window->close(); // Действительный QCloseEvent проходит production event filter.
                    QTimer::singleShot(50, this, [this] {
                        record("close_completed", {{"visible", window->isVisible()}, {"tray_available", QSystemTrayIcon::isSystemTrayAvailable()}, {"dispatch_to_observation_ms", double(timer.nsecsElapsed()) / 1e6}});
                        if (window->isVisible()) { stop("production CloseHandler keeps window visible without tray; no P03 close/reopen cycles admitted"); return; }
                        const auto key = QCryptographicHash::hash(qgetenv("P03_DATA_DIR"), QCryptographicHash::Sha256).toHex();
                        const auto path = QDir(QStandardPaths::writableLocation(QStandardPaths::RuntimeLocation)).filePath("tofu-" + QString::fromLatin1(key));
                        auto* socket = new QLocalSocket(this);
                        timer.restart();
                        QObject::connect(socket, &QLocalSocket::connected, this, [socket] { socket->write("show\n", 5); });
                        QObject::connect(socket, &QLocalSocket::readyRead, this, [this, socket] {
                            if (socket->readAll() != "ok\n" || !window->isVisible()) { stop("production reopen failed"); return; }
                            record("reopened", {{"dispatch_to_state_ms", double(timer.nsecsElapsed()) / 1e6}});
                            socket->deleteLater();
                            if (++cycle == 100) {
                                record("completed", {{"cycles", cycle}});
                                QMetaObject::invokeMethod(bridge, "dispatch", Qt::DirectConnection, Q_ARG(QString, QStringLiteral("{\"command\":\"shutdown\"}")));
                            } else QTimer::singleShot(20, this, [this] { openRules(); });
                        });
                        socket->connectToServer(path);
                    });
                });
            });
        });
    }
public:
    P03Observer(QQuickWindow* root, QObject* parent) : QObject(parent), window(root), bridge(root->property("backendForDiagnostics").value<QObject*>()) {
        if (!bridge) { stop("DesktopBridge missing"); return; }
        QTimer::singleShot(1000, this, [this] {
            const auto state = QJsonDocument::fromJson(bridge->property("snapshot_json").toString().toUtf8()).object();
            initial = state;
            record("initial", {{"ready", state["ready"]}, {"revision", state["revision"]}, {"routing_revision", state["routing_revision"]}, {"routing", state["routing"]}});
            if (!state["ready"].toBool()) { stop("host not ready"); return; }
            openRules();
        });
        QTimer::singleShot(120000, this, [this] { stop("deadline"); });
    }
private slots:
    void modeChanged() { notifications++; }
};
void QQmlApplicationEngine::load(const QUrl& url) {
    using Load = void (*)(QQmlApplicationEngine*, const QUrl&);
    auto original = reinterpret_cast<Load>(dlsym(RTLD_NEXT, "_ZN21QQmlApplicationEngine4loadERK4QUrl"));
    if (!original || url.scheme() != "qrc") std::abort();
    QFile source(":" + url.path());
    if (!source.open(QIODevice::ReadOnly)) std::abort();
    qInfo("P03 MAIN_SHA256 %s", QCryptographicHash::hash(source.readAll(), QCryptographicHash::Sha256).toHex().constData());
    if (qEnvironmentVariableIsSet("P03_PRIVATE_WATCHER")) {
        auto* watcher = new QObject(this);
        new PrivateWatcher(watcher);
        auto bus = QDBusConnection::sessionBus();
        if (!bus.registerService("org.kde.StatusNotifierWatcher") || !bus.registerObject("/StatusNotifierWatcher", watcher, QDBusConnection::ExportAdaptors)) std::abort();
    }
    original(this, url);
    if (rootObjects().isEmpty()) std::abort();
    auto* window = qobject_cast<QQuickWindow*>(rootObjects().first());
    if (!window) std::abort();
    QTimer::singleShot(0, this, [this, window] { new P03Observer(window, this); });
}
#include "p03_lifecycle_observer.moc"
