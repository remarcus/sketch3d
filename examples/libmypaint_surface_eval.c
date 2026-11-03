/*
 * Standalone bounded libmypaint 1.6.1 evaluation. No application integration.
 * Build/run instructions and measured limits are recorded in docs/LIBMYPAINT_EVALUATION.md.
 */
#define _POSIX_C_SOURCE 200809L
#include <mypaint-brush.h>
#include <mypaint-brush-settings-gen.h>
#include <mypaint-surface.h>

#include <errno.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <time.h>

#define W 1024
#define H 1024
#define PIXEL_BYTES 4u
#define SURFACE_BYTES ((size_t)W * H * PIXEL_BYTES)
#define MAX_SAMPLES 256
#define RUNS 5
#define PI 3.14159265358979323846

typedef struct {
    MyPaintSurface api; /* Must stay first: callbacks cast from this pointer. */
    uint8_t *rgba;
    uint8_t *before;
    int width, height;
    int in_atomic, have_dirty;
    MyPaintRectangle dirty;
    uint64_t dabs, color_reads, drawn_pixels;
    uint64_t begin_calls, end_calls, cancel_calls, undo_calls;
    float min_aspect, max_aspect;
} EvalSurface;

static double now_ms(void) {
    struct timespec t;
    if (clock_gettime(CLOCK_MONOTONIC, &t) != 0) return 0.0;
    return (double)t.tv_sec * 1000.0 + (double)t.tv_nsec / 1e6;
}

static uint64_t hash_pixels(const uint8_t *p, size_t n) {
    uint64_t h = UINT64_C(1469598103934665603);
    for (size_t i = 0; i < n; ++i) { h ^= p[i]; h *= UINT64_C(1099511628211); }
    return h;
}

static void include_rect(EvalSurface *s, int x, int y, int w, int h) {
    if (w <= 0 || h <= 0) return;
    MyPaintRectangle r = {x, y, w, h};
    if (!s->have_dirty) { s->dirty = r; s->have_dirty = 1; return; }
    int x0 = s->dirty.x < r.x ? s->dirty.x : r.x;
    int y0 = s->dirty.y < r.y ? s->dirty.y : r.y;
    int x1 = s->dirty.x + s->dirty.width > r.x + r.width ? s->dirty.x + s->dirty.width : r.x + r.width;
    int y1 = s->dirty.y + s->dirty.height > r.y + r.height ? s->dirty.y + s->dirty.height : r.y + r.height;
    s->dirty = (MyPaintRectangle){x0, y0, x1 - x0, y1 - y0};
}

static void begin_atomic(MyPaintSurface *api) {
    EvalSurface *s = (EvalSurface *)api;
    memcpy(s->before, s->rgba, SURFACE_BYTES);
    s->in_atomic = 1;
    s->have_dirty = 0;
    ++s->begin_calls;
}

static void end_atomic(MyPaintSurface *api, MyPaintRectangle *roi) {
    EvalSurface *s = (EvalSurface *)api;
    if (roi) *roi = s->have_dirty ? s->dirty : (MyPaintRectangle){0, 0, 0, 0};
    s->in_atomic = 0;
    ++s->end_calls;
}

static void cancel_atomic(EvalSurface *s) {
    memcpy(s->rgba, s->before, SURFACE_BYTES);
    s->in_atomic = 0;
    s->have_dirty = 0;
    ++s->cancel_calls;
}

static void undo_last(EvalSurface *s) {
    memcpy(s->rgba, s->before, SURFACE_BYTES);
    s->have_dirty = 0;
    ++s->undo_calls;
}

