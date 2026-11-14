// Только test-private interposer: production qrc и фактические Qt notify callsites.
#include <QQmlApplicationEngine>
#include <QQuickWindow>
#include <QQuickItem>
#include <QMetaProperty>
#include <QMetaMethod>
#include <QJsonDocument>
#include <QJsonObject>
#include <QFile>
#include <QCryptographicHash>
#include <QTimer>
#include <QElapsedTimer>
#include <QSet>
#include <atomic>
#include <dlfcn.h>
#include <cstdlib>

class P07Observer final : public QObject {
    Q_OBJECT
    QQuickWindow* window;
    QObject* bridge = nullptr;
    QObject* view = nullptr;
    QObject* rows = nullptr;
    QSet<QObject*> delegates;
    std::atomic<quint64> frames{0};
    quint64 serial = 0, pages = 0, responses = 0, models = 0, counts = 0;
    quint64 data = 0, text = 0, created = 0, snapshots = 0;
    QJsonObject before;
    QElapsedTimer clock;
    int phase = 0;
    bool suppressed = qEnvironmentVariableIsSet("P07_SUPPRESS_NOTIFY");

    static QObject* find(QQuickItem* item, const QString& name) {
        if (item->objectName() == name) return item;
        for (auto* child : item->childItems()) if (auto* result = find(child, name)) return result;
        return nullptr;
    }
    bool require(bool value, const char* reason) {
        if (value) return true;
        qCritical("P07 HIDDEN_FAIL %s", reason);
        phase = -1;
        QCoreApplication::exit(3);
        return false;
    }
    void emitRecord(const char* marker, const QJsonObject& value) {
        qInfo("P07 %s %s", marker, QJsonDocument(value).toJson(QJsonDocument::Compact).constData());
    }
    void observe(QObject* object, const char* property, const char* slot) {
        const int index = object->metaObject()->indexOfProperty(property);
        if (!require(index >= 0, property)) return;
        const auto meta = object->metaObject()->property(index);
        if (!require(meta.hasNotifySignal(), property) || suppressed) return;
        const int slotIndex = metaObject()->indexOfSlot(slot);
        require(QObject::connect(object, meta.notifySignal(), this, metaObject()->method(slotIndex), Qt::DirectConnection), slot);
    }
    void dispatch(const QString& value) {
        require(QMetaObject::invokeMethod(bridge, "dispatch", Qt::DirectConnection, Q_ARG(QString, value)), "dispatch");
    }
    bool click(const char* name) {
        auto* button = find(window->contentItem(), QString::fromLatin1(name));
        if (!require(button && button->property("visible").toBool() && button->property("enabled").toBool(), name)) return false;
        return require(QMetaObject::invokeMethod(button, "clicked", Qt::DirectConnection), name);
    }
    QJsonObject sample() const {
        return {{"t_ms", double(clock.elapsed())}, {"serial_notify", double(serial)}, {"serial_value", view->property("serial").toInt()},
                {"page_apply", double(pages)}, {"response_callback", double(responses)},
                {"model_notify", double(models)}, {"count_notify", double(counts)},
                {"delegate_data_notify", double(data)}, {"delegate_text_notify", double(text)},
                {"delegate_created", double(created)}, {"frame_swapped", double(frames.load())},
                {"snapshot_callback", double(snapshots)}, {"list_count", rows->property("count").toInt()}};
    }
    QJsonObject difference(const QJsonObject& start) const {
        auto result = sample();
        for (auto it = result.begin(); it != result.end(); ++it) *it = it.value().toDouble() - start.value(it.key()).toDouble();
        return result;
    }
    bool quietConsumer(const QJsonObject& delta) {
        for (const char* key : {"serial_notify", "serial_value", "page_apply", "model_notify", "count_notify", "delegate_data_notify", "delegate_text_notify", "delegate_created"})
            if (!require(delta.value(key).toDouble() == 0, key)) return false;
        return true;
    }
    void firstRefresh() {
        if (phase < 0) return;
        view = find(window->contentItem(), "logsView"); rows = find(window->contentItem(), "logsRows");
        if (!require(view && rows, "actual LogsView/ListView")) return;
        observe(view, "serial", "serialChanged()"); observe(view, "page", "pageChanged()");
        observe(view, "response", "responseChanged()"); observe(rows, "model", "modelChanged()");
        observe(rows, "count", "countChanged()");
        auto* content = qobject_cast<QQuickItem*>(rows->property("contentItem").value<QObject*>());
        if (!require(content, "ListView contentItem")) return;
        QObject::connect(content, &QQuickItem::childrenChanged, this, &P07Observer::attachDelegates);
        attachDelegates();
        dispatch(QStringLiteral("{\"command\":\"p07_logs\"}"));
        QTimer::singleShot(200, this, [this] {
            if (phase < 0) return;
            before = sample(); phase = 2;
            click("logsRefresh");
            QTimer::singleShot(250, this, &P07Observer::visibleCanary);
        });
    }
    void visibleCanary() {
        if (phase < 0) return;
        const auto delta = difference(before); emitRecord("VISIBLE_CANARY", delta);
        if (!require(delta["serial_notify"].toDouble() > 0 && delta["page_apply"].toDouble() > 0
                     && rows->property("count").toInt() > 0 && delta["model_notify"].toDouble() > 0
                     && delta["delegate_created"].toDouble() + delta["delegate_data_notify"].toDouble() + delta["delegate_text_notify"].toDouble() > 0,
                     "visible refresh notify canary")) return;
        // Отдельный render canary; requestUpdate запрещён в измеряемом hidden interval.
        const auto frameBefore = frames.load();
        window->update();
        QTimer::singleShot(200, this, [this, frameBefore] {
            if (phase < 0) return;
            emitRecord("FRAME_CANARY", {{"delta", double(frames.load() - frameBefore)}});
            if (!require(frames.load() > frameBefore, "frameSwapped canary")) return;
            before = sample(); emitRecord("QUIET_BEGIN", before);
            QTimer::singleShot(150, this, [this] {
                if (phase < 0) return;
                const auto delta = difference(before); emitRecord("QUIET_END", delta);
                if (!quietConsumer(delta)) return;
                window->hide();
                // Ожидаем завершения visibility transition до начала passive interval.
                QTimer::singleShot(200, this, &P07Observer::hiddenBegin);
            });
        });
    }
    void hiddenBegin() {
        if (phase < 0 || !require(!window->isVisible(), "window hidden")) return;
        before = sample(); phase = 3; emitRecord("HIDDEN_BEGIN", before);
        QTimer::singleShot(1200, Qt::PreciseTimer, this, [this] {
            if (phase < 0) return;
            const auto delta = difference(before); emitRecord("HIDDEN_END", delta);
            if (!quietConsumer(delta) || !require(delta["frame_swapped"].toDouble() == 0, "hidden frameSwapped")) return;
            // Состояние/response — отдельные callbacks, не row counters; допустимы bounded notifications.
            if (!require(delta["snapshot_callback"].toDouble() <= 4 && delta["response_callback"].toDouble() <= 4, "bounded hidden callbacks")) return;
            window->show(); phase = 4;
            QTimer::singleShot(200, this, [this] {
                if (phase < 0) return;
                before = sample(); click("logsRefresh");
                QTimer::singleShot(250, this, [this] {
                    if (phase < 0) return;
                    const auto delta = difference(before); emitRecord("POST_VISIBLE_CANARY", delta);
                    if (!require(delta["serial_notify"].toDouble() > 0 && delta["page_apply"].toDouble() > 0, "post-hidden consumer live")) return;
                    qInfo("P07 HIDDEN_PASS");
                    dispatch(QStringLiteral("{\"command\":\"shutdown\"}")); phase = 5;
                });
            });
        });
    }
public:
    P07Observer(QQuickWindow* root, QObject* parent) : QObject(parent), window(root) {
        clock.start();
        for (auto* object : root->findChildren<QObject*>()) {
            if (object->metaObject()->indexOfProperty("snapshot_json") >= 0) { bridge = object; break; }
        }
        if (!require(bridge, "actual DesktopBridge")) return;
        if (!qEnvironmentVariableIsSet("P07_SUPPRESS_FRAME"))
            QObject::connect(window, &QQuickWindow::frameSwapped, this, [this] { frames.fetch_add(1); }, Qt::DirectConnection);
        // Startup callback и fixed event connections; никаких periodic timers.
        const int signalIndex = bridge->metaObject()->indexOfSignal("snapshot_jsonChanged()");
        const int slotIndex = metaObject()->indexOfSlot("snapshotChanged()");
        if (!require(signalIndex >= 0, "snapshot_jsonChanged")) return;
        QObject::connect(bridge, bridge->metaObject()->method(signalIndex), this, metaObject()->method(slotIndex));
        QTimer::singleShot(0, this, &P07Observer::snapshotChanged);
        QTimer::singleShot(15000, this, [this] { if (phase != 5 && phase != -1) require(false, "focused deadline"); });
    }
private slots:
    void snapshotChanged() {
        if (sender() == bridge) snapshots++;
        if (phase != 0) return;
        const auto state = QJsonDocument::fromJson(bridge->property("snapshot_json").toString().toUtf8()).object();
        if (!state["ready"].toBool()) return;
        phase = 1;
        if (!click("settingsButton")) return;
        QTimer::singleShot(50, this, [this] {
            if (phase < 0 || !click("settingsDiagnosticsButton")) return;
            QTimer::singleShot(50, this, &P07Observer::firstRefresh);
        });
    }
    void serialChanged() { serial++; }
    void pageChanged() { pages++; }
    void responseChanged() { responses++; }
    void modelChanged() { models++; }
    void countChanged() { counts++; }
    void dataChanged() { data++; }
    void textChanged() { text++; }
    void attachDelegates() {
        auto* content = qobject_cast<QQuickItem*>(rows->property("contentItem").value<QObject*>());
        if (!content) return;
        for (auto* child : content->childItems()) {
            if (child->metaObject()->indexOfProperty("modelData") < 0 || delegates.contains(child)) continue;
            delegates.insert(child); created++;
            observe(child, "modelData", "dataChanged()"); observe(child, "text", "textChanged()");
            QObject::connect(child, &QObject::destroyed, this, [this, child] { delegates.remove(child); });
        }
    }
};

void QQmlApplicationEngine::load(const QUrl& url) {
    using Load = void (*)(QQmlApplicationEngine*, const QUrl&);
    auto original = reinterpret_cast<Load>(dlsym(RTLD_NEXT, "_ZN21QQmlApplicationEngine4loadERK4QUrl"));
    if (!original || url.scheme() != "qrc") std::abort();
    QFile source(":" + url.path());
    if (!source.open(QIODevice::ReadOnly)) std::abort();
    qInfo("P07 MAIN_SHA256 %s", QCryptographicHash::hash(source.readAll(), QCryptographicHash::Sha256).toHex().constData());
    original(this, url);
    if (rootObjects().isEmpty()) std::abort();
    auto* window = qobject_cast<QQuickWindow*>(rootObjects().first());
    if (!window) std::abort();
    QTimer::singleShot(0, this, [this, window] { new P07Observer(window, this); });
}
#include "p07_hidden_observer.moc"
