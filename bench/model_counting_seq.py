# Makes the "seq" arm from "new": one ltl2tgba exec per formula, and the
# counts run after the oracle checks instead of beside them.
import sys
root = sys.argv[1]
p = root + '/src/count/mod.rs'
s = open(p).read()
a = '''        let automata = match self.translate(&formulas) {'''
assert a in s
s = s.replace(a, '''        let automata = match Err::<Vec<Raw>, String>(String::new()) {''')
open(p, 'w').write(s)
p = root + '/src/search/mod.rs'
s = open(p).read()
a = '''        let counting = scope.spawn(|| counter.similarities(&candidates));'''
assert a in s
s = s.replace(a, '''        let _ = scope;''')
a = '''        let semantic = counting.join().expect("the counting thread");'''
assert a in s
s = s.replace(a, '''        let semantic = counter.similarities(&candidates);''')
open(p, 'w').write(s)