static int draw_dab(MyPaintSurface *api, float x, float y, float radius,
                    float cr, float cg, float cb, float opaque, float hardness,
                    float eraser, float aspect, float angle, float lock_alpha,
                    float colorize) {
    (void)eraser; (void)lock_alpha; (void)colorize;
    EvalSurface *s = (EvalSurface *)api;
    if (!isfinite(x) || !isfinite(y) || !isfinite(radius) || radius <= 0.f ||
        !isfinite(aspect) || !isfinite(angle) || !isfinite(opaque)) return 0;
    if (aspect < 1.f) aspect = 1.f;
    if (aspect > 10.f) aspect = 10.f;
    if (hardness < 0.f) hardness = 0.f;
    if (hardness > 1.f) hardness = 1.f;
    if (opaque < 0.f) opaque = 0.f;
    if (opaque > 1.f) opaque = 1.f;
    float a = angle * (float)(PI / 180.0), cs = cosf(a), sn = sinf(a);
    float rx = radius * aspect, ry = radius;
    float ex = sqrtf(rx * rx * cs * cs + ry * ry * sn * sn);
    float ey = sqrtf(rx * rx * sn * sn + ry * ry * cs * cs);
    int x0 = (int)floorf(x - ex - 1.f), x1 = (int)ceilf(x + ex + 1.f);
    int y0 = (int)floorf(y - ey - 1.f), y1 = (int)ceilf(y + ey + 1.f);
    if (x0 < 0) x0 = 0; if (y0 < 0) y0 = 0;
    if (x1 >= s->width) x1 = s->width - 1; if (y1 >= s->height) y1 = s->height - 1;
    if (x0 > x1 || y0 > y1) return 0;
    ++s->dabs;
    if (aspect < s->min_aspect) s->min_aspect = aspect;
    if (aspect > s->max_aspect) s->max_aspect = aspect;
    int changed = 0;
    for (int py = y0; py <= y1; ++py) for (int px = x0; px <= x1; ++px) {
        float dx = (px + .5f) - x, dy = (py + .5f) - y;
        float lx = dx * cs + dy * sn, ly = -dx * sn + dy * cs;
        float q = sqrtf((lx * lx) / (rx * rx) + (ly * ly) / (ry * ry));
        if (q > 1.f) continue;
        float cov = q <= hardness || hardness >= .999f ? 1.f : (1.f - q) / (1.f - hardness);
        if (cov < 0.f) cov = 0.f; if (cov > 1.f) cov = 1.f;
        float sa = opaque * cov;
        size_t i = ((size_t)py * s->width + px) * PIXEL_BYTES;
        float da = s->rgba[i + 3] / 255.f;
        float oa = sa + da * (1.f - sa);
        if (oa <= 0.f) continue;
        float src[3] = {cr, cg, cb};
        for (int c = 0; c < 3; ++c) {
            float dst = s->rgba[i + c] / 255.f;
            float out = (src[c] * sa + dst * da * (1.f - sa)) / oa;
            if (out < 0.f) out = 0.f; if (out > 1.f) out = 1.f;
            s->rgba[i + c] = (uint8_t)lrintf(out * 255.f);
        }
        s->rgba[i + 3] = (uint8_t)lrintf(oa * 255.f);
        ++s->drawn_pixels; changed = 1;
    }
    include_rect(s, x0, y0, x1 - x0 + 1, y1 - y0 + 1);
    return changed;
}

static void get_color(MyPaintSurface *api, float x, float y, float radius,
                      float *r, float *g, float *b, float *a) {
    EvalSurface *s = (EvalSurface *)api;
    ++s->color_reads;
    int cx = (int)lrintf(x), cy = (int)lrintf(y), rad = (int)ceilf(fmaxf(radius, 1.f));
    int x0 = cx - rad, y0 = cy - rad, x1 = cx + rad, y1 = cy + rad;
    if (x0 < 0) x0 = 0; if (y0 < 0) y0 = 0;
    if (x1 >= s->width) x1 = s->width - 1; if (y1 >= s->height) y1 = s->height - 1;
    double sums[4] = {0, 0, 0, 0}; uint64_t count = 0;
    for (int py = y0; py <= y1; ++py) for (int px = x0; px <= x1; ++px) {
        float dx = (float)px - x, dy = (float)py - y;
        if (dx * dx + dy * dy > radius * radius) continue;
        size_t i = ((size_t)py * s->width + px) * PIXEL_BYTES;
        for (int c = 0; c < 4; ++c) sums[c] += s->rgba[i + c] / 255.0;
        ++count;
    }
    if (!count) { *r = *g = *b = *a = 0.f; return; }
    *r = (float)(sums[0] / count); *g = (float)(sums[1] / count);
    *b = (float)(sums[2] / count); *a = (float)(sums[3] / count);
}

