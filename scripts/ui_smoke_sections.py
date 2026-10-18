"""Sections and parallel shards of scripts/ui-smoke.py, split at run time.

The smoke script stays one plain file that runs top to bottom. This module
reads its source with `ast` and splits the top-level statements after the
preamble marker into sections: a new section starts at a top-level comment
block that follows a blank line, and is named after the first line of that
comment. Sections that share state are joined into groups:

- a name a section reads before binding it itself, whose reaching
  definition is in another section (assignment, augmented assignment, `del`,
  import, `def`, a mutation such as `x.append(...)` or `x[k] = v`, a store
  through `global` in a function), joins the two; a definition inside an
  `if`/`for`/`while`/`try` does not hide the earlier ones;
- calling a function counts the names its body reads and mutates at the
  call site (nested `def`s and lambdas included);
- path variables (`x = scratch / "name"`) belong to their section, so a
  later section reading a file under one joins it;
- two sections naming the same literal directory or file directly under
  `scratch` join too (a file one writes and the other reads), except the
  preamble's own fixtures.

Statements every group may need run in every shard, in source order: the
preamble, imports, `def`s, and assignments whose value is a pure expression
of preamble names (`env = dict(os.environ, ...)`). The trailing epilogue
(closing the stalled socket, the pass line) runs at the end of each shard.

Groups are independent by construction, so `--shard K/N` gives each shard a
deterministic, weight-balanced share (greedy, heaviest first; weights are
line counts, refined by `target/ui-smoke-timings.json` from earlier shard
runs) and keeps running after a failed group, so one run shows every
failure. `--only NAME[,NAME]` runs the groups holding the named sections.
`--list` prints the groups. Without flags, ui-smoke.py runs serially exactly
as before and this module is never imported.
"""
import ast
import builtins
import json
import os
import re
import sys
import time
import traceback
from pathlib import Path

# the line that ends the shared preamble in ui-smoke.py
PREAMBLE_MARK = "# === sections:"
# sections (by slug) that must not run beside each other: joined into one
# group pinned to shard 1. Empty: every resource a section uses is per run
# (mkdtemp scratch, its XDG dirs, port 0, data-dir-hashed launchd labels and
# socket dirs), see the isolation notes in docs/development.md.
SERIAL = []
# names the epilogue reads that the shard runner provides itself
RUNNER_NAMES = {"started"}
# preamble paths whose files sections may change: every section naming one
# joins one group (the shared profile in scratch/cfg and scratch/data)
SHARED_PATHS = {"cfg", "data"}

MUTATORS = {"append", "extend", "insert", "pop", "popitem", "remove", "clear", "update",
            "setdefault", "add", "discard", "sort", "reverse"}
PURE_CALLS = {"dict", "list", "tuple", "set", "frozenset", "str", "int", "float", "bool", "len",
              "sorted", "range", "repr", "min", "max", "abs", "round", "enumerate", "zip",
              "Path", "json.dumps", "shlex.quote", "re.compile", "re.escape", "os.getuid",
              "os.path.join", "os.environ.get"}
PURE_METHODS = {"replace", "join", "format", "lower", "upper", "strip", "rstrip", "lstrip", "split",
                "splitlines", "startswith", "endswith", "encode", "decode", "items", "keys",
                "values", "get", "copy", "with_suffix", "with_name", "count", "find"}


def dotted(node):
    parts = []
    while isinstance(node, ast.Attribute):
        parts.append(node.attr)
        node = node.value
    if isinstance(node, ast.Name):
        parts.append(node.id)
        return ".".join(reversed(parts))
    return None


def base_name(node):
    while isinstance(node, (ast.Attribute, ast.Subscript, ast.Starred)):
        node = node.value
    return node.id if isinstance(node, ast.Name) else None


class FuncInfo:
    def __init__(self):
        self.loads = set()     # free names the body reads
        self.mutates = set()   # free names the body mutates or stores (global)


