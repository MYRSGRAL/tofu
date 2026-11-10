#pragma once
#include <QString>

namespace yonik {
void copy_diagnostic_metadata(const QString& value);
int run_application(const QString& dataDir, bool smokeTest, const QString& screenshot,
                    const QString& script, const QString& trace);
}
