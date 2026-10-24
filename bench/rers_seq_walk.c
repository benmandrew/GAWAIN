/*
 * Random-walk counterexample search for one RERS sequential problem.
 *
 * Links against the same object as bench/rers_seq_explore.c (the program's
 * globals in the section `rers_state`). Each walk starts at the initial
 * state and feeds random valid inputs, tracked by a monitor for one
 * property pattern. An input that prints nothing is not a step of a run,
 * so the state is restored and another input drawn.
 *
 * When the monitor reaches a *bad point*, the walk searches breadth first
 * from the current program state for a cycle: an edge x -> y whose target
 * y is x itself or an ancestor of x in the search tree. The stem to y and
 * the loop y ... x -> y close a lasso u v^omega. At a finite violation any
 * cycle will do. At an open obligation (a response still owed) the search
 * skips every step that prints or reads an event that would discharge or
 * end it, so the loop keeps the obligation open forever.
 *
 * The walk prints `LASSO u... | v...`, input values only;
 * bench/rers_seq_walk.py recovers the outputs by replay and checks the
 * word against the formula with Spot, so a monitor slip can cost only a
 * missed counterexample.
 *
 * Events are coded as the input value for an input and 100 + value for an
 * output. Patterns (all of the shape G(trigger -> ...)):
 *   prec P Q S R   after P until Q, S never occurs before R (or Q).
 *                  Bad point: P ... S with no R or Q in between.
 *   chain N M T    after N, once some M follows, T follows that M.
 *                  Bad point: N, then M; the loop must avoid T.
 *   resp R M O X   after R until M, every O is followed by X before M.
 *                  Bad points: R ... O ... M with no X in between, or
 *                  R ... O with the loop avoiding X and M.
 *
 * Usage: walk SEED WALKS STEPS CYCLE_STATES KIND ARGS...
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
    if (nout < MAX_OUT) outs[nout++] = v;
}

static int errored;
void rers_error(int code) {
    (void)code;
    errored = 1;
}

static uint64_t rng;
static uint32_t rnd(void) {
    rng ^= rng << 13;
    rng ^= rng >> 7;
    rng ^= rng << 17;
    return (uint32_t)rng;
}

enum { PREC, CHAIN, RESP };
static int kind, a[4];

/* Monitor: f0 armed (prec, resp: after the trigger; chain: an N waits
 * for its M), f1 owed (chain: T; resp: X). feed returns 1 at a finite
 * violation. */
typedef struct {
    int f0, f1;
} Mon;

static int feed(Mon *m, int e) {
    switch (kind) {
    case PREC: /* P Q S R */
        if (m->f0) {
            if (e == a[3] || e == a[1]) m->f0 = 0;
            else if (e == a[2]) return 1;
        }
        if (e == a[0] && e != a[1]) m->f0 = 1;
        return 0;
    case CHAIN: /* N M T */
        if (e == a[2]) m->f1 = 0;
        if (e == a[1] && m->f0) {
            m->f1 = 1;
            m->f0 = 0;
        }
        if (e == a[0]) m->f0 = 1;
        return 0;
    default: /* RESP: R M O X */
        if (m->f0) {
            if (e == a[3]) m->f1 = 0;
            if (e == a[1]) {
                if (m->f1) return 1;
                m->f0 = 0;
            } else if (e == a[2]) {
                m->f1 = 1;
            }
        }
        if (e == a[0] && e != a[1]) m->f0 = 1;
        return 0;
    }
}

/* Events a loop at an open obligation must avoid. */
static int forbidden(int e) {
    if (kind == CHAIN) return e == a[2];
    if (kind == RESP) return e == a[3] || e == a[1];
    return 0;
}

static size_t width;
static char *live;
static int ninputs;

/* Run one input from the live state. Returns 1 when it is a step of a
 * run (prints something, no error). */
static int step(int in) {
    nout = 0;
    errored = 0;
    calculate_output(in);
    return nout && !errored;
}

static int step_forbidden(int in) {
    if (forbidden(in)) return 1;
    for (int k = 0; k < nout; k++)
        if (forbidden(100 + outs[k])) return 1;
    return 0;
}

/* Breadth-first cycle search from the live state. */
static char *arena;
static int32_t *parent, *via;
static uint32_t *table;
static size_t cap, tcap;

static uint64_t hash(const char *p) {
    uint64_t h = 1469598103934665603ull;
    for (size_t i = 0; i < width; i++) h = (h ^ (unsigned char)p[i]) * 1099511628211ull;
    return h;
}

static long lookup(const char *p, int insert, size_t *n) {
    uint64_t h = hash(p) & (tcap - 1);
    for (;;) {
        uint32_t e = table[h];
        if (!e) {
            if (!insert) return -1;
            memcpy(arena + *n * width, p, width);
            table[h] = (uint32_t)(*n + 1);
            return (long)(*n)++;
        }
        if (memcmp(arena + (e - 1) * width, p, width) == 0) return e - 1;
        h = (h + 1) & (tcap - 1);
    }
}

