#include <dispatch/dispatch.h>
#include <os/log.h>
#include <os/signpost.h>
#include <stdbool.h>
#include <stdint.h>

static dispatch_once_t alpine_studio_log_once;
static os_log_t alpine_studio_log;

static void alpine_studio_initialize_log(void *context) {
    (void)context;
    alpine_studio_log = os_log_create(
        "com.dbuddha.alpine-studio",
        OS_LOG_CATEGORY_DYNAMIC_TRACING
    );
}

static os_log_t alpine_studio_get_log(void) {
    dispatch_once_f(
        &alpine_studio_log_once,
        NULL,
        alpine_studio_initialize_log
    );
    return alpine_studio_log;
}

bool alpine_studio_signposts_enabled(void) {
    return os_signpost_enabled(alpine_studio_get_log());
}

#define ALPINE_STUDIO_EMIT(name) \
    os_signpost_event_emit( \
        log, \
        correlation, \
        name, \
        "event=%{public}llu scene=%{public}llu document=%{public}llu " \
        "buffer=%{public}llu a=%{public}llu b=%{public}llu c=%{public}llu", \
        (unsigned long long)event_timestamp, \
        (unsigned long long)scene_revision, \
        (unsigned long long)document_revision, \
        (unsigned long long)buffer_revision, \
        (unsigned long long)value_a, \
        (unsigned long long)value_b, \
        (unsigned long long)value_c \
    )

void alpine_studio_signpost_emit(
    uint8_t stage,
    uint64_t correlation,
    uint64_t event_timestamp,
    uint64_t scene_revision,
    uint64_t document_revision,
    uint64_t buffer_revision,
    uint64_t value_a,
    uint64_t value_b,
    uint64_t value_c
) {
    os_log_t log = alpine_studio_get_log();
    switch (stage) {
        case 0:
            ALPINE_STUDIO_EMIT("Event Dispatch Begin");
            break;
        case 1:
            ALPINE_STUDIO_EMIT("State Mutation Complete");
            break;
        case 2:
            ALPINE_STUDIO_EMIT("Frame Build Begin");
            break;
        case 3:
            ALPINE_STUDIO_EMIT("Visible Layout Begin");
            break;
        case 4:
            ALPINE_STUDIO_EMIT("Visible Layout Complete");
            break;
        case 5:
            ALPINE_STUDIO_EMIT("Text Summary");
            break;
        case 6:
            ALPINE_STUDIO_EMIT("Layout Cache Summary");
            break;
        case 7:
            ALPINE_STUDIO_EMIT("Glyph Atlas Summary");
            break;
        case 8:
            ALPINE_STUDIO_EMIT("Atlas Publication Begin");
            break;
        case 9:
            ALPINE_STUDIO_EMIT("Atlas Publication Complete");
            break;
        case 10:
            ALPINE_STUDIO_EMIT("Atlas Publication Failed");
            break;
        case 11:
            ALPINE_STUDIO_EMIT("Frame Build Complete");
            break;
        case 12:
            ALPINE_STUDIO_EMIT("Frame Build Failed");
            break;
        case 13:
            ALPINE_STUDIO_EMIT("Native Event Handler Latency");
            break;
        case 14:
            ALPINE_STUDIO_EMIT("Native Frame Queue Latency");
            break;
        case 15:
            ALPINE_STUDIO_EMIT("Native Submission Latency");
            break;
        case 16:
            ALPINE_STUDIO_EMIT("Native GPU Terminal Observed Latency");
            break;
        case 17:
            ALPINE_STUDIO_EMIT("Native Presented Handler Latency");
            break;
        case 18:
            ALPINE_STUDIO_EMIT("Native Terminal Record Latency");
            break;
        default:
            break;
    }
}