def func_info(node):
    """Free reads and mutations of a function or lambda body (nested ones too)."""
    info = FuncInfo()
    args = node.args
    params = {a.arg for a in args.posonlyargs + args.args + args.kwonlyargs}
    params |= {a.arg for a in (args.vararg, args.kwarg) if a}
    body = node.body if isinstance(node.body, list) else [node.body]
    declared = set()
    local = set(params)
    for sub in body:
        for n in ast.walk(sub):
            if isinstance(n, (ast.Global, ast.Nonlocal)):
                declared.update(n.names)
    for sub in body:
        for n in walk_scope(sub):
            if isinstance(n, ast.Name) and isinstance(n.ctx, (ast.Store, ast.Del)):
                local.add(n.id)
            elif isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                local.add(n.name)
            elif isinstance(n, (ast.Import, ast.ImportFrom)):
                local.update((a.asname or a.name).split(".")[0] for a in n.names)
            elif isinstance(n, ast.ExceptHandler) and n.name:
                local.add(n.name)
            elif isinstance(n, ast.comprehension):
                local.update(x.id for x in ast.walk(n.target) if isinstance(x, ast.Name))
    local -= declared
    for sub in body:
        for n in walk_scope(sub):
            if isinstance(n, ast.Name):
                if isinstance(n.ctx, ast.Load) and n.id not in local:
                    info.loads.add(n.id)
                elif n.id in declared:
                    info.mutates.add(n.id)
            elif isinstance(n, (ast.Attribute, ast.Subscript)) and isinstance(n.ctx, (ast.Store, ast.Del)):
                b = base_name(n)
                if b and b not in local:
                    info.mutates.add(b)
            elif isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute) and n.func.attr in MUTATORS:
                b = base_name(n.func.value)
                if b and b not in local:
                    info.mutates.add(b)
            elif isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda)):
                inner = func_info(n)
                info.loads |= inner.loads - local
                info.mutates |= inner.mutates - local
            if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)):
                for d in n.decorator_list + n.args.defaults + [x for x in n.args.kw_defaults if x]:
                    info.loads |= {x.id for x in ast.walk(d) if isinstance(x, ast.Name)} - local
    return info


def walk_scope(node):
    """ast.walk that does not descend into nested function bodies (their
    own scope; func_info handles them) but yields the nested node itself."""
    todo = [node]
    while todo:
        n = todo.pop()
        yield n
        if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda)) and n is not node:
            continue
        todo.extend(ast.iter_child_nodes(n))


