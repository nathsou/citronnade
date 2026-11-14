// A headless libretro frontend for running 3DS programs in Azahar's
// libretro core: it steps a fixed number of frames, scripts input, saves
// screenshots and prints the core's log (including the program's debug
// strings). Used to test the examples without a console.
//
// Build: tools/emu/build.sh (needs the Azahar checkout for libretro.h).
//
// Usage: emu-run --core azahar_libretro.so [options] program.3dsx
//   --frames N                 frames to run (default 120)
//   --shot F:file.ppm          save the frame shown at frame F
//   --press A-B:BTN[,BTN...]   hold buttons from frame A to B (inclusive)
//                              BTN: a b x y l r zl zr start select up down left right home
//   --touch A-B:X,Y            touch the bottom screen at (X,Y) from frame A to B
//   --stick A-B:X,Y            circle pad position (-32767..32767) from frame A to B
//   --option KEY=VALUE         a core option (citra_...)
//   --data DIR                 system/save directory (default target/emu-data)
//   --until-exit               stop when the program exits (default: keep running)

#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "libretro.h"

typedef struct { int from, to; unsigned mask; } Press;
typedef struct { int from, to; int x, y; } Point;
typedef struct { int frame; const char *path; } Shot;
typedef struct { char *key; char *value; } Option;

static Press presses[64];
static int npresses;
static Point touches[64];
static int ntouches;
static Point sticks[64];
static int nsticks;
static Shot shots[64];
static int nshots;
static Option options[64];
static int noptions;
static const char *data_dir = "target/emu-data";
static int frame;
static bool shutdown_requested;
static unsigned pixel_format = RETRO_PIXEL_FORMAT_0RGB1555;
static long audio_frames;
static int audio_peak;
static long audio_loud;

static const struct { const char *name; unsigned id; } buttons[] = {
    {"b", RETRO_DEVICE_ID_JOYPAD_B},       {"y", RETRO_DEVICE_ID_JOYPAD_Y},
    {"select", RETRO_DEVICE_ID_JOYPAD_SELECT}, {"start", RETRO_DEVICE_ID_JOYPAD_START},
    {"up", RETRO_DEVICE_ID_JOYPAD_UP},     {"down", RETRO_DEVICE_ID_JOYPAD_DOWN},
    {"left", RETRO_DEVICE_ID_JOYPAD_LEFT}, {"right", RETRO_DEVICE_ID_JOYPAD_RIGHT},
    {"a", RETRO_DEVICE_ID_JOYPAD_A},       {"x", RETRO_DEVICE_ID_JOYPAD_X},
    {"l", RETRO_DEVICE_ID_JOYPAD_L},       {"r", RETRO_DEVICE_ID_JOYPAD_R},
    {"zl", RETRO_DEVICE_ID_JOYPAD_L2},     {"zr", RETRO_DEVICE_ID_JOYPAD_R2},
    {"home", RETRO_DEVICE_ID_JOYPAD_L3},
};

static void die(const char *msg) {
    fprintf(stderr, "emu-run: %s\n", msg);
    exit(2);
}

static void log_cb(enum retro_log_level level, const char *fmt, ...) {
    static const char *names[] = {"debug", "info", "warn", "error"};
    char buf[4096];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(buf, sizeof buf, fmt, ap);
    va_end(ap);
    size_t n = strlen(buf);
    while (n && (buf[n - 1] == '\n' || buf[n - 1] == '\r')) buf[--n] = 0;
    printf("[%s] %s\n", level <= 3 ? names[level] : "?", buf);
    fflush(stdout);
}

static const char *option_value(const char *key) {
    for (int i = 0; i < noptions; i++)
        if (strcmp(options[i].key, key) == 0) return options[i].value;
    return NULL;
}

