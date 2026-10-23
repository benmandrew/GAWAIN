#!/usr/bin/env python3
"""Rewrite LMCS-2006 SMV models (NuSMV 2.4 syntax) for nuXmv 2.1.0.

Boolean values written as 0/1 become FALSE/TRUE:
  * `1 :` case guards,
  * assignments `init(x) := 0;`, `next(x) := 1;`, `x := 0;` where x is declared boolean,
    including 0/1 results inside `case` expressions of such assignments.
Variables of any other type keep their numeric constants.

usage: lmcs_fix.py SRC_DIR DST_DIR   (copies every .smv and .ltl, fixing the .smv files)
"""
import re, sys, shutil
from pathlib import Path

DECL = re.compile(r'(?m)^\s*([A-Za-z_][\w.]*)\s*:\s*([^;]*);')
ASSIGN = re.compile(r'((?:init|next)\s*\(\s*)?([A-Za-z_][\w.]*)(\s*\))?\s*:=', re.S)


def strip_comments(t):
    return re.sub(r'--[^\n]*', '', t)


def decl_types(text):
    types = {}
    clean = strip_comments(text)
    # only look inside VAR / IVAR sections
    sections = re.findall(r'\b(?:I?VAR)\b(.*?)(?=\b(?:ASSIGN|DEFINE|INIT|TRANS|INVAR|FAIRNESS|JUSTICE|COMPASSION|SPEC|LTLSPEC|CTLSPEC|MODULE|I?VAR|FROZENVAR|CONSTANTS)\b|\Z)', clean, re.S)
    for sec in sections:
      for m in DECL.finditer(sec):
          name = m.group(1).split('.')[-1]
          typ = m.group(2).strip()
          types.setdefault(name, set()).add('bool' if typ == 'boolean' else 'other')
    return types


def expr_end(text, i):
    """index of the ';' ending the expression that starts at i (case/esac aware)"""
    depth = 0
    for m in re.finditer(r'\bcase\b|\besac\b|;', text[i:]):
        s = m.group(0)
        if s == 'case': depth += 1
        elif s == 'esac': depth -= 1
        elif depth == 0: return i + m.start()
    raise ValueError('unterminated assignment')


def fix_expr(e):
    e = re.sub(r'\{\s*[01]\s*,\s*[01]\s*\}', '{FALSE, TRUE}', e)
    e = re.sub(r'(?<![\w.-])0(?![\w.-])(?=\s*;|\s*$)', 'FALSE', e) if not 'case' in e else e
    if 'case' in e:
        e = re.sub(r':\s*0(\s*;)', r': FALSE\1', e)
        e = re.sub(r':\s*1(\s*;)', r': TRUE\1', e)
    else:
        e = re.sub(r'^(\s*)1(\s*)$', r'\1TRUE\2', e)
    return e


def fix(text):
    parts = re.split(r'(?m)^(?=MODULE\b)', text)
    local = [decl_types(x) for x in parts]
    # file-wide view, for targets written as `s.x := ...` that belong to another module
    glob = {}
    for d in local:
        for n, t in d.items(): glob.setdefault(n, set()).update(t)
    return ''.join(fix_module(x, d, glob) for x, d in zip(parts, local))


def fix_module(text, local, glob):
    def is_bool(full):
        n = full.split('.')[-1]
        t = local.get(n) if ('.' not in full and n in local) else glob.get(n)
        return t == {'bool'}
    declared = set(local) | set(glob)
    out, pos = [], 0
    # comments may contain ':=' or 'case'; assignments are searched in comment-blanked copy
    blank = re.sub(r'--[^\n]*', lambda m: ' ' * len(m.group(0)), text)
    for m in ASSIGN.finditer(blank):
        if m.start() < pos: continue
        name = m.group(2).split('.')[-1]
        s = m.end(); e = expr_end(blank, s)
        if not is_bool(m.group(2)):
            # DEFINE'd symbol (not a VAR) whose value is only ever 0/1 is a boolean
            body = blank[s:e]
            if name in declared or m.group(1) or not re.fullmatch(
                    r'\s*(?:\d+|case(?:\s*[^;:]+:\s*[01]\s*;)+\s*esac)\s*', body):
                continue
        out.append(text[pos:s]); out.append(fix_expr(text[s:e])); pos = e
    out.append(text[pos:])
    text = ''.join(out)
    # module instantiation arguments that are 0/1 are booleans
    def inst(m):
        args = re.sub(r'(?<![\w.-])1(?![\w.-])', 'TRUE', re.sub(r'(?<![\w.-])0(?![\w.-])', 'FALSE', m.group(2)))
        return m.group(1) + args + m.group(3)
    text = re.sub(r'(:\s*(?:process\s+)?[A-Za-z_][\w-]*\s*\()([^()]*(?:\([^()]*\)[^()]*)*)(\)\s*;)', inst, text)
    # `b = 0` / `b = 1` on boolean variables
    text = re.sub(r'([A-Za-z_][\w.]*(?:\[\d+\])?)(\s*=\s*)([01])(?![\w.-])',
                  lambda m: m.group(1) + m.group(2) + ('TRUE' if m.group(3) == '1' else 'FALSE')
                  if is_bool(m.group(1)) else m.group(0), text)
    text = re.sub(r'(<->)(\s*)([01])(?![\w.-])', lambda m: m.group(1) + m.group(2) + ('TRUE' if m.group(3) == '1' else 'FALSE'), text)
    # arithmetic on a boolean counter bit
    text = re.sub(r'\(\s*value\s*\+\s*carry_in\s*\)\s*mod\s*2', 'value xor carry_in', text)
    # words that are keywords in nuXmv
    text = re.sub(r'\bextend\b', 'extend_', text)
    text = re.sub(r'\bK\b(?=[\s.;])', 'K_', text)
    # case guards `1 :`
    text = re.sub(r'(^|;|\bcase\b)(\s*)1(\s*):', r'\1\2TRUE\3:', text, flags=re.M)
    return text


def main(src, dst):
    src, dst = Path(src), Path(dst)
    for p in sorted(src.rglob('*')):
        if p.is_dir(): continue
        q = dst / p.relative_to(src)
        q.parent.mkdir(parents=True, exist_ok=True)
        if p.suffix == '.smv':
            q.write_text(fix(p.read_text(errors='replace')))
        elif p.suffix == '.ltl':
            t = p.read_text()
            t = re.sub(r'\(1\)', ' TRUE ', re.sub(r'\(0\)', ' FALSE ', t))
            t = re.sub(r'\bextend\b', 'extend_', t)
            q.write_text(t)

if __name__ == '__main__':
    main(*sys.argv[1:3])