class Scan(ast.NodeVisitor):
    """Module-scope effects of one top-level statement, in evaluation order."""

    def __init__(self):
        self.bound = set()
        self.loads = []        # names read before this statement bound them
        self.deferred = set()  # names read by function / lambda bodies
        self.stores = {}       # name -> True when every store is conditional
        self.funcs = {}        # name -> FuncInfo for functions this statement binds
        self.cond = 0

    def load(self, name):
        if name not in self.bound and name not in self.loads:
            self.loads.append(name)

    def store(self, name, cond=False):
        self.bound.add(name)
        c = cond or self.cond > 0
        self.stores[name] = self.stores.get(name, True) and c

    def mutate(self, node):
        b = base_name(node)
        if b:
            self.load(b)
            self.store(b, cond=True)

    def target(self, t):
        if isinstance(t, ast.Name):
            self.store(t.id)
        elif isinstance(t, (ast.Tuple, ast.List)):
            for e in t.elts:
                self.target(e)
        elif isinstance(t, ast.Starred):
            self.target(t.value)
        else:  # attribute / subscript: the object is read and changed
            self.visit(t.value)
            if isinstance(t, ast.Subscript):
                self.visit(t.slice)
            self.mutate(t)

    def visit_Name(self, node):
        if isinstance(node.ctx, ast.Load):
            self.load(node.id)
        elif isinstance(node.ctx, ast.Store):
            self.store(node.id)
        else:
            self.store(node.id)

    def visit_Assign(self, node):
        self.visit(node.value)
        for t in node.targets:
            self.target(t)
        if isinstance(node.value, ast.Lambda):
            for t in node.targets:
                if isinstance(t, ast.Name):
                    self.funcs[t.id] = func_info(node.value)

    def visit_AnnAssign(self, node):
        if node.value is not None:
            self.visit(node.value)
            self.target(node.target)

    def visit_AugAssign(self, node):
        if isinstance(node.target, ast.Name):
            self.load(node.target.id)
            self.visit(node.value)
            self.store(node.target.id)
        else:
            self.visit(node.value)
            self.target(node.target)

    def visit_NamedExpr(self, node):
        self.visit(node.value)
        self.target(node.target)

    def visit_Delete(self, node):
        for t in node.targets:
            if isinstance(t, ast.Name):
                self.store(t.id)
            else:
                self.target(t)

    def visit_Import(self, node):
        for a in node.names:
            self.store((a.asname or a.name).split(".")[0])

    visit_ImportFrom = visit_Import

    def function(self, node, name):
        info = func_info(node)
        self.deferred |= info.loads
        for m in info.mutates:  # a call changes it; the definition may be called here
            self.store(m, cond=True)
        if name:
            self.funcs[name] = info
            self.store(name)

    def visit_FunctionDef(self, node):
        for d in node.decorator_list + node.args.defaults + [x for x in node.args.kw_defaults if x]:
            self.visit(d)
        self.function(node, node.name)

    visit_AsyncFunctionDef = visit_FunctionDef

    def visit_Lambda(self, node):
        for d in node.args.defaults + [x for x in node.args.kw_defaults if x]:
            self.visit(d)
        self.function(node, None)

    def visit_ClassDef(self, node):
        for d in node.decorator_list + node.bases + [k.value for k in node.keywords]:
            self.visit(d)
        for n in ast.walk(node):
            if isinstance(n, ast.Name) and isinstance(n.ctx, ast.Load):
                self.deferred.add(n.id)
        self.store(node.name)

    def visit_Call(self, node):
        self.visit(node.func)
        for a in node.args:
            self.visit(a)
        for k in node.keywords:
            self.visit(k.value)
        if isinstance(node.func, ast.Attribute) and node.func.attr in MUTATORS:
            self.mutate(node.func.value)

    def comprehension(self, node, elts):
        shadow = set()
        saved = self.bound
        for i, gen in enumerate(node.generators):
            self.bound = saved | shadow if i else saved
            self.visit(gen.iter)
            shadow |= {x.id for x in ast.walk(gen.target) if isinstance(x, ast.Name)}
            self.bound = saved | shadow
            for c in gen.ifs:
                self.visit(c)
        self.bound = saved | shadow
        for e in elts:
            self.visit(e)
        self.bound = saved

    def visit_ListComp(self, node):
        self.comprehension(node, [node.elt])

    visit_SetComp = visit_GeneratorExp = visit_ListComp

    def visit_DictComp(self, node):
        self.comprehension(node, [node.key, node.value])

    def conditional(self, *bodies):
        self.cond += 1
        for body in bodies:
            for s in body:
                self.visit(s)
        self.cond -= 1

    def visit_If(self, node):
        self.visit(node.test)
        self.conditional(node.body, node.orelse)

    def visit_For(self, node):
        self.visit(node.iter)
        self.cond += 1
        self.target(node.target)
        self.cond -= 1
        self.conditional(node.body, node.orelse)

    visit_AsyncFor = visit_For

    def visit_While(self, node):
        self.cond += 1
        self.visit(node.test)
        self.cond -= 1
        self.conditional(node.body, node.orelse)

    def visit_Try(self, node):
        # only a handler can let the body stop half way and go on
        if node.handlers:
            self.conditional(node.body)
        else:
            for s in node.body:
                self.visit(s)
        self.cond += 1
        for h in node.handlers:
            if h.type:
                self.visit(h.type)
            if h.name:
                self.store(h.name)
            for s in h.body:
                self.visit(s)
        for s in node.orelse:
            self.visit(s)
        self.cond -= 1
        for s in node.finalbody:
            self.visit(s)

    visit_TryStar = visit_Try

    def visit_Match(self, node):
        self.visit(node.subject)
        self.cond += 1
        for case in node.cases:
            for n in ast.walk(case.pattern):
                for attr in ("name", "rest"):
                    v = getattr(n, attr, None)
                    if isinstance(v, str):
                        self.store(v)
            if case.guard:
                self.visit(case.guard)
            for s in case.body:
                self.visit(s)
        self.cond -= 1

    def visit_With(self, node):
        for item in node.items:
            self.visit(item.context_expr)
            if item.optional_vars is not None:
                self.target(item.optional_vars)
        for s in node.body:
            self.visit(s)

    visit_AsyncWith = visit_With