static bool environment(unsigned cmd, void *data) {
    switch (cmd & ~RETRO_ENVIRONMENT_EXPERIMENTAL) {
    case RETRO_ENVIRONMENT_GET_LOG_INTERFACE:
        ((struct retro_log_callback *)data)->log = log_cb;
        return true;
    case RETRO_ENVIRONMENT_GET_SYSTEM_DIRECTORY:
    case RETRO_ENVIRONMENT_GET_SAVE_DIRECTORY:
    case RETRO_ENVIRONMENT_GET_CORE_ASSETS_DIRECTORY:
        *(const char **)data = data_dir;
        return true;
    case RETRO_ENVIRONMENT_SET_PIXEL_FORMAT:
        pixel_format = *(const unsigned *)data;
        return true;
    case RETRO_ENVIRONMENT_GET_VARIABLE: {
        struct retro_variable *v = data;
        v->value = option_value(v->key);
        return v->value != NULL;
    }
    case RETRO_ENVIRONMENT_GET_VARIABLE_UPDATE:
        *(bool *)data = false;
        return true;
    case RETRO_ENVIRONMENT_GET_CAN_DUPE:
        *(bool *)data = true;
        return true;
    case RETRO_ENVIRONMENT_GET_CORE_OPTIONS_VERSION:
        *(unsigned *)data = 0;
        return true;
    case RETRO_ENVIRONMENT_SET_VARIABLES:
    case RETRO_ENVIRONMENT_SET_INPUT_DESCRIPTORS:
    case RETRO_ENVIRONMENT_SET_CONTROLLER_INFO:
    case RETRO_ENVIRONMENT_SET_SUPPORT_NO_GAME:
    case RETRO_ENVIRONMENT_SET_GEOMETRY:
    case RETRO_ENVIRONMENT_SET_SYSTEM_AV_INFO:
    case RETRO_ENVIRONMENT_SET_PERFORMANCE_LEVEL:
    case RETRO_ENVIRONMENT_SET_SUBSYSTEM_INFO:
    case RETRO_ENVIRONMENT_SET_MEMORY_MAPS:
    case RETRO_ENVIRONMENT_SET_SUPPORT_ACHIEVEMENTS:
        return true;
    case RETRO_ENVIRONMENT_GET_INPUT_BITMASKS:
        return false;
    case RETRO_ENVIRONMENT_SET_MESSAGE: {
        const struct retro_message *m = data;
        printf("[message] %s\n", m->msg);
        return true;
    }
    case RETRO_ENVIRONMENT_SET_MESSAGE_EXT: {
        const struct retro_message_ext *m = data;
        printf("[message] %s\n", m->msg);
        return true;
    }
    case RETRO_ENVIRONMENT_SHUTDOWN:
        shutdown_requested = true;
        return true;
    case RETRO_ENVIRONMENT_GET_AUDIO_VIDEO_ENABLE:
        *(int *)data = 3;
        return true;
    case RETRO_ENVIRONMENT_GET_FASTFORWARDING:
        *(bool *)data = false;
        return true;
    default:
        return false;
    }
}

static void save_frame(const void *data, unsigned w, unsigned h, size_t pitch, const char *path) {
    FILE *f = fopen(path, "wb");
    if (!f) {
        fprintf(stderr, "emu-run: cannot write %s\n", path);
        return;
    }
    fprintf(f, "P6\n%u %u\n255\n", w, h);
    for (unsigned y = 0; y < h; y++) {
        const uint8_t *row = (const uint8_t *)data + y * pitch;
        for (unsigned x = 0; x < w; x++) {
            uint8_t rgb[3];
            if (pixel_format == RETRO_PIXEL_FORMAT_XRGB8888) {
                uint32_t p = ((const uint32_t *)row)[x];
                rgb[0] = p >> 16, rgb[1] = p >> 8, rgb[2] = p;
            } else {
                uint16_t p = ((const uint16_t *)row)[x];
                if (pixel_format == RETRO_PIXEL_FORMAT_RGB565)
                    rgb[0] = (p >> 11) << 3, rgb[1] = ((p >> 5) & 63) << 2, rgb[2] = (p & 31) << 3;
                else
                    rgb[0] = ((p >> 10) & 31) << 3, rgb[1] = ((p >> 5) & 31) << 3, rgb[2] = (p & 31) << 3;
            }
            fwrite(rgb, 1, 3, f);
        }
    }
    fclose(f);
    printf("[harness] saved frame %d (%ux%u) to %s\n", frame, w, h, path);
}

