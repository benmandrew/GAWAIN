#!/usr/bin/env python3
"""Join the jobs and the SPIN results into bench/results/beem-verdicts.csv.

One row per (ERROR instance, variant, property). `verdict` is phi on the
variant and `neg_verdict` is !phi; `seconds` is the time of the two pan
runs. `fin_verdict` and `fin_neg_verdict` are the same checks with runs
that end in a deadlock ignored, as a checker without SPIN's stuttering
would; they differ only where the variant deadlocks. `beem` is BEEM's own answer from results.xml for the instance with
these parameters, where BEEM has one (yes = holds). `aps_unreachable`
lists the propositions of the formula that are false in every reachable
state. Prints the counts used in bench/notes/beem.md.

usage: collect.py JOBS_DIR OUT.csv RESULTS.jsonl ...

A later file's result for a check replaces an earlier one, unless it is UNKNOWN.
"""
import collections
import csv
import json
import re
import sys

jobs = json.load(open(sys.argv[1] + '/jobs.json'))
res = {}
for line in (x for f in sys.argv[3:] for x in open(f)):
    r = json.loads(line)
    if r['check'] != 'build':
        old = res.get((r['key'], r['check']))
        if old is None or old['verdict'] == 'UNKNOWN' or r['verdict'] != 'UNKNOWN':
            res[(r['key'], r['check'])] = r


def get(key, check):
    return res.get((key, check), {'verdict': 'MISSING', 'reason': '', 'seconds': 0})


rows = []
for p in jobs['pairs']:
    k, pid = p['key'], p['property']
    phi, neg, st = get(k, f'phi_{pid}'), get(k, f'neg_{pid}'), get(k, 'states')
    var = jobs['variants'][k]
    aps = set(re.findall(r'[A-Za-z_]\w*', p['formula'])) & {
        c[3:] for c in var['claims'] if c.startswith('ap_')}
    dead = sorted(a for a in aps if get(k, f'ap_{a}')['verdict'] == 'TRUE')
    rows.append({
        'instance': p['instance'], 'params': p['params'], 'variant': p['variant'],
        'beem_instance': p['beem_instance'], 'property': pid, 'formula': p['formula'],
        'verdict': phi['verdict'], 'neg_verdict': neg['verdict'],
        'reason': ';'.join(x for x in (phi['reason'], neg['reason']) if x),
        'seconds': round(phi['seconds'] + neg['seconds'], 2),
        # Without a deadlock the fin checks are the plain ones.
        'fin_verdict': get(k, f'finphi_{pid}')['verdict'] if st.get('deadlocks') != 0
        else phi['verdict'],
        'fin_neg_verdict': get(k, f'finneg_{pid}')['verdict'] if st.get('deadlocks') != 0
        else neg['verdict'],
        'beem': p['beem'], 'states': st.get('states'),
        'beem_states': var['beem_states'], 'deadlocks': st.get('deadlocks'),
        'aps_unreachable': ' '.join(dead)})

with open(sys.argv[2], 'w', newline='') as fh:
    w = csv.DictWriter(fh, fieldnames=list(rows[0]))
    w.writeheader()
    w.writerows(rows)

# ---- counts
by = {(r['instance'], r['property'], r['variant']): r for r in rows}
err = [r for r in rows if r['variant'] == 'error']
print('pairs', len(err))
print('erroneous verdicts', collections.Counter(r['verdict'] for r in err))
print('correct verdicts', collections.Counter(r['verdict'] for r in rows if r['variant'] == 'correct'))
both = collections.Counter()
for r in err:
    c = by[(r['instance'], r['property'], 'correct')]
    both[(r['verdict'], c['verdict'])] += 1
print('(error, correct) verdicts', dict(both))
fail = [r for r in err if r['verdict'] == 'FALSE']
print('failing on erroneous, by !phi', collections.Counter(r['neg_verdict'] for r in fail))
only = [r for r in fail if by[(r['instance'], r['property'], 'correct')]['verdict'] == 'TRUE']
print('fail only on erroneous', len(only), 'of which survive degeneracy',
      sum(r['neg_verdict'] == 'FALSE' for r in only))
for r in only:
    print('  ', r['instance'], r['params'], r['property'], r['formula'], 'neg', r['neg_verdict'])
mism = [(r['instance'], r['variant'], r['property'], r['verdict'], r['beem']) for r in rows
        if r['beem'] in ('yes', 'no') and r['verdict'] in ('TRUE', 'FALSE')
        and (r['beem'] == 'yes') != (r['verdict'] == 'TRUE')]
fmism = [(r['instance'], r['variant'], r['property'], r['fin_verdict'], r['beem']) for r in rows
         if r['beem'] in ('yes', 'no') and r['fin_verdict'] in ('TRUE', 'FALSE')
         and (r['beem'] == 'yes') != (r['fin_verdict'] == 'TRUE')]
print('with finite runs ignored, disagree', sorted(set(fmism)))
print('fin differs from stutter', [(r['instance'], r['variant'], r['property'], r['verdict'],
                                    r['fin_verdict']) for r in rows
                                   if r['fin_verdict'] != r['verdict']])
known = sum(r['beem'] in ('yes', 'no') and r['verdict'] in ('TRUE', 'FALSE') for r in rows)
print('BEEM answers compared', known, 'disagree', len(set(mism)), sorted(set(mism)))
sm = {(r['instance'], r['variant']): (r['states'], r['beem_states']) for r in rows
      if r['beem_states'] and r['states']}
print('state counts compared', len(sm), 'differ (ours-1 vs BEEM)',
      {k: v for k, v in sm.items() if v[0] - 1 != v[1]})
print('unknown', collections.Counter(r['reason'] for r in rows
                                     if 'UNKNOWN' in (r['verdict'], r['neg_verdict'])))
print('unreachable APs', {(r['instance'], r['variant']): r['aps_unreachable']
                          for r in rows if r['aps_unreachable']})
print('deadlocks', {(r['instance'], r['variant']): r['deadlocks'] for r in rows if r['deadlocks']})