def pure(node):
    """An expression with no side effects (calls only to PURE_CALLS /
    PURE_METHODS); lambdas are pure (their body runs later)."""
    if isinstance(node, ast.Lambda):
        return True
    for n in walk_scope(node):
        if isinstance(n, ast.Lambda):
            continue
        if isinstance(n, (ast.Await, ast.Yield, ast.YieldFrom, ast.NamedExpr)):
            return False
        if isinstance(n, ast.Call):
            name = dotted(n.func)
            if name in PURE_CALLS:
                continue
            if isinstance(n.func, ast.Attribute) and n.func.attr in PURE_METHODS:
                continue
            return False
    return True


def scratch_component(node):
    """'name' of `scratch / "name" ...` or f"{scratch}/name...", else None."""
    if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Div):
        left, right = node.left, node.right
        while isinstance(left, ast.BinOp) and isinstance(left.op, ast.Div):
            left, right = left.left, left.right
        if isinstance(left, ast.Name) and left.id == "scratch" and isinstance(right, ast.Constant) \
                and isinstance(right.value, str):
            return right.value.split("/")[0] or None
    if isinstance(node, ast.JoinedStr) and len(node.values) >= 2:
        first, second = node.values[0], node.values[1]
        if isinstance(first, ast.FormattedValue) and isinstance(first.value, ast.Name) \
                and first.value.id == "scratch" and isinstance(second, ast.Constant) \
                and isinstance(second.value, str) and second.value.startswith("/"):
            m = re.match(r"/([A-Za-z0-9_.-]+)", second.value)
            if m:
                return m[1]
    return None


def is_path_var(stmt):
    """`x = <path expression rooted at a name>`: a directory a section owns."""
    return isinstance(stmt, ast.Assign) and isinstance(stmt.value, ast.BinOp) \
        and isinstance(stmt.value.op, ast.Div)


class Section:
    def __init__(self, index, name, start):
        self.index = index
        self.name = name
        self.slug = ""
        self.start = start     # first line, its comment included
        self.end = start
        self.stmts = []

    @property
    def lines(self):
        return self.end - self.start + 1


class Plan:
    pass


def slugify(text, taken):
    words = re.findall(r"[a-z0-9]+", text.lower())
    slug = "-".join(words[:6])[:48].strip("-") or "section"
    base, n = slug, 2
    while slug in taken:
        slug = f"{base}-{n}"
        n += 1
    taken.add(slug)
    return slug