static void destroy_surface(MyPaintSurface *api) {
    EvalSurface *s = (EvalSurface *)api;
    free(s->rgba); free(s->before); free(s);
}

static EvalSurface *surface_new(void) {
    EvalSurface *s = calloc(1, sizeof(*s));
    if (!s) return NULL;
    s->rgba = calloc(1, SURFACE_BYTES);
    s->before = malloc(SURFACE_BYTES);
    if (!s->rgba || !s->before) { free(s->rgba); free(s->before); free(s); return NULL; }
    s->width = W; s->height = H; s->min_aspect = INFINITY;
    s->api.draw_dab = draw_dab; s->api.get_color = get_color;
    s->api.begin_atomic = begin_atomic; s->api.end_atomic = end_atomic;
    s->api.destroy = destroy_surface; s->api.refcount = 1;
    return s;
}

static void apply_sample(MyPaintBrush *b, EvalSurface *s, int i, int count) {
    float t = (float)i / (float)(count - 1);
    float x = 120.f + t * 784.f;
    float y = 510.f + 172.f * sinf(t * 2.f * (float)PI) + 45.f * sinf(t * 6.f * (float)PI);
    float pressure = .18f + .78f * (.5f + .5f * sinf(t * 2.f * (float)PI - .7f));
    float xtilt = .36f * sinf(t * 2.f * (float)PI);
    float ytilt = .32f * cosf(t * 2.f * (float)PI);
    (void)mypaint_brush_stroke_to(b, &s->api, x, y, pressure, xtilt, ytilt, 1.0 / 120.0);
}

static double run_stroke(MyPaintBrush *b, EvalSurface *s, int samples) {
    mypaint_brush_new_stroke(b);
    mypaint_surface_begin_atomic(&s->api);
    double t0 = now_ms();
    for (int i = 0; i < samples; ++i) apply_sample(b, s, i, samples);
    double elapsed = now_ms() - t0;
    MyPaintRectangle roi;
    mypaint_surface_end_atomic(&s->api, &roi);
    return elapsed;
}

static int write_ppm(const char *path, const EvalSurface *s) {
    FILE *f = fopen(path, "wb"); if (!f) return 0;
    fprintf(f, "P6\n%d %d\n255\n", W / 2, H / 2);
    for (int y = 0; y < H; y += 2) for (int x = 0; x < W; x += 2) {
        unsigned sum[3] = {0, 0, 0};
        for (int dy = 0; dy < 2; ++dy) for (int dx = 0; dx < 2; ++dx) {
            size_t i = ((size_t)(y + dy) * W + (x + dx)) * 4;
            for (int c = 0; c < 3; ++c) sum[c] += s->rgba[i + c];
        }
        uint8_t rgb[3] = {(uint8_t)(sum[0] / 4), (uint8_t)(sum[1] / 4), (uint8_t)(sum[2] / 4)};
        if (fwrite(rgb, 1, 3, f) != 3) { fclose(f); return 0; }
    }
    return fclose(f) == 0;
}

static int cmp_double(const void *a, const void *b) {
    double x = *(const double *)a, y = *(const double *)b; return (x > y) - (x < y);
}

