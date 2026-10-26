/*
 * Decide whether a RERS sequential program can print one output value, by
 * explicit breadth-first search that stores only the mutable bytes of each
 * state, tree-compressed.
 *
 * It links against the same object as bench/rers_seq_explore.c, whose
 * writable data section `rers_state` holds every global. Most of that
 * section is constant arrays, so a state is stored as the bytes of the
 * segments listed in MASK (lines `OFFSET LENGTH`, written by
 * bench/rers_seq_reach.py from the program's scalar and pointer globals).
 * After every step the harness checks that no byte outside the segments
 * changed, and stops with exit 4 if one did, so the reduction is exact.
 *
 * The stored bytes are tree-compressed, as in the state stores of SPIN and
 * LTSmin: they are cut into 16-byte leaves, level 0 interns the leaves,
 * and level l + 1 interns nodes of four level-l indices, up to a single
 * root. Every level has its own table, so a state is new exactly when its
 * root node is, and root indices number the states in discovery order.
 *
 * An input that prints nothing is invalid and ends the run, as in
 * bench/rers_seq.py, so its successor is not explored.
 *
 * Output: `FOUND depth=D inputs=V1 V2 ...` when some step prints TARGET
 * (the input values of a shortest run that prints it), `END states=N`
 * when the search completes without it, or `LIMIT states=N`. FOUND means
 * a finite run prints TARGET; whether an infinite run does is checked
 * separately. Progress goes to stderr every million states.
 *
 * Usage: reach MASK TARGET MAX_STATES
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

extern char __start_rers_state[], __stop_rers_state[];
void calculate_output(int);
int rers_input_count(void);
int rers_input(int);

static int hit, target, nout;

void rers_out(int v) {
    nout++;
    if (v == target) hit = 1;
}

void rers_error(int code) {
    fprintf(stderr, "__VERIFIER_error(%d) reached\n", code);
    exit(2);
}

static size_t nseg, width, full;
static size_t *seg_off, *seg_len;
static char *init;
static uint32_t *parent;
static uint8_t *via;
static size_t max_states;

#define MAX_LEVELS 16
typedef struct { uint32_t w[4]; } node;
typedef struct {
    node *nodes;
    size_t count, cap;
    uint32_t *table;  /* open addressing: node index + 1, 0 is empty */
    size_t table_cap;
    size_t slots;     /* nodes of this level in one state */
} level;
static level lv[MAX_LEVELS];
static int nlevels;
static uint32_t *work[MAX_LEVELS];

static void die(const char *why, int code) {
    fprintf(stderr, "%s\n", why);
    exit(code);
}

static uint64_t hash(const node *n) {
    uint64_t h = 1469598103934665603ULL;
    for (int i = 0; i < 4; i++) {
        h = (h ^ n->w[i]) * 1099511628211ULL;
        h ^= h >> 29;
    }
    return h;
}

static void place(level *L, size_t k) {
    size_t i = hash(&L->nodes[k]) & (L->table_cap - 1);
    while (L->table[i]) i = (i + 1) & (L->table_cap - 1);
    L->table[i] = (uint32_t)(k + 1);
}

/* Return the index of node n in level L, adding it if new; *fresh says
 * whether it was. */
static uint32_t intern_node(level *L, const node *n, int *fresh) {
    if (2 * (L->count + 1) > L->table_cap) {
        free(L->table);
        L->table_cap = L->table_cap ? 2 * L->table_cap : 1 << 16;
        L->table = calloc(L->table_cap, sizeof *L->table);
        if (!L->table) die("out of memory", 5);
        for (size_t k = 0; k < L->count; k++) place(L, k);
    }
    size_t i = hash(n) & (L->table_cap - 1);
    for (; L->table[i]; i = (i + 1) & (L->table_cap - 1)) {
        size_t k = L->table[i] - 1;
        if (memcmp(&L->nodes[k], n, sizeof *n) == 0) {
            *fresh = 0;
            return (uint32_t)k;
        }
    }
    if (L->count >= UINT32_MAX - 1) die("level full", 5);
    if (L->count == L->cap) {
        L->cap = L->cap ? 2 * L->cap : 1 << 16;
        L->nodes = realloc(L->nodes, L->cap * sizeof *L->nodes);
        if (!L->nodes) die("out of memory", 5);
    }
    L->nodes[L->count] = *n;
    L->table[i] = (uint32_t)(L->count + 1);
    *fresh = 1;
    return (uint32_t)L->count++;
}

/* Intern the packed state p (lv[0].slots * 16 bytes) and return its
 * number. */
static size_t intern(const char *p, uint32_t from, uint8_t input) {
    node n;
    int fresh = 0;
    for (size_t i = 0; i < lv[0].slots; i++) {
        memcpy(&n, p + 16 * i, 16);
        work[0][i] = intern_node(&lv[0], &n, &fresh);
    }
    for (int l = 1; l < nlevels; l++)
        for (size_t i = 0; i < lv[l].slots; i++) {
            for (int j = 0; j < 4; j++) {
                size_t c = 4 * i + j;
                n.w[j] = c < lv[l - 1].slots ? work[l - 1][c] : 0;
            }
            work[l][i] = intern_node(&lv[l], &n, &fresh);
        }
    size_t k = work[nlevels - 1][0];
    if (fresh) {
        if (k >= max_states) {
            printf("LIMIT states=%zu\n", k);
            exit(3);
        }
        parent[k] = from;
        via[k] = input;
    }
    return k;
}