def analyse(source, filename="ui-smoke.py"):
    tree = ast.parse(source, filename)
    lines = source.splitlines()
    mark = next((i + 1 for i, l in enumerate(lines) if l.startswith(PREAMBLE_MARK)), None)
    if mark is None:
        raise SystemExit(f"{filename}: no line starting with {PREAMBLE_MARK!r} ends the preamble")
    body = tree.body
    first = lambda s: min([s.lineno] + [d.lineno for d in getattr(s, "decorator_list", [])])
    pre = [i for i, s in enumerate(body) if s.lineno < mark]

    # sections: a top-level comment block after a blank line, or one opening
    # with "# ---", starts one
    sections = []
    prev_end = mark
    for i, s in enumerate(body):
        if i in pre:
            continue
        gap = lines[prev_end:first(s) - 1]  # lines between the statements
        comment = []
        for l in reversed(gap):
            if not l.startswith("#"):
                break
            comment.insert(0, l)
        before = gap[len(gap) - len(comment) - 1] if len(gap) > len(comment) else None
        starts = not sections or bool(comment) and (
            (before is not None and not before.strip()) or comment[0].startswith("# ---"))
        if starts:
            if comment:
                name = comment[0].lstrip("#").strip().strip("-").strip()
                start = first(s) - len(comment)
            else:
                name, start = f"lines {first(s)}", first(s)
            sections.append(Section(len(sections), name, start))
        sec = sections[-1]
        sec.stmts.append(i)
        sec.end = s.end_lineno
        prev_end = s.end_lineno
    taken = set()
    for sec in sections:
        sec.slug = slugify(sec.name, taken)
    sec_of = {i: sec.index for sec in sections for i in sec.stmts}

    scans = {}
    for i, s in enumerate(body):
        sc = Scan()
        sc.visit(s)
        scans[i] = sc

    # union-find over sections
    parent = list(range(len(sections)))
    reasons = {}

    def find(x):
        while parent[x] != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x

    def union(a, b, why):
        ra, rb = find(a), find(b)
        if ra != rb:
            parent[max(ra, rb)] = min(ra, rb)
            reasons.setdefault((min(a, b), max(a, b)), why)

    definers = {}   # name -> statement indices since its last unconditional definition
    funcs = {}      # statement index -> {name: FuncInfo}
    replicated = set()
    kind = {}       # statement index -> "pre" | "rep" | "sec"

    def expand(names, seen=None):
        """names plus what the functions among them read / change when called."""
        seen = set() if seen is None else seen
        loads, mutates = set(names), set()
        for n in names:
            for d in definers.get(n, []):
                info = funcs.get(d, {}).get(n)
                if info and (d, n) not in seen:
                    seen.add((d, n))
                    more, mut = expand(info.loads, seen)
                    loads |= more
                    mutates |= info.mutates | mut
        return loads, mutates

    def define(i, sc, cond_names=()):
        for name, cond in sc.stores.items():
            if cond and name in definers:
                definers[name] = definers[name] + [i]
            else:
                definers[name] = [i]
        for name in cond_names:
            definers[name] = definers.get(name, []) + [i]

    edges = []      # (reading statement, name, section, section, why)
    resolved = {}   # statement index -> {name read: its reaching definitions}
    for i, s in enumerate(body):
        sc = scans[i]
        funcs[i] = sc.funcs
        if i in pre:
            kind[i] = "pre"
            define(i, sc)
            continue
        candidate = isinstance(s, (ast.Import, ast.ImportFrom, ast.FunctionDef, ast.AsyncFunctionDef)) \
            or (isinstance(s, ast.Assign) and all(isinstance(t, ast.Name) for t in s.targets)
                and pure(s.value) and not is_path_var(s))
        # a def's or a lambda's body runs when called (its reads count at the
        # call site); a lambda inside an expression may run right away
        later = candidate and (isinstance(s, (ast.FunctionDef, ast.AsyncFunctionDef))
                               or isinstance(getattr(s, "value", None), ast.Lambda))
        loads = list(sc.loads) + ([] if later else sorted(sc.deferred - sc.bound))
        loads, mutates = expand(loads)
        deps = {d for n in loads for d in definers.get(n, [])}
        if candidate and all(kind.get(d) in ("pre", "rep") for d in deps) and not mutates:
            kind[i] = "rep"
            replicated.add(i)
            define(i, sc)
            continue
        kind[i] = "sec"
        for n in sorted(loads):
            for d in definers.get(n, []):
                if kind.get(d) == "sec" and sec_of[d] != sec_of[i]:
                    edges.append((i, n, sec_of[d], sec_of[i], f"name `{n}` (line {body[d].lineno} -> {s.lineno})"))
        define(i, sc, sorted(mutates))
        resolved[i] = {n: list(definers.get(n, [])) for n in loads}

    # files and dirs under scratch named by literal in two sections
    pre_comp = set(SHARED_PATHS)
    for i in pre:
        for n in ast.walk(body[i]):
            c = scratch_component(n)
            if c:
                pre_comp.add(c)
    owner = {}
    for sec in sections:
        for i in sec.stmts:
            for n in ast.walk(body[i]):
                c = scratch_component(n)
                if c and c not in pre_comp:
                    if c in owner and owner[c] != sec.index:
                        union(owner[c], sec.index, f"path scratch/{c} (line {n.lineno})")
                    owner.setdefault(c, sec.index)
    # the shared profile: sections naming scratch/cfg or scratch/data
    shared = {}
    for sec in sections:
        for i in sec.stmts:
            if i in replicated:
                continue
            named = {n for n, ds in resolved[i].items()
                     if n in SHARED_PATHS and any(kind.get(d) == "pre" for d in ds)}
            for p in named:
                if p in shared and shared[p] != sec.index:
                    union(shared[p], sec.index, f"shared profile `{p}` (line {body[i].lineno})")
                shared.setdefault(p, sec.index)

    # epilogue: trailing statements reading only what every shard has (and
    # the names the runner provides)
    epilogue = []
    last = sections[-1] if sections else None
    if last:
        for i in reversed(last.stmts):
            if i in replicated:
                continue
            ok = all(n in RUNNER_NAMES or all(kind.get(d) in ("pre", "rep") for d in ds)
                     for n, ds in resolved[i].items())
            if not ok:
                break
            epilogue.insert(0, i)
        if len(epilogue) == len([i for i in last.stmts if i not in replicated]):
            epilogue = []  # never the whole last section
        if epilogue:
            last.stmts = [i for i in last.stmts if i not in epilogue]
            last.end = max(body[i].end_lineno for i in last.stmts)

    for i, n, a, b, why in edges:
        if not (i in epilogue and n in RUNNER_NAMES):
            union(a, b, why)

    serial = [sec.index for sec in sections if sec.slug in SERIAL]
    for a, b in zip(serial, serial[1:]):
        union(a, b, "SERIAL")

    groups = {}
    for sec in sections:
        groups.setdefault(find(sec.index), []).append(sec)
    plan = Plan()
    plan.tree, plan.body, plan.lines = tree, body, lines
    plan.preamble = pre
    plan.replicated = sorted(replicated)
    plan.epilogue = epilogue
    plan.sections = sections
    plan.groups = [groups[k] for k in sorted(groups)]
    plan.serial = [g for g in plan.groups if any(s.index in serial for s in g)]
    plan.reasons = reasons
    plan.sec_of = sec_of
    return plan