static void video_cb(const void *data, unsigned w, unsigned h, size_t pitch) {
    if (!data || data == RETRO_HW_FRAME_BUFFER_VALID) return;
    for (int i = 0; i < nshots; i++)
        if (shots[i].frame == frame) save_frame(data, w, h, pitch, shots[i].path);
}

static void track_sample(int16_t v) {
    int a = v < 0 ? -(int)v : v;
    if (a > audio_peak) audio_peak = a;
    if (a > 256) audio_loud++;
}
static void audio_cb(int16_t l, int16_t r) { track_sample(l), track_sample(r), audio_frames++; }
static size_t audio_batch_cb(const int16_t *d, size_t n) {
    for (size_t i = 0; i < 2 * n; i++) track_sample(d[i]);
    audio_frames += n;
    return n;
}
static void input_poll_cb(void) {}

static int16_t input_state_cb(unsigned port, unsigned device, unsigned index, unsigned id) {
    if (port != 0) return 0;
    if (device == RETRO_DEVICE_JOYPAD) {
        for (int i = 0; i < npresses; i++)
            if (frame >= presses[i].from && frame <= presses[i].to && (presses[i].mask & (1u << id))) return 1;
        return 0;
    }
    if (device == RETRO_DEVICE_ANALOG && index == RETRO_DEVICE_INDEX_ANALOG_LEFT) {
        for (int i = 0; i < nsticks; i++)
            if (frame >= sticks[i].from && frame <= sticks[i].to)
                return id == RETRO_DEVICE_ID_ANALOG_X ? sticks[i].x : sticks[i].y;
        return 0;
    }
    if (device == RETRO_DEVICE_POINTER || device == RETRO_DEVICE_MOUSE) {
        for (int i = 0; i < ntouches; i++) {
            if (frame < touches[i].from || frame > touches[i].to) continue;
            // Pointer coordinates span the whole frame (-32767..32767); with
            // the default layout the bottom screen is the lower half,
            // 320 wide centred under the 400-wide top screen.
            int fx = 40 + touches[i].x, fy = 240 + touches[i].y;
            switch (id) {
            case RETRO_DEVICE_ID_POINTER_X: return (int16_t)(fx * 65534 / 400 - 32767);
            case RETRO_DEVICE_ID_POINTER_Y: return (int16_t)(fy * 65534 / 480 - 32767);
            case RETRO_DEVICE_ID_POINTER_PRESSED: return 1;
            default: return 0;
            }
        }
        return 0;
    }
    return 0;
}

static void parse_range(const char *s, int *from, int *to, const char **rest) {
    char *end;
    *from = (int)strtol(s, &end, 10);
    *to = *from;
    if (*end == '-') *to = (int)strtol(end + 1, &end, 10);
    if (*end != ':') die("expected FROM-TO:...");
    *rest = end + 1;
}

