// A-08: normal clicked handlers production qrc; hidden dwell без polling.
#include <QQmlApplicationEngine>
#include <QQuickWindow>
#include <QQuickItem>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonArray>
#include <QFile>
#include <QTimer>
#include <QPointer>
#include <QCryptographicHash>
#include <dlfcn.h>
#include <functional>
class A08 final : public QObject {
    QQuickWindow* w; QObject* bridge; QString root; QJsonObject baseline;
    QPointer<QObject> oldEditor; int attempts=0; bool failed=false;
    QJsonObject state() { return QJsonDocument::fromJson(bridge->property("snapshot_json").toString().toUtf8()).object(); }
    QObject* find(QQuickItem* p, const QString& key, bool text=false) {
        if ((text ? p->property("text").toString() : p->objectName()) == key) return p;
        for(auto* c:p->childItems()) if(auto* result=find(c,key,text)) return result;
        return nullptr;
    }
    QObject* item(const QString& key) { return find(w->contentItem(),key); }
    QObject* field(QQuickItem* p) {
        if(p->property("placeholderText").toString()==QString::fromUtf8("Поиск программы")) return p;
        for(auto* c:p->childItems()) if(auto* result=field(c)) return result;
        return nullptr;
    }
    QObject* text(const QString& key) { return find(w->contentItem(),key,true); }
    void record(QString phase, QJsonObject data={}) { data["phase"]=phase; qWarning("A08 %s",QJsonDocument(data).toJson(QJsonDocument::Compact).constData()); }
    bool require(bool ok, QString why) { if(!ok && !failed) { failed=true; record("FAIL",{{"reason",why}}); QCoreApplication::exit(3); } return ok; }
    void wait(std::function<bool()> ready, std::function<void()> next) {
        if(failed) return; if(ready()) { attempts=0; next(); return; }
        if(!require(++attempts<400,"deadline")) return;
        QTimer::singleShot(10,this,[this,ready,next]{wait(ready,next);});
    }
    bool click(QObject* p) {
        if(p && p==load() && qEnvironmentVariable("A08_CASE")=="drop") { record("negative_drop_load"); return true; }
        return require(p && p->property("visible").toBool() && p->property("enabled").toBool(),"visible enabled control: "+(p?p->objectName()+"/"+p->property("text").toString():"missing")) && require(QMetaObject::invokeMethod(p,"clicked",Qt::DirectConnection),"clicked"); }
    void put(QString name) { QFile f(root+"/"+name); require(f.open(QIODevice::WriteOnly),"own barrier write"); f.write("1"); }
    QByteArray trace() { QFile f(root+"/catalog.jsonl"); if(!f.open(QIODevice::ReadOnly)) return {}; return f.readAll(); }
    bool samePolicy() { auto s=state(); return s["routing"]==baseline["routing"] && s["routing_revision"]==baseline["routing_revision"]; }
    QObject* load() { return text(QString::fromUtf8("Загрузить каталог")); }
    void open(std::function<void()> next) {
        if(!item("settingsRoutingButton") || !item("settingsRoutingButton")->property("visible").toBool()) {
            if(!click(item("settingsButton"))) return;
        }
        wait([this]{return item("settingsRoutingButton");},[this,next]{
            if(!click(item("settingsRoutingButton"))) return;
            wait([this]{return item("routingEditor") && load();},next);
        });
    }
    void desktop() {
        baseline=state();
        require(baseline["routing_diagnostics"].toObject()["error"]=="unsupported_selector","exact Linux mapping");
        auto* label=item("routingDiagnosticsIncomplete");
        require(label && label->property("visible").toBool() && label->property("text").toString().contains("#8"),"explainable rule mapping");
        record("known_unsupported",{{"diagnostics",baseline["routing_diagnostics"]}});
        if(!click(item("routingEdit8")) || !click(item("routingSaveRule"))) return;
        wait([this]{return !state()["routing_ack"].isNull() && state()["routing_ack"].toObject()["request_id"].toInt()>0;},[this]{
            auto after=state();
            require(after["routing"]==baseline["routing"],"edit preserves platform and other conditions");
            require(after["routing_revision"].toInt()==baseline["routing_revision"].toInt()+1,"save CAS revision");
            baseline=after;
            record("edit_preserved",{{"routing",baseline["routing"]},{"revision",baseline["routing_revision"]}});
            loadDesktop();
        });
    }
    void loadDesktop() {
        if(!click(load())) return;
        wait([this]{return state()["routing_catalog"].toObject()["total"].toInt()>0;},[this]{
            auto catalog=state()["routing_catalog"].toObject();
            const bool limited=qEnvironmentVariable("A08_CASE")!="complete";
            require(catalog["incomplete"].toBool()==limited,"catalog limit causal control");
            auto* explanation=text(QString::fromUtf8("Каталог неполный: достигнут лимит или нет доступа"));
            require(explanation && explanation->property("visible").toBool()==limited,"incomplete UI explanation");
            require(samePolicy(),"desktop policy unchanged");
            record("desktop",{{"catalog",catalog}});
            auto* check=text(QString::fromUtf8("Запущенные процессы"));
            require(check,"process checkbox"); if(failed) return;
            check->setProperty("checked",true);
            if(!click(load())) return;
            wait([this]{ for(auto v:state()["routing_catalog"].toObject()["items"].toArray()) if(v.toObject()["name"]=="a08-helper") return true; return false;},[this]{process();});
        });
    }
    void process() {
        auto catalog=state()["routing_catalog"].toObject();
        QString row;
        for(auto v:catalog["items"].toArray()) { auto p=v.toObject(); if(p["name"]=="a08-helper") row=p["name"].toString()+" · PID "+QString::number(p["pid"].toInt())+" · "+p["executable"].toString(); }
        require(!row.isEmpty(),"own helper candidate");
        if(!click(text(row))) return;
        require(samePolicy(),"selection does not save policy");
        record("process_selected",{{"catalog",catalog}});
        auto* query=field(w->contentItem());
        require(query,"catalog query field"); if(failed) return;
        query->setProperty("text","a08-helper");
        put("arm");
        if(!click(load())) return;
        wait([this]{return QFile::exists(root+"/held");},[this]{
            oldEditor=item("routingEditor");
            if(!click(item("homeButton"))) return;
            wait([this]{return oldEditor.isNull();},[this]{
                record("closed_inflight");
                // Один deadline: никаких hidden polls, screenshots или property reads.
                QTimer::singleShot(500,this,[this]{
                    auto before=trace();
                    if(qEnvironmentVariable("A08_CASE")=="hidden-scan") {
                        record("negative_hidden_scan");
                        QMetaObject::invokeMethod(bridge,"dispatch",Qt::DirectConnection,Q_ARG(QString,QString("{\"command\":\"routing_catalog\",\"processes\":true,\"query\":\"\",\"offset\":0}")));
                    }
                    put("release");
                    QTimer::singleShot(500,this,[this,before]{
                        auto after=trace();
                        require(after.count("admitted")==before.count("admitted"),"hidden no NEW admission");
                        require(after.count("read")==before.count("read"),"hidden no NEW read after completed IO");
                        require(samePolicy(),"late callback policy unchanged");
                        require(state()["routing_catalog"].toObject()["total"].toInt()==1,"late filtered catalog published while picker closed");
                        record("hidden",{{"before",QString::fromUtf8(before)},{"after",QString::fromUtf8(after)},{"late_catalog",state()["routing_catalog"]}});
                        reopen();
                    });
                });
            });
        });
    }
    void reopen() {
        open([this]{
            auto before=trace();
            require(samePolicy(),"reopen policy unchanged");
            auto* check=text(QString::fromUtf8("Запущенные процессы"));
            require(check && !check->property("checked").toBool(),"new picker defaults to desktop despite late process result");
            record("reopened",{{"catalog",state()["routing_catalog"]},{"processes_checked",check->property("checked").toBool()}});
            if(!click(load())) return;
            wait([this,before]{return trace().count("released")>before.count("released");},[this,before]{
                require(trace().count("read")>before.count("read"),"visible producer canary");
                require(samePolicy(),"final policy unchanged");
                record("completed",{{"routing",state()["routing"]},{"revision",state()["routing_revision"]}});
                QCoreApplication::exit(0);
            });
        });
    }
public:
    explicit A08(QQuickWindow* window):QObject(window),w(window),root(QString::fromUtf8(qgetenv("A08_ROOT"))) {
        bridge=w->property("backendForDiagnostics").value<QObject*>();
        if(!require(bridge,"production bridge")) return;
        wait([this]{return state()["ready"].toBool();},[this]{open([this]{desktop();});});
    }
};
void QQmlApplicationEngine::load(const QUrl& url) {
    using Load=void(*)(QQmlApplicationEngine*,const QUrl&);
    auto original=reinterpret_cast<Load>(dlsym(RTLD_NEXT,"_ZN21QQmlApplicationEngine4loadERK4QUrl"));
    if(!original || url.scheme()!="qrc") std::abort();
    QFile f(":"+url.path()); if(!f.open(QIODevice::ReadOnly)) std::abort();
    qWarning("A08 MAIN_SHA256 %s",QCryptographicHash::hash(f.readAll(),QCryptographicHash::Sha256).toHex().constData());
    original(this,url); if(rootObjects().isEmpty()) std::abort();
    auto* w=qobject_cast<QQuickWindow*>(rootObjects().first()); if(!w) std::abort();
    QTimer::singleShot(0,w,[w]{new A08(w);});
}
