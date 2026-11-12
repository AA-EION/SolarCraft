/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "solarcraft.h"
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
static int32_t on_progress(double f, void *u) { (*(int *)u)++; (void)f; return 1; }
int main(void) {
    printf("%s\n", sc_version());
    ScQuickInput in = {4.711, -74.072, 2640.0, 10.0, 180.0, 10, 2026};
    ScAnnualSummary out; double hourly[8784]; int calls = 0;
    int32_t rc = sc_quick_annual(&in, &out, hourly, 8784, on_progress, &calls);
    printf("bogota rc=%d kwh=%.1f yield=%.1f kwp=%.2f steps=%u progress_calls=%d\n", rc, out.annual_ac_kwh, out.specific_yield_kwh_per_kwp, out.dc_kwp, out.steps, calls);
    size_t n = 131072; double *buf = malloc(n * sizeof(double));
    clock_t t = clock(); double s = sc_fill_probe(buf, n); double ms = 1000.0 * (clock() - t) / CLOCKS_PER_SEC;
    printf("probe 1MiB sum=%.0f ms=%.3f\n", s, ms);
    printf("panic probe rc=%d\n", sc_panic_probe());
    return (rc == SC_OK && sc_panic_probe() == SC_INTERNAL_ERROR && out.steps == 8760) ? 0 : 1;
}
