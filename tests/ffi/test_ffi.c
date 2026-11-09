/*
 * C test program for the diffyman C ABI (crates/diffyman-ffi).
 *
 *   test_ffi                          run the self-checks (statuses, NULL
 *                                     arguments, ownership, versions)
 *   test_ffi diff LEFT RIGHT OPTIONS  read both files, call diffyman_diff
 *                                     with the paths as names, print the
 *                                     document to stdout
 *   test_ffi paths LEFT RIGHT OPTIONS call diffyman_diff_paths, print the
 *                                     document to stdout
 *
 * OPTIONS is a file holding the options JSON. On failure the status and the
 * message go to stderr and the exit code is 10 + the status. tests/ffi/run.sh
 * builds this against the shared library and compares the printed documents
 * with the corpus (tests/ffi/check_corpus.py).
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "diffyman.h"

static int failures = 0;

#define CHECK(cond)                                                            \
    do {                                                                       \
        if (!(cond)) {                                                         \
            fprintf(stderr, "%s:%d: check failed: %s\n", __FILE__, __LINE__,   \
                    #cond);                                                    \
            failures++;                                                        \
        }                                                                      \
    } while (0)

/* Reads a whole file. Returns NULL on failure; *len gets the size. */
static unsigned char *read_file(const char *path, size_t *len) {
    FILE *f = fopen(path, "rb");
    if (f == NULL) {
        return NULL;
    }
    size_t cap = 4096, used = 0;
    unsigned char *buf = malloc(cap);
    while (buf != NULL) {
        size_t n = fread(buf + used, 1, cap - used, f);
        used += n;
        if (n == 0) {
            break;
        }
        if (used == cap) {
            unsigned char *bigger = realloc(buf, cap * 2);
            if (bigger == NULL) {
                free(buf);
                buf = NULL;
                break;
            }
            buf = bigger;
            cap *= 2;
        }
    }
    int failed = ferror(f);
    fclose(f);
    if (failed) {
        free(buf);
        return NULL;
    }
    *len = used;
    return buf;
}

/* Reads a text file as a NUL-terminated string. */
static char *read_text(const char *path) {
    size_t len = 0;
    unsigned char *data = read_file(path, &len);
    if (data == NULL) {
        return NULL;
    }
    char *text = malloc(len + 1);
    if (text != NULL) {
        memcpy(text, data, len);
        text[len] = '\0';
    }
    free(data);
    return text;
}

/* Prints the document, or the failure, and frees both strings. */
static int report(int status, char *json, char *error) {
    if (status == DIFFYMAN_OK && json != NULL && error == NULL) {
        fputs(json, stdout);
        diffyman_free_string(json);
        return 0;
    }
    fprintf(stderr, "status %d: %s\n", status, error != NULL ? error : "(no message)");
    diffyman_free_string(json);
    diffyman_free_string(error);
    return 10 + status;
}

static int run_diff(const char *left_path, const char *right_path, const char *options_path) {
    size_t left_len = 0, right_len = 0;
    unsigned char *left = read_file(left_path, &left_len);
    unsigned char *right = read_file(right_path, &right_len);
    char *options = read_text(options_path);
    if (left == NULL || right == NULL || options == NULL) {
        fprintf(stderr, "cannot read the inputs\n");
        free(left);
        free(right);
        free(options);
        return 2;
    }
    char *json = NULL, *error = NULL;
    /* The engine keeps only the final path component of each name. */
    int status = diffyman_diff(left, left_len, right, right_len, options, left_path, right_path,
                               &json, &error);
    free(left);
    free(right);
    free(options);
    return report(status, json, error);
}

static int run_paths(const char *left_path, const char *right_path, const char *options_path) {
    char *options = read_text(options_path);
    if (options == NULL) {
        fprintf(stderr, "cannot read %s\n", options_path);
        return 2;
    }
    char *json = NULL, *error = NULL;
    int status = diffyman_diff_paths(left_path, right_path, options, 0, &json, &error);
    free(options);
    return report(status, json, error);
}

