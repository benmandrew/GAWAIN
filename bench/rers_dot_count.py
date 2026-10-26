#!/usr/bin/env python3
"""Count the reachable states of a RERS 2018 parallel model from its .dot file.

This is an independent check of src/rers.rs on the 2018 files. The .dot file
gives each component as a labelled transition system. An action moves every
component whose alphabet contains it, each by one of its edges with that
label, and is enabled only when all of them have such an edge. A state is the
component states plus the last action, the initial state has no last action,
and a state with no step gets a self-loop, as in src/rers.rs.

Prints states and transitions, which src/rers.rs must reproduce.

usage: rers_dot_count.py FILE.dot
"""
import itertools
import re
import sys

EDGE = re.compile(r'^(\d+) -> (\d+) \[\s*label="([^"]*)"')


def components(path):
    """[(initial, {state: {action: [targets]}}, alphabet)] in file order."""
    out = []
    for line in open(path):
        line = line.strip()
        if line.startswith('subgraph'):
            out.append([None, {}, set()])
            continue
        m = EDGE.match(line)
        if not m:
            continue
        src, dst, label = int(m[1]), int(m[2]), m[3]
        comp = out[-1]
        if label == '':
            comp[0] = dst
        else:
            comp[1].setdefault(src, {}).setdefault(label, []).append(dst)
            comp[2].add(label)
    return out


def main(path):
    comps = components(path)
    actions = sorted(set().union(*(c[2] for c in comps)))
    owners = {a: [i for i, c in enumerate(comps) if a in c[2]] for a in actions}
    first = (tuple(c[0] for c in comps), None)
    index = {first: 0}
    queue = [first]
    transitions = 0
    head = 0
    while head < len(queue):
        locals_, _ = queue[head]
        succ = set()
        for a in actions:
            moves = [comps[i][1].get(locals_[i], {}).get(a) for i in owners[a]]
            if not all(moves):
                continue
            for targets in itertools.product(*moves):
                nxt = list(locals_)
                for i, t in zip(owners[a], targets):
                    nxt[i] = t
                key = (tuple(nxt), a)
                if key not in index:
                    index[key] = len(queue)
                    queue.append(key)
                succ.add(index[key])
        transitions += len(succ) or 1
        head += 1
    print(f'{path}: {len(queue)} states, {transitions} transitions')


if __name__ == '__main__':
    main(*sys.argv[1:])