def group_key(group):
    return group[0].slug


def weights(plan, timings):
    """Each group's weight: its recorded seconds, else its line count at the
    recorded seconds-per-line rate (line counts alone without timings)."""
    lines = {group_key(g): sum(s.lines for s in g) for g in plan.groups}
    known = {k: timings[k] for k in lines if isinstance(timings.get(k), (int, float))}
    rate = sum(known.values()) / sum(lines[k] for k in known) if known else None
    out = {}
    for k, n in lines.items():
        if rate is None:
            out[k] = float(n)
        else:
            out[k] = float(known.get(k, n * rate))
    return out, rate is not None


def shards(plan, n, timings):
    w, _ = weights(plan, timings)
    load = [0.0] * n
    out = [[] for _ in range(n)]
    rest = list(plan.groups)
    for g in plan.serial:  # pinned to shard 1
        out[0].append(g)
        load[0] += w[group_key(g)]
        rest.remove(g)
    for g in sorted(rest, key=lambda g: (-w[group_key(g)], g[0].start)):
        k = min(range(n), key=lambda j: (load[j], j))
        out[k].append(g)
        load[k] += w[group_key(g)]
    for k in range(n):
        out[k].sort(key=lambda g: g[0].start)
    return out, load


def read_timings():
    path = os.environ.get("RIFT_UI_TIMINGS")
    if not path:
        path = str(Path(__file__).resolve().parents[1] / "target" / "ui-smoke-timings.json")
    try:
        data = json.loads(Path(path).read_text())
        return data if isinstance(data, dict) else {}
    except (OSError, ValueError):
        return {}


