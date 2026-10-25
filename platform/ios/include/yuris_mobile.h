#ifndef YURIS_MOBILE_H
#define YURIS_MOBILE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The C interface of the YU-RIS runtime for app hosts (crates/yuris-mobile).
 *
 * A session runs one game: open it, give it a view to draw into, then call
 * yuris_step once per display frame (CADisplayLink) and pass the input on.
 * The engine draws with Metal (wgpu) into the view's CAMetalLayer. Sound
 * is pulled by the host: render yuris_audio_render from its audio callback.
 *
 * Every call except the yuris_audio_* ones belongs on the thread that
 * opened the session (the main thread). Strings are UTF-8 and borrowed
 * for the duration of the call. */

#define YURIS_ABI_VERSION 1u

typedef struct YurisSession YurisSession;
typedef struct YurisAudio YurisAudio;

typedef struct {
    uint32_t struct_size;          /* sizeof(YurisConfig) */
    const char *game_dir;          /* the game folder */
    const char *launcher;          /* a patch's packed executable, or NULL */
    const char *save_dir;          /* where saves go, or NULL for <game_dir>/save */
    uint32_t encoding;             /* 0 Shift-JIS, 1 GBK, 2 UTF-8 */
    int64_t utc_offset_ms;         /* local time offset, for the date the game reads */
} YurisConfig;

enum {
    YURIS_RUNNING = 0,
    YURIS_ENDED = 1,   /* the game's main script ended */
    YURIS_FAILED = 2,  /* see yuris_last_error */
};

enum {
    YURIS_POINTER_MOVE = 0,
    YURIS_POINTER_LEFT_DOWN = 1,
    YURIS_POINTER_LEFT_UP = 2,
    YURIS_POINTER_RIGHT_DOWN = 3,
    YURIS_POINTER_RIGHT_UP = 4,
};

uint32_t yuris_abi_version(void);

/* The last error on this thread, or "" when none. Valid until the next
 * call on this thread. */
const char *yuris_last_error(void);

/* Engine log records: level 1 error, 2 warn, 3 info, 4 debug, 5 trace;
 * target is the Rust module. Strings are valid during the call; any
 * thread. NULL stops them. */
typedef void (*yuris_log_callback)(int32_t level, const char *target, const char *message, void *context);
void yuris_set_log_callback(yuris_log_callback callback, void *context, int32_t max_level);

/* Fonts for the sessions opened after the call: a TTF, OTF or TTC file and
 * the PostScript or family name of the face in it (NULL or "" for the
 * first). The first font registered draws text whose font the game names
 * but nobody registered (MS Gothic...). Returns 1, or 0 when the file is
 * missing; a face that is not in the file fails yuris_open. */
int32_t yuris_register_font(const char *path, const char *face);
void yuris_clear_fonts(void);

/* Opens and boots a game. NULL on failure (see yuris_last_error). */
YurisSession *yuris_open(const YurisConfig *config);

/* The game's screen size in pixels (1152 x 720 for Erewhon). */
void yuris_screen_size(const YurisSession *session, uint32_t *width, uint32_t *height);

/* Draw into ui_view, a UIView whose layer is a CAMetalLayer, of
 * width x height drawable pixels. Call again after the size changed.
 * The view must outlive the session or yuris_detach_surface.
 * Returns 0, or -1 on failure (see yuris_last_error). */
int32_t yuris_set_surface(YurisSession *session, void *ui_view, uint32_t width_px, uint32_t height_px);
void yuris_detach_surface(YurisSession *session);

/* Advances the clock by dt_ms, runs the game's next frame and draws it.
 * Returns YURIS_RUNNING, YURIS_ENDED or YURIS_FAILED. */
int32_t yuris_step(YurisSession *session, uint32_t dt_ms);

/* The pointer, in drawable pixels of the surface (the frame is fitted
 * into it with black bars; this maps back). One of YURIS_POINTER_*. */
void yuris_pointer(YurisSession *session, int32_t phase, float x, float y);
/* Mouse wheel: positive notches up, negative down. */
void yuris_wheel(YurisSession *session, int32_t notches);
/* Windows virtual-key code: 0x0D Enter, 0x1B Escape, 0x11 Ctrl, 0x20 Space... */
void yuris_key(YurisSession *session, int32_t vk, int32_t down);

/* Sound: a reference to the session's mixer, usable from any thread and
 * after the session closed, until released. */
YurisAudio *yuris_audio(const YurisSession *session);
uint32_t yuris_audio_sample_rate(void);
/* Writes frames interleaved stereo float samples (2 * frames floats). */
void yuris_audio_render(YurisAudio *audio, float *out, uint32_t frames);
void yuris_audio_release(YurisAudio *audio);

void yuris_close(YurisSession *session);

#ifdef __cplusplus
}
#endif

#endif
