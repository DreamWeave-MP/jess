/*
 * The audio device: miniaudio's device layer, and nothing else of it.
 *
 * jess mixes, resamples, and schedules; miniaudio opens a playback device, runs the thread that
 * asks for audio, and follows the default device when it changes. This file is all jess asks of
 * it, behind a handle Rust holds, so no miniaudio structure crosses into Rust. Built only with
 * the audio-device feature (see build.rs).
 */

#define MA_NO_DECODING
#define MA_NO_ENCODING
#define MA_NO_RESOURCE_MANAGER
#define MA_NO_NODE_GRAPH
#define MA_NO_ENGINE
#define MA_NO_GENERATION
#define MINIAUDIO_IMPLEMENTATION
#include "miniaudio.h"

#include <stdio.h>
#include <stdlib.h>

/* Fills `frames` interleaved frames at `out` for the device's thread. */
typedef void (*jess_audio_render)(void *user, float *out, ma_uint32 frames);
/* Hears what happened to the device: a miniaudio notification type. */
typedef void (*jess_audio_notify)(void *user, int kind);

typedef struct jess_audio_device {
    ma_context context;
    ma_device device;
    jess_audio_render render;
    jess_audio_notify notify;
    void *user;
} jess_audio_device;

static void jess_audio_data(ma_device *device, void *output, const void *input, ma_uint32 frames) {
    jess_audio_device *owner = (jess_audio_device *)device->pUserData;
    (void)input;
    if (owner->render != NULL) {
        owner->render(owner->user, (float *)output, frames);
    }
}

static void jess_audio_notification(const ma_device_notification *notification) {
    jess_audio_device *owner = (jess_audio_device *)notification->pDevice->pUserData;
    if (owner->notify != NULL) {
        owner->notify(owner->user, (int)notification->type);
    }
}

static void jess_audio_error(char *error, size_t length, const char *what, ma_result result) {
    if (error != NULL && length > 0) {
        snprintf(error, length, "%s: %s", what, ma_result_description(result));
    }
}

/*
 * Opens the default playback device, or with `null_only` the null device, which plays to nowhere
 * in real time, at `rate` frames per second (0 for the device's own) with `channels` channels of
 * 32-bit floats, asking for periods of `period` frames (0 for miniaudio's choice). Nothing plays
 * until it is attached and started. NULL on failure, with the reason in `error`.
 */
jess_audio_device *jess_audio_device_open(ma_uint32 rate, ma_uint32 channels, ma_uint32 period, int null_only, char *error, size_t error_length) {
    ma_backend null_backend[] = {ma_backend_null};
    ma_device_config config;
    ma_result result;
    jess_audio_device *owner = (jess_audio_device *)calloc(1, sizeof(jess_audio_device));
    if (owner == NULL) {
        jess_audio_error(error, error_length, "opening the audio device", MA_OUT_OF_MEMORY);
        return NULL;
    }
    result = ma_context_init(null_only ? null_backend : NULL, null_only ? 1 : 0, NULL, &owner->context);
    if (result != MA_SUCCESS) {
        jess_audio_error(error, error_length, "starting the audio backend", result);
        free(owner);
        return NULL;
    }
    config = ma_device_config_init(ma_device_type_playback);
    config.playback.format = ma_format_f32;
    config.playback.channels = channels;
    config.sampleRate = rate;
    config.periodSizeInFrames = period;
    config.performanceProfile = ma_performance_profile_low_latency;
    config.dataCallback = jess_audio_data;
    config.notificationCallback = jess_audio_notification;
    config.pUserData = owner;
    result = ma_device_init(&owner->context, &config, &owner->device);
    if (result != MA_SUCCESS) {
        jess_audio_error(error, error_length, "opening the audio device", result);
        ma_context_uninit(&owner->context);
        free(owner);
        return NULL;
    }
    return owner;
}

/* Gives the device what to call; before it starts, so no call races it. */
void jess_audio_device_attach(jess_audio_device *owner, jess_audio_render render, jess_audio_notify notify, void *user) {
    owner->render = render;
    owner->notify = notify;
    owner->user = user;
}

int jess_audio_device_start(jess_audio_device *owner) { return ma_device_start(&owner->device) == MA_SUCCESS; }

int jess_audio_device_stop(jess_audio_device *owner) { return ma_device_stop(&owner->device) == MA_SUCCESS; }

/* Stops the device, waits for its thread to finish, and frees it. */
void jess_audio_device_close(jess_audio_device *owner) {
    ma_device_uninit(&owner->device);
    ma_context_uninit(&owner->context);
    free(owner);
}

ma_uint32 jess_audio_device_rate(const jess_audio_device *owner) { return owner->device.sampleRate; }

ma_uint32 jess_audio_device_channels(const jess_audio_device *owner) { return owner->device.playback.channels; }

ma_uint32 jess_audio_device_period(const jess_audio_device *owner) { return owner->device.playback.internalPeriodSizeInFrames; }

ma_uint32 jess_audio_device_periods(const jess_audio_device *owner) { return owner->device.playback.internalPeriods; }

const char *jess_audio_device_backend(const jess_audio_device *owner) { return ma_get_backend_name(owner->context.backend); }
