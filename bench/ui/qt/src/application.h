#pragma once
#include <cstdint>

namespace tofu_bench {
int run_application();
void request_frame(std::int32_t token);
void exit_application();
void resume_for_show();
}
