// SPDX-License-Identifier: MPL-2.0
// Standalone epoch-2 todo showcase. C11, POSIX and json-c 0.15+.
#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include <fcntl.h>
#include <json_object.h>
#include <json_tokener.h>
#include <poll.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#define LIMIT (1024 * 1024)
#define VERSION "runyte-experimental-2"
typedef struct json_object J;
enum Stage { NONE, CREATE, SHOW, PUBLISH };
static J *tasks, *candidate;
static bool filtered, candidate_filter, added, view_closed, uncertain;
static char view[257], revision[257], command_id[129], request_id[32];
static uint64_t serial, next_task = 2;
static int phase;
static enum Stage stage;
static double deadline;

static void fail(void) { exit(1); }
static J *parse(const char *text, size_t length) {
    struct json_tokener *tok = json_tokener_new_ex(128);
    if (!tok) fail();
    json_tokener_set_flags(tok, JSON_TOKENER_STRICT | JSON_TOKENER_VALIDATE_UTF8);
    J *result = json_tokener_parse_ex(tok, text, (int)length);
    bool valid = json_tokener_get_error(tok) == json_tokener_success &&
                 json_tokener_get_parse_end(tok) == length;
    json_tokener_free(tok);
    if (!valid || !result) fail();
    return result;
}
static J *literal(const char *text) { return parse(text, strlen(text)); }
static J *get(J *object, const char *key) {
    J *value = NULL;
    if (json_object_is_type(object, json_type_object)) json_object_object_get_ex(object, key, &value);
    return value;
}
static const char *string(J *value) {
    if (!json_object_is_type(value, json_type_string)) return "";
    const char *s = json_object_get_string(value);
    // Embedded NUL must never turn one token into a different C string.
    if (strlen(s) != (size_t)json_object_get_string_len(value)) return "";
    return s;
}
static const char *field(J *object, const char *key) { return string(get(object, key)); }
static void put(J *object, const char *key, J *value) {
    if (json_object_object_add(object, key, value) != 0) fail();
}
static void text(J *object, const char *key, const char *value) {
    J *s = json_object_new_string(value);
    if (!s) fail();
    put(object, key, s);
}
static void append(J *array, J *value) { if (json_object_array_add(array, value)) fail(); }
static bool contains(J *array, const char *value) {
    if (!json_object_is_type(array, json_type_array)) return false;
    for (size_t i = 0; i < json_object_array_length(array); i++)
        if (!strcmp(string(json_object_array_get_idx(array, i)), value)) return true;
    return false;
}
static double now(void) {
    struct timespec ts;
    if (clock_gettime(CLOCK_MONOTONIC, &ts)) fail();
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1000000000;
}
static void wait_for(int fd, short events, double end) {
    for (;;) {
        double left = end ? end - now() : 0;
        if (end && left <= 0) fail();
        int timeout = !end ? -1 : left <= 0 ? 0 : (int)(left * 1000);
        struct pollfd descriptor = {fd, events, 0};
        int result = poll(&descriptor, 1, timeout);
        if (result > 0) return;
        if (!result || errno != EINTR) fail();
    }
}
static void send(J *message) {
    const char *s = json_object_to_json_string_ext(message, JSON_C_TO_STRING_PLAIN);
    size_t size = strlen(s);
    if (size + 1 > LIMIT) fail();
    char *frame = malloc(size + 1);
    if (!frame) fail();
    memcpy(frame, s, size);
    frame[size++] = '\n';
    double end = now() + 2;
    for (size_t offset = 0; offset < size;) {
        wait_for(STDOUT_FILENO, POLLOUT, end);
        ssize_t count = write(STDOUT_FILENO, frame + offset, size - offset);
        if (count > 0) offset += (size_t)count;
        else if (!count || (errno != EAGAIN && errno != EINTR)) fail();
    }
    free(frame);
    json_object_put(message);
}
static void finish(const char *id, const char *code) {
    J *message = literal("{\"type\":\"response\"}");
    text(message, "id", id);
    if (code) {
        J *error = literal("{}");
        text(error, "code", code);
        text(error, "message", !strcmp(code, "stale") ? "Todo list changed; invoke the action again" :
             !strcmp(code, "busy") ? "A todo update is pending" :
             !strcmp(code, "unavailable") ? "Restart this plugin after an uncertain update" :
             !strcmp(code, "limit_exceeded") ? "Todo limit reached" : "Todo request refused");
        put(message, "error", error);
    } else put(message, "result", literal("{\"job\":null}"));
    send(message);
}
static J *model(J *items, bool only_unfinished) {
    J *result = literal("{\"purpose\":\"list\",\"rows\":[]}");
    text(result, "title", only_unfinished ? "Todo · C · unfinished" : "Todo · C");
    for (size_t i = 0; i < json_object_array_length(items); i++) {
        J *task = json_object_array_get_idx(items, i);
        bool done = json_object_get_boolean(get(task, "done"));
        if (only_unfinished && done) continue;
        J *row = literal("{}");
        char label[261];
        snprintf(label, sizeof(label), "%s %s", done ? "[x]" : "[ ]", field(task, "title"));
        text(row, "id", field(task, "id"));
        text(row, "text", label);
        text(row, "role", done ? "muted" : "ordinary");
        append(get(result, "rows"), row);
    }
    return result;
}
static void request(enum Stage next, J *params) {
    if (serial == UINT64_MAX) fail();
    snprintf(request_id, sizeof(request_id), "p:%llu", (unsigned long long)++serial);
    stage = next;
    J *message = literal("{\"type\":\"request\"}");
    text(message, "id", request_id);
    text(message, "method", next == CREATE ? "view.create" : next == SHOW ? "pane.show" : "view.publish");
    put(message, "params", params);
    send(message);
}
static void show(void) {
    J *params = literal("{}");
    text(params, "invocation", command_id);
    text(params, "view", view);
    request(SHOW, params);
}
static bool valid_title(const char *title) {
    size_t length = strlen(title);
    bool nonspace = false;
    if (!length || length > 256) return false;
    const unsigned char *s = (const unsigned char *)title;
    for (size_t i = 0; i < length; i++) {
        if (s[i] != ' ') nonspace = true;
        if (s[i] < 32 || s[i] == 127 ||
            (s[i] == 0xc2 && i + 1 < length && s[i+1] >= 0x80 && s[i+1] <= 0x9f) ||
            (s[i] == 0xe2 && i + 2 < length && s[i+1] == 0x80 && (s[i+2] == 0xa8 || s[i+2] == 0xa9))) return false;
    }
    return nonspace;
}
static void invoke(J *message) {
    const char *id = field(message, "id");
    J *context = get(message, "params");
    const char *command = field(context, "command");
    if (!*id || strlen(id) >= sizeof(command_id) || (stage && !strcmp(id, command_id))) fail();
    if (strcmp(field(message, "method"), "command.invoke")) { finish(id, "unsupported"); return; }
    if (stage) { finish(id, "busy"); return; }
    if (uncertain) { finish(id, "unavailable"); return; }
    bool opening = !strcmp(command, "open");
    if (opening) {
        if (strcmp(field(context, "context"), "workspace")) { finish(id, "invalid_argument"); return; }
    } else {
        if (strcmp(command, "add") && strcmp(command, "toggle") && strcmp(command, "remove") && strcmp(command, "filter")) {
            finish(id, "not_found"); return;
        }
        if (!*view || strcmp(field(context, "context"), "view") || strcmp(field(context, "view"), view) ||
            strcmp(field(context, "model_revision"), revision)) { finish(id, "stale"); return; }
    }
    if (json_object_deep_copy(tasks, &candidate, NULL)) fail();
    candidate_filter = filtered;
    added = !strcmp(command, "add");
    const char *error = NULL;
    if (added) {
        const char *title = field(get(context, "arguments"), "title");
        if (!valid_title(title)) error = "invalid_argument";
        else if (json_object_array_length(tasks) >= 100 || next_task >= (UINT64_C(1) << 53)) error = "limit_exceeded";
        else {
            J *task = literal("{\"done\":false}");
            char key[32];
            snprintf(key, sizeof(key), "task-%llu", (unsigned long long)next_task);
            text(task, "id", key); text(task, "title", title); append(candidate, task);
        }
    } else if (!strcmp(command, "filter")) candidate_filter = !filtered;
    else if (!opening) {
        J *rows = get(context, "rows");
        if (!json_object_is_type(rows, json_type_array) || !json_object_array_length(rows) || json_object_array_length(rows) > 100)
            error = "invalid_argument";
        else for (size_t i = 0; i < json_object_array_length(rows); i++) {
            const char *key = string(json_object_array_get_idx(rows, i));
            bool found = false;
            for (size_t j = 0; j < json_object_array_length(tasks); j++) {
                J *task = json_object_array_get_idx(tasks, j);
                if (!strcmp(key, field(task, "id")) && (!filtered || !json_object_get_boolean(get(task, "done")))) found = true;
            }
            for (size_t j = 0; j < i; j++) if (!strcmp(key, string(json_object_array_get_idx(rows, j)))) found = false;
            if (!found) { error = "invalid_argument"; break; }
        }
        if (!error) for (size_t i = json_object_array_length(candidate); i > 0; i--) {
            J *task = json_object_array_get_idx(candidate, i - 1);
            if (!contains(rows, field(task, "id"))) continue;
            if (!strcmp(command, "remove")) json_object_array_del_idx(candidate, i - 1, 1);
            else put(task, "done", json_object_new_boolean(!json_object_get_boolean(get(task, "done"))));
        }
    }
    if (error) { json_object_put(candidate); candidate = NULL; finish(id, error); return; }
    strcpy(command_id, id);
    view_closed = false;
    deadline = now() + 8;
    if (opening && *view) { show(); return; }
    J *params = literal("{}");
    put(params, "model", model(candidate, candidate_filter));
    if (!opening) { text(params, "view", view); text(params, "expected_revision", revision); }
    request(opening ? CREATE : PUBLISH, params);
}
static void receive(J *message) {
    if (!json_object_is_type(message, json_type_object)) fail();
    const char *type = field(message, "type");
    if (phase < 2) {
        if (strcmp(type, phase ? "registered" : "hello") || !contains(get(message, "capabilities"), "views")) fail();
        if (!phase) {
            if (strcmp(field(message, "version"), VERSION)) fail();
            send(literal("{\"type\":\"register\",\"version\":\"" VERSION "\",\"name\":\"Todo · C\","
                "\"required_capabilities\":[\"views\"],\"optional_capabilities\":[],\"commands\":["
                "{\"name\":\"open\",\"description\":\"Open todo list\",\"context\":\"workspace\"},"
                "{\"name\":\"add\",\"description\":\"Add a task\",\"context\":\"view\",\"arguments\":[{\"name\":\"title\",\"type\":\"string\"}]},"
                "{\"name\":\"toggle\",\"description\":\"Toggle selected tasks\",\"context\":\"view\",\"primary\":true},"
                "{\"name\":\"remove\",\"description\":\"Remove selected tasks\",\"context\":\"view\"},"
                "{\"name\":\"filter\",\"description\":\"Toggle unfinished-only filter\",\"context\":\"view\"}]}"));
        } else deadline = 0;
        phase++;
        return;
    }
    if (!strcmp(type, "request")) invoke(message);
    else if (!strcmp(type, "event")) {
        if (*view && !strcmp(field(message, "event"), "view.closed") && !strcmp(field(get(message, "data"), "view"), view)) {
            *view = *revision = '\0';
            if (stage) view_closed = true;
        }
    } else if (!strcmp(type, "response")) {
        J *result = get(message, "result"), *error = get(message, "error");
        if (!stage || strcmp(field(message, "id"), request_id) || (!!result == !!error)) fail();
        const char *code = NULL;
        if (error) {
            code = field(error, "code");
            J *allowed = literal("[\"stale\",\"closed\",\"busy\",\"context_changed\",\"no_frontend\",\"unavailable\",\"limit_exceeded\",\"timeout\",\"outcome_unknown\"]");
            if (!contains(allowed, code)) code = "unavailable";
            json_object_put(allowed);
            if (stage != SHOW && (!strcmp(code, "timeout") || !strcmp(code, "outcome_unknown"))) uncertain = true;
        } else if (view_closed) code = "stale";
        else if (stage != SHOW) {
            const char *v = field(result, "view"), *r = field(result, "revision");
            if (!*v || strlen(v) > 256 || !*r || strlen(r) > 256 ||
                (stage == PUBLISH && (strcmp(v, view) || !strcmp(r, revision)))) {
                uncertain = true; code = "outcome_unknown";
            } else if (stage == CREATE) {
                strcpy(view, v); strcpy(revision, r); show(); return;
            } else {
                json_object_put(tasks); tasks = candidate; candidate = NULL;
                filtered = candidate_filter; next_task += added; strcpy(revision, r);
            }
        }
        finish(command_id, code);
        json_object_put(candidate); candidate = NULL; stage = NONE; deadline = 0;
    } else fail();
}
int main(void) {
    int flags = fcntl(STDOUT_FILENO, F_GETFL);
    if (flags < 0 || fcntl(STDOUT_FILENO, F_SETFL, flags | O_NONBLOCK) < 0) fail();
    tasks = literal("[{\"id\":\"task-1\",\"title\":\"Read the plugin guide\",\"done\":false}]");
    deadline = now() + 8;
    char *buffer = malloc(LIMIT);
    if (!buffer) fail();
    size_t used = 0;
    char chunk[8192];
    for (;;) {
        wait_for(STDIN_FILENO, POLLIN, deadline);
        ssize_t count = read(STDIN_FILENO, chunk, sizeof(chunk));
        if (count < 0) { if (errno == EINTR) continue; fail(); }
        if (!count) { if (used) fail(); break; }
        for (ssize_t i = 0; i < count; i++) {
            buffer[used++] = chunk[i];
            if (chunk[i] == '\n') { J *message = parse(buffer, used); receive(message); json_object_put(message); used = 0; }
            else if (used == LIMIT) fail();
        }
    }
    free(buffer); json_object_put(tasks); json_object_put(candidate);
    return 0;
}
