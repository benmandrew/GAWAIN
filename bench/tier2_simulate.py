#!/usr/bin/env python3
"""Simulate tier 2 lookup policies on logged batches, from per-pair replay times.

usage: tier2_simulate.py REPLAY_CSV LOG...

A batch's complete-check time is taken as 8 ms start-up plus, per pair, its solo time
less 8 ms; it times out when that passes the budget. Policies:
  full60      complete check only, 60 s (the old code)
  synt        syntactic check only
  synt+fullB  syntactic, then the complete check with budget B when nothing was proved
"""
import csv, sys

rows = {(r['purpose'], r['psi'], r['phi']): r for r in csv.DictReader(open(sys.argv[1]))}
batches = []
for log in sys.argv[2:]:
    cur = None
    for raw in open(log):
        parts = raw.rstrip('\n').split('\t')
        if parts[0] == 'BATCH':
            cur = {'purpose': parts[1], 'pairs': []}
            batches.append(cur)
        else:
            cur['pairs'].append(rows.get((cur['purpose'], parts[1], parts[2])))
START = 0.008


def full(b, budget):
    t = START
    for r in b['pairs']:
        t += (float(r['full_s']) - START) if r['full'] != 'TIMEOUT' else 1e9
    if t > budget:
        return False, budget
    return any(r['full'] == 'proved' for r in b['pairs']), t


def synt(b):
    t = START + sum(float(r['synt_s']) - START for r in b['pairs'])
    return any(r['synt'] == 'proved' for r in b['pairs']), t


for purpose in ('lookup', 'evict'):
    bs = [b for b in batches if b['purpose'] == purpose and None not in b['pairs']]
    if not bs:
        continue
    print(f'{purpose}: {len(bs)} batches')
    for name, budget in [('full60', 60), ('synt', None), ('synt+full0.25', .25), ('synt+full1', 1), ('synt+full5', 5), ('synt+full60', 60)]:
        hits = tsum = 0.0
        tmax = 0
        for b in bs:
            if name == 'full60':
                h, t = full(b, 60)
            else:
                h, t = synt(b)
                if not h and budget is not None:
                    h2, t2 = full(b, budget)
                    h, t = h2, t + t2
            hits += h
            tsum += t
            tmax = max(tmax, t)
        print(f'   {name:14} proved-something {int(hits):5}  total {tsum:9.2f} s  mean {tsum/len(bs)*1000:8.2f} ms  max {tmax:.2f} s')
