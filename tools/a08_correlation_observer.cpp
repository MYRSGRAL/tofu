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
    QString helperRow; QByteArray hiddenBefore;
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
    double expected() { auto* editor=item("routingEditor"); return editor?editor->property("catalogExpectedId").toDouble():0; }
    double replyId() { return state()["routing_catalog"].toObject()["request_id"].toDouble(); }
    int rows() { auto* repeater=item("routingCatalogRows"); return repeater?repeater->property("count").toInt():-1; }
    QString held() { QFile f(root+"/held"); if(!f.open(QIODevice::ReadOnly)) return {}; return QString::fromUtf8(f.readAll()); }
    void release() { put("release-"+held()); }
    void request(std::function<void()> next) {
        if(!click(load())) return;
        const auto id=expected();
        if(!require(id>0 && rows()==0,"request clears old display")) return;
        wait([this,id]{return replyId()==id;},[this,next]{
            require(rows()==state()["routing_catalog"].toObject()["items"].toArray().size(),"matched reply rendered");
            if(!failed) next();
        });
    }
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
        helperRow=row;
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
                hiddenBefore=trace();
                if(qEnvironmentVariable("A08_CASE")=="hidden-scan") {
                    QMetaObject::invokeMethod(bridge,"dispatch",Qt::DirectConnection,Q_ARG(QString,QString("{\"command\":\"routing_catalog\",\"processes\":false,\"query\":\"\",\"offset\":0}")));
                }
                // Hidden dwell: один deadline, без polling или GUI reads.
                QTimer::singleShot(500,this,[this]{ reopen(); });
            });
        });
    }
    void reopen() {
        open([this]{
            auto before=trace();
            require(samePolicy(),"reopen policy unchanged");
            auto* check=text(QString::fromUtf8("Запущенные процессы"));
            require(check && !check->property("checked").toBool(),"new picker defaults to desktop despite late process result");
            record("reopened_held",{{"catalog",state()["routing_catalog"]},{"processes_checked",check->property("checked").toBool()}});
            require(expected()==0 && rows()==0,"new Loader starts with no expected reply");
            release();
            QTimer::singleShot(500,this,[this,before]{
                auto after=trace();
                require(after.count("admitted")==hiddenBefore.count("admitted"),"close/reopen no NEW admission");
                require(after.count("read")==hiddenBefore.count("read"),"close/reopen no NEW IO");
                require(samePolicy(),"late callback policy unchanged");
                require(state()["routing_catalog"].toObject()["total"].toInt()==1,"actual delayed process reply after reopen");
                auto* stale=text(helperRow);
                if(!require(!stale || !stale->property("visible").toBool(),"stale process reply displayed after reopen")) return;
                record("stale_rejected",{{"reply",state()["routing_catalog"]},{"expected",expected()},{"rows",rows()},{"new_admissions",after.count("admitted")-hiddenBefore.count("admitted")},{"new_reads",after.count("read")-hiddenBefore.count("read")}});
                request([this]{
                    require(state()["routing_catalog"].toObject()["items"].toArray().size()>0,"new explicit desktop request");
                    record("desktop_after_reopen",{{"reply",state()["routing_catalog"]},{"expected",expected()}});
                    inputs();
                });
            });
        });
    }
    void inputs() {
        auto before=trace();
        field(w->contentItem())->setProperty("text","A08 marker");
        require(expected()==0 && rows()==0,"query change invalidates display");
        record("query_invalidated",{{"expected",expected()},{"rows",rows()}});
        require(trace().count("admitted")==before.count("admitted"),"query does not scan");
        request([this]{
            require(state()["routing_catalog"].toObject()["total"].toInt()==1,"query result");
            text(QString::fromUtf8("Запущенные процессы"))->setProperty("checked",true);
            require(expected()==0 && rows()==0,"mode change invalidates display");
            record("mode_invalidated",{{"expected",expected()},{"rows",rows()}});
            field(w->contentItem())->setProperty("text","a08-helper");
            request([this]{
                require(state()["routing_catalog"].toObject()["total"].toInt()==1,"mode result");
                record("input_invalidation");
                text(QString::fromUtf8("Запущенные процессы"))->setProperty("checked",false);
                field(w->contentItem())->setProperty("text","");
                request([this]{page();});
            });
        });
    }
    void page() {
        if(!click(item("routingCatalogNext"))) return;
        const auto id=expected();
        require(id>0 && rows()==0,"page change clears display");
        wait([this,id]{return replyId()==id;},[this]{
            require(state()["routing_catalog"].toObject()["offset"].toInt()==32 && rows()>0,"next page matched");
            record("page_correlated",{{"reply_id",replyId()},{"offset",state()["routing_catalog"].toObject()["offset"]},{"rows",rows()}});
            // A удержан, затем выдан более новый B. Реальный старый A приходит при expected B.
            field(w->contentItem())->setProperty("text","A08 marker");
            put("arm");
            const auto priorHeld=held();
            if(!click(load())) return;
            const auto a=expected();
            wait([this,priorHeld]{return held()!=priorHeld;},[this,a]{
                const auto aToken=held();
                field(w->contentItem())->setProperty("text","A08 wrapper");
                put("arm");
                if(!click(load())) return;
                const auto b=expected();
                require(b>a && rows()==0,"later request has fresh ID");
                release();
                wait([this,aToken,a]{return held()!=aToken && replyId()==a;},[this,a,b]{
                    require(expected()==b && rows()==0,"older reply after later request is not displayed");
                    record("older_reply_rejected",{{"older_reply",a},{"current_expected",b}});
                    release();
                    wait([this,b]{return replyId()==b;},[this]{
                        require(rows()==1,"later reply accepted");
                        legacy();
                    });
                });
            });
        });
    }
    void legacy() {
        const auto before=trace().count("released");
        const auto sequence=w->property("catalogRequestSequence");
        QMetaObject::invokeMethod(bridge,"dispatch",Qt::DirectConnection,Q_ARG(QString,QString("{\"command\":\"routing_catalog\",\"processes\":false,\"query\":\"A08 marker\",\"offset\":0}")));
        wait([this,before]{return trace().count("released")>before && !state()["routing_catalog"].toObject().contains("request_id");},[this,sequence]{
            const auto page=state()["routing_catalog"].toObject();
            require(page.size()==4 && page.contains("items") && page.contains("total") && page.contains("offset") && page.contains("incomplete"),"legacy reply exact old shape");
            require(rows()==0,"legacy reply not displayed by new GUI");
            require(sequence==w->property("catalogRequestSequence"),"legacy does not allocate GUI ID");
            record("legacy_shape",{{"reply",page}});
            errors(0);
        });
    }
    void errors(int id) {
        const auto before=trace();
        const QJsonValue catalog=state().value("routing_catalog");
        QJsonObject command{{"command","routing_catalog"},{"processes",false},{"query",QString(257,'x')},{"offset",0}};
        if(id) command["request_id"]=id;
        QMetaObject::invokeMethod(bridge,"dispatch",Qt::DirectConnection,Q_ARG(QString,QString::fromUtf8(QJsonDocument(command).toJson(QJsonDocument::Compact))));
        wait([this]{return state()["error"]=="limit";},[this,before,catalog,id]{
            require(state()["routing_catalog"]==catalog,"error leaves previous reply shape/data unchanged");
            require(trace().count("admitted")==before.count("admitted"),"invalid query never enters producer");
            require(samePolicy(),"error preserves policy/revision");
            record("error_preserved",{{"request_id",id},{"error",state()["error"]},{"new_admissions",trace().count("admitted")-before.count("admitted")}});
            if(id) overflow(); else request([this]{ errors(7777); });
        });
    }
    void overflow() {
        // Только boundary fault injection: пропускаем недостижимое число действий,
        // не подменяем snapshot, expected ID, Catalog или producer.
        w->setProperty("catalogRequestSequence",9007199254740990.0);
        request([this]{
            require(replyId()==9007199254740991.0,"last safe ID roundtrip");
            const auto before=trace();
            if(!click(load())) return;
            require(expected()==0 && rows()==0,"overflow clears display");
            auto* limit=item("routingCatalogLimit");
            require(limit && limit->property("visible").toBool(),"overflow explained");
            if(!click(load())) return;
            QTimer::singleShot(200,this,[this,before]{
                require(trace().count("admitted")==before.count("admitted"),"overflow never dispatches/wraps");
                require(w->property("catalogRequestSequence").toDouble()==9007199254740991.0,"counter never wraps");
                require(samePolicy(),"all controls preserve policy/revision");
                if(failed) return;
                record("completed",{{"routing",state()["routing"]},{"revision",state()["routing_revision"]},{"last_safe_id",replyId()}});
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