static void self_checks(void) {
    char *json = NULL, *error = NULL;
    int status;

    /* Versions are static strings. */
    CHECK(strcmp(diffyman_schema_version(), "0.4.0") == 0);
    CHECK(strlen(diffyman_engine_version()) > 0);

    /* A plain diff: a document, no message, provenance names the C ABI. */
    status = diffyman_diff((const uint8_t *)"a\nb\n", 4, (const uint8_t *)"a\nc\n", 4,
                           "{\"context\": 1}", "left.txt", "dir/right.txt", &json, &error);
    CHECK(status == DIFFYMAN_OK);
    CHECK(error == NULL);
    CHECK(json != NULL && strstr(json, "\"status\":\"different\"") != NULL);
    CHECK(json != NULL && strstr(json, "\"binding\":\"c\"") != NULL);
    CHECK(json != NULL && strstr(json, "\"name\":\"right.txt\"") != NULL);
    diffyman_free_string(json);

    /* Empty inputs may be NULL; NULL options and names are the defaults. */
    json = error = NULL;
    status = diffyman_diff(NULL, 0, NULL, 0, NULL, NULL, NULL, &json, &error);
    CHECK(status == DIFFYMAN_OK);
    CHECK(json != NULL && strstr(json, "\"status\":\"identical\"") != NULL);
    diffyman_free_string(json);

    /* Invalid options: status 1, no document, the engine's message. */
    json = error = NULL;
    status = diffyman_diff((const uint8_t *)"a", 1, (const uint8_t *)"b", 1, "{\"bogus\": true}",
                           NULL, NULL, &json, &error);
    CHECK(status == DIFFYMAN_INVALID_OPTIONS);
    CHECK(json == NULL);
    CHECK(error != NULL && strncmp(error, "invalid options: ", 17) == 0);
    diffyman_free_string(error);

    /* Options that are not UTF-8 are invalid options too. */
    json = error = NULL;
    status = diffyman_diff(NULL, 0, NULL, 0, "{\"context\": \xff}", NULL, NULL, &json, &error);
    CHECK(status == DIFFYMAN_INVALID_OPTIONS);
    CHECK(json == NULL);
    diffyman_free_string(error);

    /* A NULL buffer with a length, a non-UTF-8 name: status 3. */
    json = error = NULL;
    status = diffyman_diff(NULL, 3, (const uint8_t *)"abc", 3, NULL, NULL, NULL, &json, &error);
    CHECK(status == DIFFYMAN_OTHER);
    CHECK(json == NULL);
    CHECK(error != NULL && strncmp(error, "invalid argument: ", 18) == 0);
    diffyman_free_string(error);

    json = error = NULL;
    status = diffyman_diff(NULL, 0, NULL, 0, NULL, "l\xe9" "ft", NULL, &json, &error);
    CHECK(status == DIFFYMAN_OTHER);
    CHECK(error != NULL && strstr(error, "left_name is not valid UTF-8") != NULL);
    diffyman_free_string(error);

    /* out_json must not be NULL; out_error may be. */
    error = NULL;
    status = diffyman_diff(NULL, 0, NULL, 0, NULL, NULL, NULL, NULL, &error);
    CHECK(status == DIFFYMAN_OTHER);
    CHECK(error != NULL && strcmp(error, "invalid argument: out_json is NULL") == 0);
    diffyman_free_string(error);

    json = NULL;
    status = diffyman_diff(NULL, 0, NULL, 0, "{\"bogus\": 1}", NULL, NULL, &json, NULL);
    CHECK(status == DIFFYMAN_INVALID_OPTIONS);
    CHECK(json == NULL);

    /* Paths: a missing file is status 2, a NULL path status 3. */
    json = error = NULL;
    status = diffyman_diff_paths("no/such/left.txt", "no/such/right.txt", NULL, 0, &json, &error);
    CHECK(status == DIFFYMAN_IO);
    CHECK(json == NULL);
    CHECK(error != NULL && strncmp(error, "cannot read ", 12) == 0);
    diffyman_free_string(error);

    json = error = NULL;
    status = diffyman_diff_paths(NULL, "right.txt", NULL, 0, &json, &error);
    CHECK(status == DIFFYMAN_OTHER);
    CHECK(error != NULL && strcmp(error, "invalid argument: left_path is NULL") == 0);
    diffyman_free_string(error);

    /* Freeing NULL is a no-op. */
    diffyman_free_string(NULL);
}

int main(int argc, char **argv) {
    if (argc == 5 && strcmp(argv[1], "diff") == 0) {
        return run_diff(argv[2], argv[3], argv[4]);
    }
    if (argc == 5 && strcmp(argv[1], "paths") == 0) {
        return run_paths(argv[2], argv[3], argv[4]);
    }
    if (argc != 1) {
        fprintf(stderr, "usage: %s [diff|paths LEFT RIGHT OPTIONS_FILE]\n", argv[0]);
        return 2;
    }
    self_checks();
    if (failures != 0) {
        fprintf(stderr, "%d checks failed\n", failures);
        return 1;
    }
    printf("self-checks passed (engine %s, schema %s)\n", diffyman_engine_version(),
           diffyman_schema_version());
    return 0;
}
