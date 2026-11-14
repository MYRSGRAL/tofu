// P-03: test-private observer. Production qrc, dispatch, CloseHandler и show server.
#include <QQmlApplicationEngine>
#include <QQuickWindow>
#include <QQuickItem>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonArray>
#include <QFile>
#include <QDir>
#include <QLocalSocket>
#include <QStandardPaths>
#include <QCryptographicHash>
#include <QTimer>
#include <QElapsedTimer>
#include <QPointer>
#include <QThread>
#include <QMetaMethod>
#include <QDBusConnection>
#include <QDBusAbstractAdaptor>
#include <atomic>
#include <functional>
#include <dlfcn.h>
#include <cstdlib>

class P03Watcher final : public QDBusAbstractAdaptor {
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.kde.StatusNotifierWatcher")
    Q_PROPERTY(bool IsStatusNotifierHostRegistered READ registered)
public:
    explicit P03Watcher(QObject* parent) : QDBusAbstractAdaptor(parent) {}
    bool registered() const { return true; }
public slots:
    void RegisterStatusNotifierItem(const QString&) {}
    void RegisterStatusNotifierHost(const QString&) {}
};
class P03Integrated final : public QObject {
    Q_OBJECT
    QQuickWindow* window;
    QObject* bridge;
    QElapsedTimer clock;
    std::atomic<qint64> frameNs{0};
    std::atomic<int> frames{0};
    bool failed = false;
    int cycle = 0, limit = qEnvironmentVariableIntValue("P03_CYCLES");
    int requestId = 100;
    QJsonObject baseline;
    QPointer<QObject> editor;
    QList<QPointer<QObject>> owned;
    int modeNotify = 0;
    QJsonObject fields;
    qint64 traceOffset = 0;
    QByteArray traceTail;
    int admittedVisible = 0, admittedHidden = 0, loadedObservations = 0;
    bool effectiveVisible = false;
    std::function<bool()> pendingReady;
    qint64 observedReady = 0;
    static QObject* find(QQuickItem* item, const QString& name) {
        if (item->objectName() == name) return item;
        for (auto* child : item->childItems()) if (auto* result = find(child, name)) return result;
        return nullptr;
    }
    QObject* item(const char* name) { return find(window->contentItem(), QString::fromLatin1(name)); }
    QJsonObject state() { return QJsonDocument::fromJson(bridge->property("snapshot_json").toString().toUtf8()).object(); }
    void record(const char* phase, QJsonObject value = {}) {
        value["phase"] = phase; value["cycle"] = cycle;
        value["classification"] = cycle < 10 ? "warmup" : "measured";
        value["t_ns"] = double(clock.nsecsElapsed()); value["frames"] = frames.load();
        QFile memory("/proc/self/smaps_rollup");
        if (memory.open(QIODevice::ReadOnly)) value["smaps_rollup"] = QString::fromUtf8(memory.readAll());
        value["fd"] = QDir("/proc/self/fd").entryList(QDir::AllEntries | QDir::NoDotAndDotDot).size();
        value["threads"] = QDir("/proc/self/task").entryList(QDir::Dirs | QDir::NoDotAndDotDot).size();
        qInfo("P03I %s", QJsonDocument(value).toJson(QJsonDocument::Compact).constData());
    }
    bool require(bool value, const char* reason) {
        if (failed) return false;
        if (value) return true;
        failed = true; record("FAIL", {{"reason", reason}}); QCoreApplication::exit(3); return false;
    }
    void wait(std::function<bool()> predicate, std::function<void()> next, const char* reason, qint64 until = 0) {
        if (failed) return;
        if (predicate()) { next(); return; }
        if (!until) until = clock.elapsed() + 1500;
        if (!require(clock.elapsed() < until, reason)) return;
        QTimer::singleShot(1, this, [this, predicate, next, reason, until] { wait(predicate, next, reason, until); });
    }
    bool click(const char* name) {
        auto* button = item(name);
        if (!require(button && button->property("visible").toBool() && button->property("enabled").toBool(), name)) return false;
        return require(QMetaObject::invokeMethod(button, "clicked", Qt::DirectConnection), "actual clicked");
    }
    void dispatch(QJsonObject command) {
        require(QMetaObject::invokeMethod(bridge, "dispatch", Qt::DirectConnection, Q_ARG(QString, QString::fromUtf8(QJsonDocument(command).toJson(QJsonDocument::Compact)))), "actual dispatch");
    }
    QJsonObject gateSummary() {
        QFile file(QString::fromUtf8(qgetenv("P03_TRACE")));
        if (!require(file.open(QIODevice::ReadOnly), "trace file")) return {};
        file.seek(traceOffset); const auto added = file.readAll(); traceOffset += added.size(); traceTail += added;
        for (;;) {
            const auto end = traceTail.indexOf('\n'); if (end < 0) break;
            const auto event = QJsonDocument::fromJson(traceTail.left(end)).object(); traceTail.remove(0,end+1);
            if (!require(!event.isEmpty(), "complete trace JSON record")) return {};
            if (event["kind"] == "admitted" && event["request"].toObject()["command"] == "subscription_visibility") {
                if (event["request"].toObject()["visible"].toBool()) admittedVisible++; else admittedHidden++;
            }
            if (event["kind"] == "schedule_gate" && event["loaded"].toBool()) { loadedObservations++; effectiveVisible = event["visible"].toBool(); }
        }
        return {{"admitted_visible",admittedVisible},{"admitted_hidden",admittedHidden},{"loaded_gate_observations",loadedObservations},{"effective_visible",effectiveVisible}};
    }
    void action(const char* name, std::function<void()> invoke, std::function<bool()> ready, std::function<void()> next) {
        const auto start = clock.nsecsElapsed();
        observedReady = 0;
        pendingReady = ready;
        invoke();
        if (ready()) observedReady = clock.nsecsElapsed();
        wait([this,ready] { if (!observedReady && ready()) observedReady=clock.nsecsElapsed(); return observedReady>0; }, [this, name, start, next] {
            const auto model = observedReady;
            pendingReady = {};
            wait([this, model] { return frameNs.load() >= model; }, [this, name, start, model, next] {
                record(name, {{"dispatch_to_ready_ms",double(model-start)/1e6},{"dispatch_to_frame_ms",double(frameNs.load()-start)/1e6}});
                next();
            }, "action rendered frame deadline");
        }, "action Loader/model ready deadline");
    }
    void seed(int id) {
        if (id == 3) {
            baseline = state();
            if (!require(baseline["routing"].toObject()["rules"].toArray().size() == 2, "mixed policy persisted")) return;
            record("seeded", {{"routing",baseline["routing"]},{"routing_revision",baseline["routing_revision"]},{"revision",baseline["revision"]}});
            wait([this] { const auto g=gateSummary(); return g["loaded_gate_observations"].toInt()>0 && g["effective_visible"].toBool(); }, [this] {
                if (!require(gateSummary()["admitted_visible"].toInt()==1,"initial single visibility admission")) return;
                visibleCanary();
            }, "initial backend visible gate"); return;
        }
        const int request = ++requestId;
        QJsonObject rule = id == 1 ? QJsonObject{{"id",1},{"enabled",true},{"executables",QJsonArray{"/usr/bin/browser","/usr/bin/mail"}},{"domains",QJsonArray{"*.example.test","exact.example.test"}},{"destinations",QJsonArray{"192.0.2.0/24","198.51.100.0/24"}},{"action","block"}}
            : QJsonObject{{"id",2},{"enabled",true},{"executables",QJsonArray{}},{"domains",QJsonArray{"fallback.example.test"}},{"destinations",QJsonArray{}},{"action","direct"}};
        dispatch({{"command","edit_routing"},{"expected_revision",state()["routing_revision"]},{"request_id",request},{"create",true},{"confirm_broad",false},{"rule",rule}});
        wait([this, request] { return state()["routing_ack"].toObject()["request_id"].toInt() == request; }, [this,id] {
            if (!require(state()["routing_ack"].toObject()["error"].isNull(), "seed production ack")) return;
            seed(id+1);
        }, "seed acknowledgement");
    }
    void visibleCanary() {
        const auto before = frames.load(); window->update();
        wait([this,before] { return frames.load() > before; }, [this] { record("visible_frame_canary"); openRules(); }, "visible frame canary");
    }
    QJsonObject editorFields() {
        return {{"executable",item("routingExecutable")->property("text").toString()}, {"domain",item("routingDomain")->property("text").toString()}, {"cidr",item("routingCidr")->property("text").toString()}, {"selected",editor->property("selectedId").toInt()}};
    }
    void openRules() {
        if (!click("settingsButton")) return;
        action("rules_ready_frame", [this] { click("settingsRoutingButton"); }, [this] {
            auto* loader = item("settingsPanel");
            return loader && loader->property("status").toInt() == 1 && item("routingEditor") && item("routingEdit1") && item("routingEdit2");
        }, [this] {
            editor = item("routingEditor"); owned.clear(); modeNotify = 0;
            for (auto* object : editor->findChildren<QObject*>()) owned.append(object);
            // Repeater visual delegates могут иметь другой QObject owner: следим за actual rows отдельно.
            for (const auto* name : {"routingEdit1", "routingEdit2"}) {
                auto* rowControl = item(name);
                if (!require(rowControl, "actual rule row model control")) return;
                if (!owned.contains(QPointer<QObject>(rowControl))) owned.append(rowControl);
            }
            const auto signal = editor->metaObject()->indexOfSignal("advancedChanged()");
            QObject::connect(editor, editor->metaObject()->method(signal), this, metaObject()->method(metaObject()->indexOfSlot("modeChanged()")));
            if (!click("routingEdit1")) return;
            fields = editorFields();
            if (!require(fields["executable"] == "/usr/bin/browser\n/usr/bin/mail" && fields["domain"] == "*.example.test\nexact.example.test" && fields["cidr"] == "192.0.2.0/24\n198.51.100.0/24", "mixed rule actual controls")) return;
            mode(true);
        });
    }
    void mode(bool advanced) {
        action(advanced ? "advanced_ready_frame" : "simple_ready_frame", [this] { click("routingModeButton"); }, [this,advanced] { return editor && editor->property("advanced").toBool() == advanced; }, [this,advanced] {
            if (!require(modeNotify == (advanced ? 1 : 2), "single production mode transition")) return;
            if (advanced && qEnvironmentVariableIsSet("P03_CORRUPT_FIELDS")) item("routingDomain")->setProperty("text", "lost.example.test");
            if (!require(editorFields() == fields, "mixed fields preserved across views")) return;
            if (!require(item("routingExecutable")->property("visible").toBool() && item("routingDomain")->property("visible").toBool() == advanced && item("routingCidr")->property("visible").toBool() == advanced, "simple advanced view semantics")) return;
            const auto now = state();
            if (!require(now["routing"] == baseline["routing"] && now["routing_revision"] == baseline["routing_revision"] && now["revision"] == baseline["revision"], "no unintended policy revision change")) return;
            record("policy_and_fields",{{"advanced",advanced},{"routing",now["routing"]},{"fields",fields},{"routing_revision",now["routing_revision"]},{"revision",now["revision"]}});
            if (advanced) mode(false); else release();
        });
    }
    void release() {
        if (qEnvironmentVariableIsSet("P03_RETAIN_RULE_CONTROL")) {
            auto* kept = item("routingEdit1");
            kept->setParent(this);
            qobject_cast<QQuickItem*>(kept)->setParentItem(nullptr);
        }
        if (!click("homeButton")) return;
        wait([this] { if (editor) return false; for (const auto& object : owned) if (object) return false; return true; }, [this] {
            record("released",{{"observed_objects",owned.size()},{"actual_rule_controls_destroyed",true}});
            if (!click("homeButton")) return;
            const auto start = clock.nsecsElapsed(); window->close();
            wait([this] { const auto g = gateSummary(); return !window->isVisible() && g["loaded_gate_observations"].toInt() > 0 && !g["effective_visible"].toBool(); }, [this,start] {
                auto g = gateSummary();
                if (!require(g["admitted_visible"].toInt() == cycle+1 && g["admitted_hidden"].toInt() == cycle+1, "exact visibility admission cardinality")) return;
                g["dispatch_to_gate_ms"] = double(clock.nsecsElapsed()-start)/1e6; record("closed_gate",g);
                QTimer::singleShot(30, this, [this] {
                    const auto before = frames.load();
                    QTimer::singleShot(60, this, [this,before] {
                        const auto g = gateSummary();
                        if (!require(frames.load()==before && !g["effective_visible"].toBool(), "hidden frame/gate quiet")) return;
                        record("hidden_quiet",g); reopen();
                    });
                });
            }, "backend hidden release gate");
        }, "editor tree release");
    }
    void reopen() {
        const auto key = QCryptographicHash::hash(qgetenv("P03_DATA_DIR"),QCryptographicHash::Sha256).toHex();
        auto* socket = new QLocalSocket(this);
        action("reopen_ready_frame", [this,socket,key] {
            QObject::connect(socket,&QLocalSocket::connected,this,[socket] { socket->write("show\n",5); });
            socket->connectToServer(QDir(QStandardPaths::writableLocation(QStandardPaths::RuntimeLocation)).filePath("tofu-"+QString::fromLatin1(key)));
        }, [this,socket] {
            return window->isVisible() && socket->state() == QLocalSocket::ConnectedState;
        }, [this,socket] {
            wait([this,socket] { const auto g=gateSummary(); return socket->bytesAvailable()>=3 && g["loaded_gate_observations"].toInt()>0 && g["effective_visible"].toBool(); }, [this,socket] {
            if (!require(socket->readAll()=="ok\n", "show actual ack")) return;
            socket->deleteLater(); const auto g = gateSummary();
            if (!require(g["admitted_visible"].toInt()==cycle+2 && g["admitted_hidden"].toInt()==cycle+1,"reopen exact admission")) return;
            record("reopened_gate",g);
            if (++cycle == limit) { record("PASS",{{"cycles",cycle}}); dispatch({{"command","shutdown"}}); }
            else QTimer::singleShot(0,this,[this] { openRules(); });
            }, "backend visible admission gate");
        });
    }
public:
    P03Integrated(QQuickWindow* root,QObject* parent) : QObject(parent),window(root),bridge(root->property("backendForDiagnostics").value<QObject*>()) {
        clock.start();
        if (!require(bridge && limit>0 && limit<=100,"fixture arguments")) return;
        QObject::connect(window,&QQuickWindow::afterAnimating,this,[this] {
            if (!require(QThread::currentThread()==window->thread(),"GUI readiness observer thread")) return;
            if (pendingReady && !observedReady && pendingReady()) observedReady=clock.nsecsElapsed();
        },Qt::DirectConnection);
        if (!qEnvironmentVariableIsSet("P03_SUPPRESS_FRAME"))
            QObject::connect(window,&QQuickWindow::frameSwapped,this,[this] { frameNs.store(clock.nsecsElapsed());frames.fetch_add(1); },Qt::DirectConnection);
        QTimer::singleShot(0,this,[this] { wait([this] { return state()["ready"].toBool(); },[this] { seed(1); },"host ready"); });
        QTimer::singleShot(120000,this,[this] { require(false,"campaign deadline"); });
    }
private slots:
    void modeChanged() { modeNotify++; }
};
void QQmlApplicationEngine::load(const QUrl& url) {
    using Load=void (*)(QQmlApplicationEngine*,const QUrl&);
    auto original=reinterpret_cast<Load>(dlsym(RTLD_NEXT,"_ZN21QQmlApplicationEngine4loadERK4QUrl"));
    if (!original || url.scheme()!="qrc") std::abort();
    QFile source(":"+url.path()); if (!source.open(QIODevice::ReadOnly)) std::abort();
    qInfo("P03I MAIN_SHA256 %s",QCryptographicHash::hash(source.readAll(),QCryptographicHash::Sha256).toHex().constData());
    auto* watcher=new QObject(this);new P03Watcher(watcher);auto bus=QDBusConnection::sessionBus();
    if (!bus.registerService("org.kde.StatusNotifierWatcher") || !bus.registerObject("/StatusNotifierWatcher",watcher,QDBusConnection::ExportAdaptors)) std::abort();
    original(this,url); if (rootObjects().isEmpty()) std::abort();
    auto* window=qobject_cast<QQuickWindow*>(rootObjects().first());if (!window) std::abort();
    QTimer::singleShot(0,this,[this,window] { new P03Integrated(window,this); });
}
#include "p03_integrated_observer.moc"