def select(plan, names):
    wanted = [n.strip().lower() for n in names.split(",") if n.strip()]
    chosen = []
    for w in wanted:
        hits = [g for g in plan.groups if any(w == s.slug for s in g)] or \
            [g for g in plan.groups if any(w in s.slug or w in s.name.lower() for s in g)]
        if not hits:
            raise SystemExit(f"--only {w}: no section matches (ui-smoke.py BINARY --list)")
        chosen += [g for g in hits if g not in chosen]
    return sorted(chosen, key=lambda g: g[0].start)


def describe(plan, groups, timings, header=None):
    w, timed = weights(plan, timings)
    if header:
        print(header)
    for g in groups:
        k = group_key(g)
        lines = sum(s.lines for s in g)
        est = f"{w[k]:.0f}s" if timed else f"{lines} lines"
        pinned = " [serial, shard 1]" if g in plan.serial else ""
        print(f"group {k}  ({len(g)} section{'s' if len(g) > 1 else ''}, {lines} lines, weight {est}){pinned}")
        for s in g:
            print(f"  {s.slug:<50} lines {s.start}-{s.end}  {s.name[:70]}")


def run(plan, path, binary, groups, label, code_name):
    """Run the preamble, the chosen groups (with every replicated statement)
    and the epilogue in one namespace; a failed group is reported and the
    next one runs. Returns the process exit status."""
    chosen = {s.index for g in groups for s in g}
    group_of = {s.index: g for g in groups for s in g}
    ns = {"__name__": "__main__", "__file__": str(path), "__builtins__": builtins}
    sys.argv = [str(path), binary]
    started = time.monotonic()
    failed = []
    timings = {}

    def execute(i):
        code = compile(ast.Module(body=[plan.body[i]], type_ignores=[]), code_name, "exec")
        exec(code, ns)

    def report(what, exc):
        # where it failed, then the end of the message (an assertion's
        # message is often a whole dump)
        stack = [f for f in traceback.extract_tb(exc.__traceback__) if f.filename != __file__]
        frames = "".join(traceback.format_list(stack)).rstrip().splitlines()[-8:]
        message = "".join(traceback.format_exception_only(type(exc), exc)).rstrip().splitlines()
        tail = frames + (message[:1] + ["..."] + message[-20:] if len(message) > 22 else message)
        print(f"\nFAIL {label}: {what}", flush=True)
        print("\n".join("  | " + l for l in tail), flush=True)

    for i in plan.preamble:
        try:
            execute(i)
        except Exception as exc:  # nothing runs without the preamble
            report(f"preamble (line {plan.body[i].lineno})", exc)
            return 1
    ns["started"] = started
    order = sorted(set(plan.replicated) | {i for s in plan.sections if s.index in chosen for i in s.stmts})
    current, since = None, 0.0  # the running section and when it started
    for i in order:
        if i in plan.replicated:
            try:
                execute(i)
            except Exception as exc:
                report(f"shared statement (line {plan.body[i].lineno})", exc)
                return 1
            continue
        sec = plan.sections[plan.sec_of[i]]
        k = group_key(group_of[sec.index])
        if k in failed:
            continue
        if sec is not current:
            now = time.monotonic()
            if current is not None:
                ck = group_key(group_of[current.index])
                timings[ck] = timings.get(ck, 0) + now - since
            current, since = sec, now
            print(f"-- {label}: {sec.slug} (line {sec.start}, group {k})", flush=True)
        try:
            execute(i)
        except KeyboardInterrupt:
            raise
        except BaseException as exc:
            failed.append(k)
            timings.pop(k, None)
            current = None
            report(f"group {k}, section {sec.slug} ({path.name}:{plan.body[i].lineno})", exc)
    if current is not None:
        ck = group_key(group_of[current.index])
        timings[ck] = timings.get(ck, 0) + time.monotonic() - since
    out = os.environ.get("RIFT_UI_TIMINGS_OUT")
    if out:  # passed groups only: a failure's time says nothing
        try:
            Path(out).write_text(json.dumps({k: round(v, 1) for k, v in timings.items() if k not in failed},
                                            indent=1, sort_keys=True))
        except OSError:
            pass
    if failed:
        print(f"\nFAIL {label}: {len(failed)} of {len(groups)} groups failed: {', '.join(failed)}", flush=True)
        return 1
    for i in plan.epilogue:
        try:
            execute(i)
        except Exception as exc:
            report(f"epilogue (line {plan.body[i].lineno})", exc)
            return 1
    print(f"{label}: {len(groups)} groups passed in {time.monotonic() - started:.1f}s", flush=True)
    return 0


