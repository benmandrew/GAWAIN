/*
 * Explicit-state explorer for one RERS sequential problem (2016, 2017).
 *
 * bench/rers_seq.py compiles the problem's C file into an object whose
 * writable data section is renamed `rers_state`, so that section holds every
 * global of the program and nothing else. This harness links against it,
 * treats the section's bytes as the program state, and explores breadth
 * first: for every reached state and every valid input it restores the
 * state, calls calculate_output and records the outputs and the new state.
 *
 * Output, one line per (state, input) pair: `SRC INPUT DST OUT...`, where
 * OUT are the values printed in that step (none for an invalid input).
 * States are numbered in discovery order and state 0 is the initial one.
 * The last line is `END states=N` when the exploration is complete, or
 * `LIMIT states=N` when it stopped at --max-states.
 *
 * With `replay`, it instead reads input values from stdin, runs them from
 * the initial state, and prints one line per input: the outputs, `|`, and
 * the state after the step in hex. bench/rers_seq.py uses this to check
 * that a lasso from the official solutions is a real run.
 *
 * Usage: explore MAX_STATES [count|dump] > edges.txt
 *        explore replay < inputs.txt
 * With `count`, only the END/LIMIT line is printed. With `dump`, each
 * state's edges are preceded by `S K HEX`, its number and bytes in hex;
 * bench/rers_seq_abs.py uses this to check its abstraction. Progress goes to stderr
 * every million states.
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

extern char __start_rers_state[], __stop_rers_state[];
void calculate_output(int);
int rers_input_count(void);
int rers_input(int);

#define MAX_OUT 64
static int outs[MAX_OUT];
static int nout;

void rers_out(int v) {
    if (nout == MAX_OUT) {
        fprintf(stderr, "more than %d outputs in one step\n", MAX_OUT);
        exit(2);
    }
    outs[nout++] = v;
}

void rers_error(int code) {
    fprintf(stderr, "__VERIFIER_error(%d) reached\n", code);
    exit(2);
}

static size_t width;      /* bytes per state */
static char *arena;       /* states, back to back */
static size_t count, arena_cap;
static uint32_t *table;   /* open addressing: state number + 1, 0 is empty */
static size_t table_cap;

static uint64_t hash(const char *p) {
    uint64_t h = 0x9e3779b97f4a7c15ull;
    size_t i = 0;
    for (; i + 8 <= width; i += 8) {
        uint64_t w;
        memcpy(&w, p + i, 8);
        h = (h ^ w) * 0xff51afd7ed558ccdull;
        h ^= h >> 32;
    }
    for (; i < width; i++)
        h = (h ^ (unsigned char)p[i]) * 0x100000001b3ull;
    return h ^ (h >> 29);
}

static void grow_table(void) {
    size_t cap = table_cap ? table_cap * 2 : 1 << 16;
    uint32_t *t = calloc(cap, sizeof *t);
    if (!t) { perror("calloc"); exit(2); }
    for (size_t k = 0; k < count; k++) {
        size_t i = hash(arena + k * width) & (cap - 1);
        while (t[i]) i = (i + 1) & (cap - 1);
        t[i] = (uint32_t)(k + 1);
    }
    free(table);
    table = t;
    table_cap = cap;
}

/* The number of state `p`, adding it if it is new. */
static size_t intern(const char *p) {
    if (2 * (count + 1) > table_cap) grow_table();
    size_t i = hash(p) & (table_cap - 1);
    while (table[i]) {
        size_t k = table[i] - 1;
        if (memcmp(arena + k * width, p, width) == 0) return k;
        i = (i + 1) & (table_cap - 1);
    }
    if (count == arena_cap) {
        arena_cap = arena_cap ? arena_cap * 2 : 1 << 12;
        arena = realloc(arena, arena_cap * width);
        if (!arena) { perror("realloc"); exit(2); }
    }
    memcpy(arena + count * width, p, width);
    table[i] = (uint32_t)(count + 1);
    return count++;
}

static int replay(char *live) {
    int v;
    while (scanf("%d", &v) == 1) {
        nout = 0;
        calculate_output(v);
        for (int o = 0; o < nout; o++) printf("%d ", outs[o]);
        putchar('|');
        for (size_t i = 0; i < width; i++) printf("%02x", (unsigned char)live[i]);
        putchar('\n');
    }
    return 0;
}

int main(int argc, char **argv) {
    if (argc > 1 && strcmp(argv[1], "replay") == 0) {
        width = (size_t)(__stop_rers_state - __start_rers_state);
        return replay(__start_rers_state);
    }
    size_t max_states = argc > 1 ? strtoull(argv[1], 0, 10) : 10000000;
    char *live = __start_rers_state;
    width = (size_t)(__stop_rers_state - __start_rers_state);
    char *scratch = malloc(width);
    int quiet = argc > 2 && strcmp(argv[2], "count") == 0;
    int dump = argc > 2 && strcmp(argv[2], "dump") == 0;
    int ninputs = rers_input_count();
    fprintf(stderr, "state width %zu bytes, %d inputs\n", width, ninputs);
    intern(live);
    for (size_t k = 0; k < count; k++) {
        if (k % 1000000 == 0 && k)
            fprintf(stderr, "expanded %zu of %zu\n", k, count);
        if (count > max_states) {
            printf("LIMIT states=%zu\n", count);
            return 3;
        }
        if (dump) {
            printf("S %zu ", k);
            for (size_t i = 0; i < width; i++)
                printf("%02x", (unsigned char)arena[k * width + i]);
            putchar('\n');
        }
        for (int j = 0; j < ninputs; j++) {
            memcpy(live, arena + k * width, width);
            nout = 0;
            calculate_output(rers_input(j));
            memcpy(scratch, live, width);
            size_t dst = intern(scratch);
            if (quiet) continue;
            printf("%zu %d %zu", k, rers_input(j), dst);
            for (int o = 0; o < nout; o++) printf(" %d", outs[o]);
            putchar('\n');
        }
    }
    printf("END states=%zu\n", count);
    return 0;
}
