#pragma once
#include <QString>

namespace yonik {
int run_application(const QString& dataDir, bool smokeTest, const QString& screenshot,
                    const QString& script, const QString& trace);
}
