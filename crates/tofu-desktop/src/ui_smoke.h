#pragma once

#include <QCoreApplication>
#include <QDir>
#include <QElapsedTimer>
#include <QFileInfo>
#include <QHash>
#include <QJsonObject>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QQuickItem>
#include <QQuickWindow>
#include <QRegularExpression>
#include <QVariant>
#include <QWheelEvent>
#include <algorithm>
#include <cmath>

namespace tofu::smoke {
// Эти операции вызываются только из явно включённого --smoke-test.
inline QQuickItem* findItem(QQuickItem* root, const QString& name)
{
    if (!root) return nullptr;
    if (root->objectName() == name) return root;
    for (auto* child : root->childItems()) {
        if (auto* found = findItem(child, name)) return found;
    }
    return nullptr;
}

inline bool validName(const QString& name)
{
    static const QRegularExpression pattern(QStringLiteral("^[A-Za-z][A-Za-z0-9_]{0,63}$"));
    return pattern.match(name).hasMatch();
}

inline QObject* findObject(QQuickWindow* window, const QString& name)
{
    if (name == QStringLiteral("window")) return window;
    if (!validName(name)) return nullptr;
    if (auto* item = findItem(window->contentItem(), name)) return item;
    return window->findChild<QObject*>(name);
}

inline bool matches(QQuickWindow* window, const QJsonObject& expected)
{
    if (expected.size() > 32) return false;
    for (auto object = expected.begin(); object != expected.end(); ++object) {
        auto* target = findObject(window, object.key());
        if (!target || !object.value().isObject()) return false;
        const auto properties = object.value().toObject();
        if (properties.size() > 16) return false;
        for (auto property = properties.begin(); property != properties.end(); ++property) {
            if (!validName(property.key()) || property.value().isArray()) return false;
            const auto value = target->property(property.key().toLatin1().constData());
            if (!value.isValid()) return false;
            if (property.value().isObject()) {
                const auto condition = property.value().toObject();
                const auto fragment = condition.value(QStringLiteral("contains"));
                if (condition.size() != 1 || !fragment.isString() || fragment.toString().size() > 2048
                    || value.metaType().id() != QMetaType::QString || !value.toString().contains(fragment.toString())) return false;
            } else if (QJsonValue::fromVariant(value) != property.value()) return false;
        }
    }
    return true;
}

inline void reveal(QQuickItem* item)
{
    // Прокручиваем только принадлежащие этому окну Flickable-предки.
    for (auto* parent = item->parentItem(); parent; parent = parent->parentItem()) {
        if (parent->metaObject()->indexOfProperty("contentY") < 0) continue;
        const auto current = parent->property("contentY").toDouble();
        const auto maximum = std::max(0.0, parent->property("contentHeight").toDouble() - parent->height());
        const auto point = item->mapToItem(parent, QPointF(item->width() / 2, item->height() / 2));
        parent->setProperty("contentY", std::clamp(current + point.y() - parent->height() / 2, 0.0, maximum));
    }
}

inline bool perform(QQuickWindow* window, const QJsonObject& action)
{
    const auto kind = action.value(QStringLiteral("action")).toString();
    if (kind == QStringLiteral("resize")) {
        const auto width = action.value(QStringLiteral("width")).toInt();
        const auto height = action.value(QStringLiteral("height")).toInt();
        if (width < 520 || width > 1600 || height < 640 || height > 1400) return false;
        window->resize(width, height);
        window->update();
        return true;
    }
    auto* item = qobject_cast<QQuickItem*>(findObject(window, action.value(QStringLiteral("object")).toString()));
    if (!item || !item->isVisible() || !item->isEnabled()) return false;
    reveal(item);
    item->forceActiveFocus(Qt::OtherFocusReason);
    if (kind == QStringLiteral("wheel")) {
        const auto delta = action.value(QStringLiteral("delta"));
        const auto number = delta.toDouble();
        if (!delta.isDouble() || !std::isfinite(number) || number == 0
            || std::abs(number) > 1200 || number != std::floor(number)) return false;
        // Край scroll viewport: поле ввода/SpinBox в центре может принять колесо.
        const auto point = item->mapToScene(QPointF(std::min(4.0, item->width() / 2), item->height() / 2));
        if (item->width() <= 0 || item->height() <= 0 || point.x() < 0 || point.y() < 0
            || point.x() >= window->width() || point.y() >= window->height()) return false;
        QWheelEvent wheel(point, window->mapToGlobal(point), QPoint(), QPoint(0, int(number)),
                          Qt::NoButton, Qt::NoModifier, Qt::NoScrollPhase, false);
        // Flickable вычисляет скорость по timestamp; два нуля отбрасывают reverse.
        static QElapsedTimer wheelClock;
        if (!wheelClock.isValid()) wheelClock.start();
        wheel.setTimestamp(static_cast<ulong>(wheelClock.elapsed() + 1));
        QCoreApplication::sendEvent(window, &wheel);
        return true;
    }
    if (kind == QStringLiteral("click")) {
        const auto point = item->mapToScene(QPointF(item->width() / 2, item->height() / 2));
        if (item->width() <= 0 || item->height() <= 0 || point.x() < 0 || point.y() < 0
            || point.x() >= window->width() || point.y() >= window->height()) return false;
        QMouseEvent press(QEvent::MouseButtonPress, point, window->mapToGlobal(point),
                          Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
        // Qt 6.4 разделяет QEventPoint: release нельзя создавать до доставки press.
        QCoreApplication::sendEvent(window, &press);
        QMouseEvent release(QEvent::MouseButtonRelease, point, window->mapToGlobal(point),
                            Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
        QCoreApplication::sendEvent(window, &release);
        return true;
    }
    if (kind == QStringLiteral("key")) {
        const auto key = action.value(QStringLiteral("key")).toString();
        const auto keys = QHash<QString, int>{{"Enter", Qt::Key_Return}, {"Escape", Qt::Key_Escape},
            {"Space", Qt::Key_Space}, {"Tab", Qt::Key_Tab}, {"Down", Qt::Key_Down}, {"Up", Qt::Key_Up}};
        if (!keys.contains(key)) return false;
        QKeyEvent press(QEvent::KeyPress, keys.value(key), Qt::NoModifier);
        QKeyEvent release(QEvent::KeyRelease, keys.value(key), Qt::NoModifier);
        QCoreApplication::sendEvent(window, &press);
        QCoreApplication::sendEvent(window, &release);
        return true;
    }
    if (kind == QStringLiteral("text")) {
        const auto text = action.value(QStringLiteral("value"));
        if (!text.isString() || text.toString().size() > 2048
            || item->metaObject()->indexOfProperty("text") < 0) return false;
        return item->setProperty("text", text.toString());
    }
    if (kind == QStringLiteral("value")) {
        const auto value = action.value(QStringLiteral("value"));
        const auto number = value.toDouble(-1);
        if (!value.isDouble() || !std::isfinite(number) || number < 0 || number > 65535
            || number != std::floor(number) || item->metaObject()->indexOfProperty("value") < 0) return false;
        return item->setProperty("value", int(number));
    }
    return false;
}

inline QString capturePath(const QString& base, const QJsonObject& step)
{
    if (!step.contains(QStringLiteral("capture_name"))) return base;
    const auto name = step.value(QStringLiteral("capture_name")).toString();
    static const QRegularExpression pattern(QStringLiteral("^[A-Za-z0-9_-]{1,60}\\.png$"));
    if (base.isEmpty() || !pattern.match(name).hasMatch()) return {};
    return QFileInfo(base).dir().filePath(name);
}
}