static int is_ancestor(long y, long x) {
    while (x >= 0) {
        if (x == y) return 1;
        x = parent[x];
    }
    return 0;
}

static void print_path(long from, long to) { /* inputs from `from` down to `to` */
    int buf[1 << 16];
    int n = 0;
    for (long k = to; k != from && n < (1 << 16); k = parent[k]) buf[n++] = via[k];
    while (n) printf(" %d", buf[--n]);
}

static int close_cycle(const int *trail, long len, int restrict_loop) {
    char *start = malloc(width), *save = malloc(width);
    memcpy(start, live, width);
    memset(table, 0, sizeof(uint32_t) * tcap);
    size_t n = 0;
    lookup(start, 1, &n);
    parent[0] = -1;
    int found = 0;
    for (size_t k = 0; k < n && !found; k++) {
        memcpy(save, arena + k * width, width);
        for (int j = 0; j < ninputs && !found; j++) {
            memcpy(live, save, width);
            int in = rers_input(j);
            if (!step(in)) continue;
            if (restrict_loop && step_forbidden(in)) continue;
            long y = lookup(live, 0, &n);
            if (y >= 0) {
                if (is_ancestor(y, (long)k)) {
                    printf("LASSO");
                    for (long t = 0; t < len; t++) printf(" %d", trail[t]);
                    print_path(0, y);
                    printf(" |");
                    print_path(y, (long)k);
                    printf(" %d\n", in);
                    fflush(stdout);
                    found = 1;
                }
            } else if (n < cap) {
                long z = lookup(live, 1, &n);
                parent[z] = (int32_t)k;
                via[z] = in;
            }
        }
    }
    memcpy(live, start, width);
    free(start);
    free(save);
    return found;
}

int main(int argc, char **argv) {
    if (argc < 7) {
        fprintf(stderr, "usage: walk SEED WALKS STEPS CYCLE_STATES KIND ARGS...\n");
        return 1;
    }
    rng = strtoull(argv[1], 0, 10) * 2654435761u + 1;
    long walks = atol(argv[2]), steps = atol(argv[3]);
    cap = (size_t)atol(argv[4]);
    kind = strcmp(argv[5], "prec") == 0 ? PREC : strcmp(argv[5], "chain") == 0 ? CHAIN : RESP;
    int nargs = kind == CHAIN ? 3 : 4;
    for (int k = 0; k < nargs; k++) a[k] = atoi(argv[6 + k]);
    width = __stop_rers_state - __start_rers_state;
    live = __start_rers_state;
    for (tcap = 1024; tcap < 2 * cap; tcap *= 2) {
    }
    arena = malloc(width * cap);
    parent = malloc(sizeof(int32_t) * cap);
    via = malloc(sizeof(int32_t) * cap);
    table = malloc(sizeof(uint32_t) * tcap);
    char *init = malloc(width), *save = malloc(width);
    memcpy(init, live, width);
    ninputs = rers_input_count();
    int *trail = malloc(sizeof(int) * (steps + 1));
    long found = 0, bad = 0, tried = 0, total = 0, stuck = 0, armed = 0, owed = 0;
    for (long w = 0; w < walks && found < 1; w++) {
        memcpy(live, init, width);
        Mon m = {0, 0};
        long len = 0;
        for (long s = 0; s < steps; s++) {
            memcpy(save, live, width);
            int stepped = 0, violated = 0;
            int first = rnd() % ninputs;
            for (int tries = 0; tries < ninputs && !stepped; tries++) {
                int in = rers_input((first + tries) % ninputs);
                if (!step(in)) {
                    memcpy(live, save, width);
                    continue;
                }
                stepped = 1;
                trail[len++] = in;
                violated = feed(&m, in);
                for (int k = 0; k < nout && !violated; k++) violated = feed(&m, 100 + outs[k]);
            }
            if (!stepped) {
                stuck++;
                break;
            }
            total++;
            armed += m.f0;
            owed += m.f1;
            if (violated) {
                bad++;
                tried++;
                found += close_cycle(trail, len, 0);
                break;
            }
            /* An open obligation: try to keep it open forever, now and then. */
            if (kind != PREC && m.f1 && (rnd() % 16) == 0) {
                bad++;
                tried++;
                if (close_cycle(trail, len, 1)) {
                    found++;
                    break;
                }
            }
        }
    }
    printf("END bad_points=%ld cycle_searches=%ld found=%ld steps=%ld stuck=%ld armed=%ld "
           "owed=%ld\n", bad, tried, found, total, stuck, armed, owed);
    return 0;
}