int main(int argc, char **argv) {
    const char *out = argc > 1 ? argv[1] : "verification/libmypaint-evaluation";
    char ppm_path[1024], json_path[1024];
    if (snprintf(ppm_path, sizeof(ppm_path), "%s/preview.ppm", out) >= (int)sizeof(ppm_path) ||
        snprintf(json_path, sizeof(json_path), "%s/report.json", out) >= (int)sizeof(json_path)) return 2;
    if (mkdir(out, 0777) != 0 && errno != EEXIST) {
        fprintf(stderr, "create output directory %s: %s\n", out, strerror(errno)); return 2;
    }
    FILE *probe = fopen(json_path, "wb");
    if (!probe) { fprintf(stderr, "create %s: %s\n", json_path, strerror(errno)); return 2; }
    fclose(probe);
    EvalSurface *s = surface_new(); MyPaintBrush *b = mypaint_brush_new();
    if (!s || !b) { fprintf(stderr, "surface/brush allocation failed\n"); return 2; }
    mypaint_brush_from_defaults(b);
    mypaint_brush_set_base_value(b, MYPAINT_BRUSH_SETTING_COLOR_H, .08f);
    mypaint_brush_set_base_value(b, MYPAINT_BRUSH_SETTING_COLOR_S, .9f);
    mypaint_brush_set_base_value(b, MYPAINT_BRUSH_SETTING_COLOR_V, .9f);
    mypaint_brush_set_base_value(b, MYPAINT_BRUSH_SETTING_RADIUS_LOGARITHMIC, log2f(13.f));
    mypaint_brush_set_base_value(b, MYPAINT_BRUSH_SETTING_HARDNESS, .62f);
    mypaint_brush_set_base_value(b, MYPAINT_BRUSH_SETTING_SMUDGE, .22f);
    mypaint_brush_set_mapping_n(b, MYPAINT_BRUSH_SETTING_ELLIPTICAL_DAB_RATIO, MYPAINT_BRUSH_INPUT_TILT_DECLINATION, 2);
    mypaint_brush_set_mapping_point(b, MYPAINT_BRUSH_SETTING_ELLIPTICAL_DAB_RATIO, MYPAINT_BRUSH_INPUT_TILT_DECLINATION, 0, 0.f, 1.f);
    mypaint_brush_set_mapping_point(b, MYPAINT_BRUSH_SETTING_ELLIPTICAL_DAB_RATIO, MYPAINT_BRUSH_INPUT_TILT_DECLINATION, 1, 90.f, 3.f);
    mypaint_brush_set_mapping_n(b, MYPAINT_BRUSH_SETTING_ELLIPTICAL_DAB_ANGLE, MYPAINT_BRUSH_INPUT_TILT_ASCENSION, 2);
    mypaint_brush_set_mapping_point(b, MYPAINT_BRUSH_SETTING_ELLIPTICAL_DAB_ANGLE, MYPAINT_BRUSH_INPUT_TILT_ASCENSION, 0, -180.f, 0.f);
    mypaint_brush_set_mapping_point(b, MYPAINT_BRUSH_SETTING_ELLIPTICAL_DAB_ANGLE, MYPAINT_BRUSH_INPUT_TILT_ASCENSION, 1, 180.f, 180.f);

    const uint64_t blank_hash = hash_pixels(s->rgba, SURFACE_BYTES);
    /* Explicit abort path: paint half a stroke, end callbacks, restore snapshot. */
    mypaint_brush_new_stroke(b); mypaint_surface_begin_atomic(&s->api);
    for (int i = 0; i < MAX_SAMPLES / 2; ++i) apply_sample(b, s, i, MAX_SAMPLES);
    MyPaintRectangle cancel_roi; mypaint_surface_end_atomic(&s->api, &cancel_roi);
    cancel_atomic(s);
    const uint64_t cancelled_hash = hash_pixels(s->rgba, SURFACE_BYTES);

    double timings[RUNS];
    for (int r = 0; r < RUNS; ++r) { timings[r] = run_stroke(b, s, MAX_SAMPLES); cancel_atomic(s); }
    qsort(timings, RUNS, sizeof(timings[0]), cmp_double);
    MyPaintRectangle committed_roi;
    double accepted_ms = run_stroke(b, s, MAX_SAMPLES);
    /* run_stroke already closed the transaction; its before image is our single undo record. */
    uint64_t painted_hash = hash_pixels(s->rgba, SURFACE_BYTES);
    if (!write_ppm(ppm_path, s)) { fprintf(stderr, "could not write preview\n"); return 2; }
    committed_roi = s->dirty;
    float sampled[4] = {0, 0, 0, 0};
    mypaint_surface_get_color(&s->api, 512.f, 512.f, 8.f, &sampled[0], &sampled[1], &sampled[2], &sampled[3]);
    uint64_t engine_color_reads = s->color_reads - 1;
    undo_last(s);
    uint64_t undone_hash = hash_pixels(s->rgba, SURFACE_BYTES);
    struct rusage usage; memset(&usage, 0, sizeof(usage)); getrusage(RUSAGE_SELF, &usage);
    const uint64_t rgba_bytes = SURFACE_BYTES;
    const uint64_t dirty_upload = (uint64_t)committed_roi.width * (uint64_t)committed_roi.height * 4u;
    double brush_time = mypaint_brush_get_total_stroke_painting_time(b);
    FILE *f = fopen(json_path, "wb"); if (!f) return 2;
    fprintf(f,
      "{\n  \"engine\":\"libmypaint 1.6.1\",\n  \"surface\":{\"width\":%d,\"height\":%d,\"format\":\"RGBA8\",\"texture_bytes\":%llu,\"undo_scratch_bytes\":%llu,\"cpu_pixel_storage_bytes\":%llu,\"memory_budget_bytes\":%u},\n"
      "  \"stroke\":{\"input_samples\":%d,\"timed_runs\":%d,\"elapsed_time_s\":%.6f,\"pressure_min\":0.18,\"pressure_max\":0.96,\"tilt_x_range\":[-0.36,0.36],\"tilt_y_range\":[-0.32,0.32],\"median_cpu_ms\":%.6f,\"accepted_cpu_ms\":%.6f,\"engine_stroke_time_s\":%.6f},\n"
      "  \"callbacks\":{\"dabs\":%llu,\"shaded_pixels\":%llu,\"engine_color_reads\":%llu,\"manual_color_sample_calls\":1,\"aspect_ratio_min\":%.4f,\"aspect_ratio_max\":%.4f,\"atomic_begin\":%llu,\"atomic_end\":%llu,\"cancel\":%llu,\"undo\":%llu},\n"
      "  \"dirty_rect\":{\"x\":%d,\"y\":%d,\"width\":%d,\"height\":%d,\"estimated_rgba8_upload_bytes\":%llu,\"estimated_full_upload_bytes\":%llu},\n"
      "  \"memory\":{\"peak_rss_kib\":%ld,\"gpu_upload_measured\":false,\"upload_estimate_only\":true},\n"
      "  \"color_sample\":[%.5f,%.5f,%.5f,%.5f],\n  \"hashes\":{\"blank\":\"%016llx\",\"after_cancel\":\"%016llx\",\"painted\":\"%016llx\",\"after_undo\":\"%016llx\",\"cancel_restored\":%s,\"undo_restored\":%s}\n}\n",
      W,H,(unsigned long long)rgba_bytes,(unsigned long long)rgba_bytes,(unsigned long long)(rgba_bytes*2),(unsigned)(32u*1024u*1024u),
      MAX_SAMPLES,RUNS,MAX_SAMPLES/120.0,timings[RUNS/2],accepted_ms,brush_time,
      (unsigned long long)s->dabs,(unsigned long long)s->drawn_pixels,(unsigned long long)engine_color_reads,s->min_aspect,s->max_aspect,
      (unsigned long long)s->begin_calls,(unsigned long long)s->end_calls,(unsigned long long)s->cancel_calls,(unsigned long long)s->undo_calls,
      committed_roi.x,committed_roi.y,committed_roi.width,committed_roi.height,(unsigned long long)dirty_upload,(unsigned long long)rgba_bytes,(long)usage.ru_maxrss,
      sampled[0],sampled[1],sampled[2],sampled[3],(unsigned long long)blank_hash,(unsigned long long)cancelled_hash,(unsigned long long)painted_hash,(unsigned long long)undone_hash,
      cancelled_hash==blank_hash?"true":"false",undone_hash==blank_hash?"true":"false");
    fclose(f);
    int tilt_exercised = s->max_aspect > 1.05f;
    printf("libmypaint 1.6.1: median CPU %.3f ms, dabs %llu, engine color reads %llu, dirty upload estimate %llu bytes; cancel/undo %s/%s\n",
      timings[RUNS/2],(unsigned long long)s->dabs,(unsigned long long)engine_color_reads,(unsigned long long)dirty_upload,
      cancelled_hash==blank_hash?"pass":"FAIL",undone_hash==blank_hash?"pass":"FAIL");
    mypaint_brush_unref(b); mypaint_surface_unref(&s->api);
    return cancelled_hash==blank_hash && undone_hash==blank_hash && painted_hash!=blank_hash && tilt_exercised && engine_color_reads > 0 ? 0 : 1;
}
