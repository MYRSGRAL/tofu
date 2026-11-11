// Отдельный offscreen QtTest runner для скомпилированного набора notices.
#include <QtQuickTest/quicktest.h>
#include <QResource>
#include <QByteArray>

int main(int argc, char **argv) {
    const auto resource = qgetenv("YONIK_NOTICES_TEST_RCC");
    if (resource.isEmpty() || !QResource::registerResource(QString::fromUtf8(resource)))
        return 2;
    return quick_test_main(argc, argv, "notices", nullptr);
}
