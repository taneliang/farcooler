#!/usr/bin/env python3
"""Prove that deleting any archive validation guard makes its tests fail."""
import ast
import copy
import io
from pathlib import Path
import types
import unittest

import prototype_test as tests

path = Path(__file__).with_name("prototype.py")
tree = ast.parse(path.read_text())
function = next(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == "validate")
guards = [i for i, n in enumerate(function.body) if isinstance(n, ast.If)]
for index in guards:
    mutant = copy.deepcopy(tree)
    changed = next(n for n in mutant.body if isinstance(n, ast.FunctionDef) and n.name == "validate")
    line = changed.body[index].lineno
    changed.body[index].test = ast.Constant(value=False)
    ast.fix_missing_locations(mutant)
    module = types.ModuleType("mutant")
    module.__file__ = str(path.resolve())
    exec(compile(mutant, str(path), "exec"), module.__dict__)
    tests.p = module
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(tests.ArchiveTests)
    result = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
    if result.wasSuccessful():
        raise SystemExit(f"SURVIVED: deleted guard at line {line}")
    print(f"CAUGHT: deleted validation guard at line {line}")
print(f"All {len(guards)} guard-deletion mutations caught")