int main(int argc, char **argv) {
    const char *core = getenv("AZAHAR_LIBRETRO");
    const char *program = NULL;
    int frames = 120;
    bool until_exit = false;

    // Defaults: the software renderer and the standard layout.
    static char *defaults[][2] = {
        {"citra_graphics_api", "Software"},  {"citra_layout_option", "default"},
        {"citra_use_cpu_jit", "enabled"},    {"citra_use_virtual_sd", "enabled"},
        {"citra_is_new_3ds", "disabled"},    {"citra_enable_touch_touchscreen", "enabled"},
        {"citra_enable_mouse_touchscreen", "disabled"}, {"citra_audio_emulation", "hle"},
    };
    for (size_t i = 0; i < sizeof defaults / sizeof *defaults; i++)
        options[noptions++] = (Option){defaults[i][0], defaults[i][1]};

    for (int i = 1; i < argc; i++) {
        const char *a = argv[i];
        const char *v = i + 1 < argc ? argv[i + 1] : NULL;
        if (!strcmp(a, "--core") && v) core = v, i++;
        else if (!strcmp(a, "--frames") && v) frames = atoi(v), i++;
        else if (!strcmp(a, "--data") && v) data_dir = v, i++;
        else if (!strcmp(a, "--until-exit")) until_exit = true;
        else if (!strcmp(a, "--shot") && v) {
            char *colon = strchr(v, ':');
            if (!colon) die("--shot F:path");
            shots[nshots++] = (Shot){atoi(v), colon + 1};
            i++;
        } else if (!strcmp(a, "--press") && v) {
            Press p = {0};
            const char *rest;
            parse_range(v, &p.from, &p.to, &rest);
            char *list = strdup(rest), *save, *tok;
            for (tok = strtok_r(list, ",", &save); tok; tok = strtok_r(NULL, ",", &save)) {
                bool found = false;
                for (size_t b = 0; b < sizeof buttons / sizeof *buttons; b++)
                    if (!strcmp(tok, buttons[b].name)) p.mask |= 1u << buttons[b].id, found = true;
                if (!found) die("unknown button");
            }
            presses[npresses++] = p;
            i++;
        } else if ((!strcmp(a, "--touch") || !strcmp(a, "--stick")) && v) {
            Point p;
            const char *rest;
            parse_range(v, &p.from, &p.to, &rest);
            if (sscanf(rest, "%d,%d", &p.x, &p.y) != 2) die("expected X,Y");
            if (a[2] == 't') touches[ntouches++] = p; else sticks[nsticks++] = p;
            i++;
        } else if (!strcmp(a, "--option") && v) {
            char *kv = strdup(v), *eq = strchr(kv, '=');
            if (!eq) die("--option KEY=VALUE");
            *eq = 0;
            bool replaced = false;
            for (int k = 0; k < noptions; k++)
                if (!strcmp(options[k].key, kv)) options[k].value = eq + 1, replaced = true;
            if (!replaced) options[noptions++] = (Option){kv, eq + 1};
            i++;
        } else if (a[0] != '-') program = a;
        else die("unknown option (see the comment at the top of run.c)");
    }
    if (!core) die("no core: pass --core or set AZAHAR_LIBRETRO");
    if (!program) die("no program given");
    mkdir(data_dir, 0755);

    void *lib = dlopen(core, RTLD_NOW | RTLD_LOCAL);
    if (!lib) die(dlerror());
#define SYM(name) typeof(&name) p_##name = (typeof(&name))dlsym(lib, #name); if (!p_##name) die("missing " #name)
    SYM(retro_set_environment); SYM(retro_set_video_refresh); SYM(retro_set_audio_sample);
    SYM(retro_set_audio_sample_batch); SYM(retro_set_input_poll); SYM(retro_set_input_state);
    SYM(retro_init); SYM(retro_load_game); SYM(retro_run); SYM(retro_unload_game); SYM(retro_deinit);
    SYM(retro_get_system_av_info); SYM(retro_set_controller_port_device);

    p_retro_set_environment(environment);
    p_retro_set_video_refresh(video_cb);
    p_retro_set_audio_sample(audio_cb);
    p_retro_set_audio_sample_batch(audio_batch_cb);
    p_retro_set_input_poll(input_poll_cb);
    p_retro_set_input_state(input_state_cb);
    p_retro_init();
    p_retro_set_controller_port_device(0, RETRO_DEVICE_JOYPAD);

    struct retro_game_info game = {.path = program};
    if (!p_retro_load_game(&game)) die("the core could not load the program");
    struct retro_system_av_info av;
    p_retro_get_system_av_info(&av);
    printf("[harness] running %s: %ux%u at %.2f fps\n", program, av.geometry.base_width,
           av.geometry.base_height, av.timing.fps);

    for (frame = 0; frame < frames; frame++) {
        p_retro_run();
        if (shutdown_requested) {
            printf("[harness] the core shut down at frame %d\n", frame);
            if (until_exit) break;
        }
    }
    printf("[harness] ran %d frames, %ld audio frames\n", frame, audio_frames);
    printf("[harness] audio peak %d, %ld loud samples\n", audio_peak, audio_loud);
    fflush(stdout);
    p_retro_unload_game();
    p_retro_deinit();
    // The core's threads may not wind down cleanly; skip global destructors.
    _exit(0);
}