USAGE = "usage: ui-smoke.py BINARY [--list] [--only NAME[,NAME]] [--shard K/N]"


def main(file, argv):
    path = Path(file).resolve()
    args = list(argv)
    if not args or args[0].startswith("--"):
        raise SystemExit(USAGE)
    binary = args.pop(0)
    listing, only, shard = False, None, None
    while args:
        a = args.pop(0)
        if a == "--list":
            listing = True
        elif a == "--only" and args:
            only = args.pop(0)
        elif a.startswith("--only="):
            only = a.split("=", 1)[1]
        elif a == "--shard" and args:
            shard = args.pop(0)
        elif a.startswith("--shard="):
            shard = a.split("=", 1)[1]
        else:
            raise SystemExit(f"unknown argument {a!r}\n{USAGE}")
    # every shard of one run plans from the same text: the launcher passes a
    # snapshot taken before any shard started (the file may be edited meanwhile)
    snapshot = os.environ.get("RIFT_UI_SOURCE")
    source = Path(snapshot or path).read_text()
    # tracebacks quote the file the lines came from
    code_name = snapshot if snapshot and path.read_text() != source else str(path)
    plan = analyse(source, str(path))
    timings = read_timings()
    groups = plan.groups
    label = "ui"
    if shard:
        m = re.fullmatch(r"(\d+)/(\d+)", shard)
        if not m or not 1 <= int(m[1]) <= int(m[2]):
            raise SystemExit(f"--shard {shard}: expected K/N with 1 <= K <= N")
        k, n = int(m[1]), int(m[2])
        parts, load = shards(plan, n, timings)
        groups = parts[k - 1]
        label = f"shard {k}/{n}"
        _, timed = weights(plan, timings)
        weight = f"{load[k - 1]:.0f}{'s' if timed else ' lines'}"
    if only:
        picked = select(plan, only)
        groups = [g for g in groups if g in picked] if shard else picked
        label += f" --only {only}"
    if listing:
        print(f"{path.name}: {len(plan.body)} top-level statements; preamble {len(plan.preamble)}, "
              f"shared (every shard) {len(plan.replicated)}, epilogue {len(plan.epilogue)}; "
              f"{len(plan.sections)} sections in {len(plan.groups)} groups")
        header = f"[{label}: {len(groups)} groups" + (f", weight {weight}]" if shard and not only else "]")
        describe(plan, groups, timings, header=header if shard or only else None)
        return 0
    return run(plan, path, binary, groups, label, code_name)


def merge_timings(target, sources):
    """Fold the per-shard timings of one run into `target` (newest wins)."""
    merged = {}
    try:
        merged = json.loads(Path(target).read_text())
    except (OSError, ValueError):
        pass
    if not isinstance(merged, dict):
        merged = {}
    for src in sources:
        try:
            merged.update(json.loads(Path(src).read_text()))
        except (OSError, ValueError):
            pass
    try:
        Path(target).write_text(json.dumps(merged, indent=1, sort_keys=True) + "\n")
    except OSError:
        pass  # a read-only checkout (the linux gate's container): no record


if __name__ == "__main__":
    # scripts/ui-smoke.sh: ui_smoke_sections.py merge-timings TARGET SHARD.json...
    if len(sys.argv) >= 3 and sys.argv[1] == "merge-timings":
        merge_timings(sys.argv[2], sys.argv[3:])
    else:
        raise SystemExit("usage: ui_smoke_sections.py merge-timings TARGET SHARD.json...")
