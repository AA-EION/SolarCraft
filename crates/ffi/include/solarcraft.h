/* SPDX-License-Identifier: GPL-3.0-or-later */
/* SolarCraft core C ABI. All functions are thread-safe and never unwind. */
#ifndef SOLARCRAFT_H
#define SOLARCRAFT_H
#include <stdint.h>
#include <stddef.h>
#ifdef __cplusplus
extern "C" {
#endif

enum { SC_OK = 0, SC_CANCELLED = 1, SC_INVALID_ARGUMENT = 2, SC_INTERNAL_ERROR = 3 };

typedef struct {
    double latitude, longitude, elevation_m;
    double tilt_deg, azimuth_deg;   /* azimuth clockwise from north */
    uint32_t modules;               /* modules in one string */
    uint32_t year;
} ScQuickInput;

typedef struct {
    double annual_ac_kwh;
    double specific_yield_kwh_per_kwp;
    double dc_kwp;
    uint32_t steps;
} ScAnnualSummary;

/* Progress callback: fraction in [0,1]; return 0 to cancel. */
typedef int32_t (*ScProgressFn)(double fraction, void *user);

const char *sc_version(void);
/* Hourly clear-sky simulation (spike). hourly_ac_w may be NULL or point to >= 8784 doubles. */
int32_t sc_quick_annual(const ScQuickInput *in, ScAnnualSummary *out, double *hourly_ac_w, size_t hourly_len,
                        ScProgressFn progress, void *user);
/* FFI throughput probe: fills buf[i] = i * 0.5 and returns the sum. */
double sc_fill_probe(double *buf, size_t len);
/* Deliberately panics inside the core; must return SC_INTERNAL_ERROR, not abort. */
int32_t sc_panic_probe(void);

#ifdef __cplusplus
}
#endif
#endif
