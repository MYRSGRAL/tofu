#pragma once

// Приватный acceptance API: только gui+test-support, без QML execution.
#include <atomic>
#include <memory>
#include <QAction>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickWindow>
#include <QSocketNotifier>
#include <sys/socket.h>
#include <unistd.h>

namespace yonik {
class P01Observer final : public QObject {
public:
    P01Observer(int fd, QWindow* window, DesktopBridge* bridge, QAction* open, QAction* quit)
        : fd_(fd), window_(window), bridge_(bridge), open_(open), quit_(quit)
    {
        int type = 0;
        socklen_t length = sizeof(type);
        sockaddr_storage address{};
        socklen_t addressLength = sizeof(address);
        if (::getsockopt(fd_, SOL_SOCKET, SO_TYPE, &type, &length) != 0
            || type != SOCK_STREAM
            || ::getsockname(fd_, reinterpret_cast<sockaddr*>(&address), &addressLength) != 0
            || address.ss_family != AF_UNIX) return;
        auto* quick = qobject_cast<QQuickWindow*>(window_);
        if (!quick) return;
        QObject::connect(quick, &QQuickWindow::frameSwapped, this, [this] { ++frames_; }, Qt::DirectConnection);
        QObject::connect(window_, &QWindow::visibleChanged, this, [this](bool) { ++visibility_; });
        notifier_ = std::make_unique<QSocketNotifier>(fd_, QSocketNotifier::Read, this);
        QObject::connect(notifier_.get(), &QSocketNotifier::activated, this, [this] { receive(); });
    }
    bool valid() const { return bool(notifier_); }

private:
    void receive()
    {
        char data[128];
        const auto count = ::recv(fd_, data, sizeof(data), MSG_DONTWAIT);
        if (count <= 0) {
            notifier_->setEnabled(false);
            quit_->trigger();
            return;
        }
        input_.append(data, count);
        if (input_.size() > 128) { reject(); return; }
        int newline;
        while ((newline = input_.indexOf('\n')) >= 0) {
            const auto command = input_.left(newline);
            input_.remove(0, newline + 1);
            bool ok = true;
            if (command == "Open") open_->trigger();
            else if (command == "Hide") window_->close(); // Штатный CloseHandler проверяет tray.
            else if (command == "Quit") quit_->trigger();
            else if (command == "VisibleFrameCanary" && window_->isVisible()) qobject_cast<QQuickWindow*>(window_)->update();
            else if (command != "Snapshot") ok = false;
            const auto backend = QJsonDocument::fromJson(bridge_->getSnapshot_json().toUtf8()).object();
            const QJsonObject state{
                {"ready", backend.value("ready")},
                {"status", backend.value("status")},
                {"can_stop", backend.value("can_stop")},
                {"error", backend.value("error")},
                {"schema", 1}, {"command", QString::fromLatin1(command)}, {"ok", ok},
                {"visible", window_->isVisible()},
                {"frames", static_cast<qint64>(frames_.load(std::memory_order_relaxed))},
                {"visibility_events", static_cast<qint64>(visibility_)},
                {"ui_stats_delivery", QJsonValue::Null},
                {"statistics_poll", QJsonValue::Null},
                {"stats_support", "unsupported-source-has-no-statistics-path"}
            };
            const auto line = QJsonDocument(state).toJson(QJsonDocument::Compact) + '\n';
            // Не блокируем event loop, не копим неограниченную очередь ответов.
            if (::send(fd_, line.constData(), line.size(), MSG_DONTWAIT | MSG_NOSIGNAL) != line.size()) {
                reject(); return;
            }
        }
    }
    void reject() { notifier_->setEnabled(false); quit_->trigger(); }
    int fd_;
    QWindow* window_;
    DesktopBridge* bridge_;
    QAction* open_;
    QAction* quit_;
    // Render thread может испускать frameSwapped параллельно GUI thread.
    std::atomic<quint64> frames_{0};
    quint64 visibility_ = 0;
    QByteArray input_;
    std::unique_ptr<QSocketNotifier> notifier_;
};
}
