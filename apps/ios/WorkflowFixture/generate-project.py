#!/usr/bin/env python3
"""Generate a simulator-only host of the production workflow view."""
import importlib.util
from pathlib import Path
root = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('generator', root.parent / 'NativeRowsFixture/generate-project.py')
g = importlib.util.module_from_spec(spec)
spec.loader.exec_module(g)
g.ROOT = root
g.FIXTURE = '../../../crates/client/src/testdata/workflow-presentation.json'
g.HOST_SOURCES = ['FixtureHost.swift'] + [f'../../shared/AgentKit/Sources/AgentKit/{name}.swift' for name in ['WireTypes', 'WorkflowRuns', 'WorkflowRunsView']]
g.TEST_SOURCES = {'FixtureLoaderTests': [], 'FixtureHostUITests': []}
g.BUNDLE_IDS = {'NativeRowsFixtureHost': 'com.farcooler.fixture.workflow', 'FixtureLoaderTests': 'com.farcooler.fixture.workflow.tests', 'FixtureHostUITests': 'com.farcooler.fixture.workflow.uitests'}
g.main()