/* Write the packed bytes of state k to p. */
static void decode(size_t k, char *p) {
    work[nlevels - 1][0] = (uint32_t)k;
    for (int l = nlevels - 1; l > 0; l--)
        for (size_t i = 0; i < lv[l].slots; i++) {
            const node *n = &lv[l].nodes[work[l][i]];
            for (int j = 0; j < 4; j++) {
                size_t c = 4 * i + j;
                if (c < lv[l - 1].slots) work[l - 1][c] = n->w[j];
            }
        }
    for (size_t i = 0; i < lv[0].slots; i++)
        memcpy(p + 16 * i, &lv[0].nodes[work[0][i]], 16);
}

static void pack(char *dst, const char *live) {
    for (size_t s = 0; s < nseg; s++) {
        memcpy(dst, live + seg_off[s], seg_len[s]);
        dst += seg_len[s];
    }
}

static void unpack(char *live, const char *src) {
    for (size_t s = 0; s < nseg; s++) {
        memcpy(live + seg_off[s], src, seg_len[s]);
        src += seg_len[s];
    }
}

/* Whether every byte outside the segments still has its initial value. */
static int constant_ok(const char *live) {
    size_t at = 0;
    for (size_t s = 0; s <= nseg; s++) {
        size_t end = s < nseg ? seg_off[s] : full;
        if (end > at && memcmp(live + at, init + at, end - at)) return 0;
        if (s < nseg) at = seg_off[s] + seg_len[s];
    }
    return 1;
}

static void witness(size_t k, int last) {
    size_t depth = 0;
    for (size_t x = k; x; x = parent[x]) depth++;
    int *word = malloc((depth + 1) * sizeof *word);
    size_t n = depth;
    word[n] = last;
    for (size_t x = k; x; x = parent[x]) word[--n] = rers_input(via[x]);
    printf("FOUND depth=%zu inputs=", depth + 1);
    for (size_t i = 0; i <= depth; i++) printf("%s%d", i ? " " : "", word[i]);
    putchar('\n');
}

int main(int argc, char **argv) {
    if (argc != 4) die("usage: reach MASK TARGET MAX_STATES", 1);
    FILE *fh = fopen(argv[1], "r");
    if (!fh) die("cannot open MASK", 1);
    size_t off, len, cap = 0;
    while (fscanf(fh, "%zu %zu", &off, &len) == 2) {
        if (nseg == cap) {
            cap = cap ? 2 * cap : 256;
            seg_off = realloc(seg_off, cap * sizeof *seg_off);
            seg_len = realloc(seg_len, cap * sizeof *seg_len);
        }
        seg_off[nseg] = off;
        seg_len[nseg++] = len;
        width += len;
    }
    fclose(fh);
    target = atoi(argv[2]);
    max_states = strtoull(argv[3], 0, 10);
    char *live = __start_rers_state;
    full = (size_t)(__stop_rers_state - __start_rers_state);
    for (size_t s = 0; s < nseg; s++)
        if (seg_off[s] + seg_len[s] > full ||
            (s && seg_off[s] < seg_off[s - 1] + seg_len[s - 1]))
            die("bad segment", 1);
    init = malloc(full);
    memcpy(init, live, full);
    size_t leaves = (width + 15) / 16;
    char *scratch = calloc(leaves, 16), *cur = calloc(leaves, 16);
    for (size_t slots = leaves;; slots = (slots + 3) / 4) {
        if (nlevels == MAX_LEVELS) die("state too wide", 1);
        lv[nlevels].slots = slots;
        work[nlevels++] = malloc(slots * sizeof(uint32_t));
        if (slots == 1) break;
    }
    int ninputs = rers_input_count();
    fprintf(stderr, "state width %zu of %zu bytes, %d levels, %d inputs\n", width, full,
            nlevels, ninputs);
    parent = malloc(max_states * sizeof *parent);
    via = malloc(max_states);
    if (!parent || !via) die("out of memory", 5);
    pack(scratch, live);
    intern(scratch, 0, 0);
    level *top = &lv[nlevels - 1];
    for (size_t k = 0; k < top->count; k++) {
        if (k % 1000000 == 0 && k) {
            size_t bytes = 0;
            for (int l = 0; l < nlevels; l++)
                bytes += lv[l].cap * sizeof(node) + lv[l].table_cap * sizeof(uint32_t);
            fprintf(stderr, "expanded %zu of %zu, %.1f GB in tables\n", k, top->count,
                    bytes / 1e9);
        }
        decode(k, cur);
        for (int j = 0; j < ninputs; j++) {
            /* Bytes outside the segments keep their initial value. */
            unpack(live, cur);
            hit = nout = 0;
            calculate_output(rers_input(j));
            if (!constant_ok(live)) die("a byte outside the mask changed", 4);
            if (hit) {
                witness(k, rers_input(j));
                return 0;
            }
            if (!nout) continue; /* an invalid input ends the run */
            pack(scratch, live);
            intern(scratch, (uint32_t)k, (uint8_t)j);
        }
    }
    printf("END states=%zu\n", top->count);
    return 0;
}
