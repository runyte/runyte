# SPDX-License-Identifier: MPL-2.0
"""Guard event routing, release identity and merge-gate coverage."""
from pathlib import Path
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[2]


def read(path):
    return yaml.load((ROOT / path).read_text(), Loader=yaml.BaseLoader)


class WorkflowPolicyTests(unittest.TestCase):
    def setUp(self):
        self.ci = read('.github/workflows/ci.yml')
        self.desktop = read('.github/workflows/native-window.yml')
        self.dev = read('.github/workflows/development.yml')

    def test_full_pr_and_exact_main_push_gates(self):
        for workflow in (self.ci, self.desktop):
            self.assertIn('main', workflow['on']['push']['branches'])
            self.assertEqual(set(workflow['on']['pull_request']['branches']),
                             {'main', 'dev', 'exp'})
            for branch in ('dev', 'exp'):
                self.assertNotIn(branch, workflow['on']['push']['branches'])
            self.assertTrue(workflow['on']['schedule'])
            source = workflow['jobs']['source']
            self.assertEqual(source['steps'][0]['with']['ref'],
                             "${{ github.event_name == 'schedule' && 'dev' || github.sha }}")
            self.assertIn('git rev-parse HEAD', source['steps'][-1]['run'])
            self.assertIn("'nightly-dev'", workflow['concurrency']['group'])
            for name, job in workflow['jobs'].items():
                if 'if' in job:
                    self.assertEqual(job['if'], '${{ !cancelled() }}', name)
                if name == 'source':
                    continue
                self.assertIn('source', job['needs'])
                checkouts = [s for s in job['steps']
                             if s.get('uses', '').startswith('actions/checkout@')
                             and 'repository' not in s.get('with', {})
                             and s.get('with', {}).get('path') != '.ci-tools']
                self.assertEqual(len(checkouts), 1, name)
                self.assertEqual(checkouts[0]['with']['ref'],
                                 "${{ needs.source.outputs.sha || '0000000000000000000000000000000000000000' }}")

    def test_slim_pushes_have_real_tests_and_distinct_names(self):
        self.assertEqual(set(self.dev['on']['push']['branches']), {'dev', 'exp'})
        self.assertEqual(len(self.dev['jobs']), 1)
        job = self.dev['jobs']['test']
        self.assertEqual(job['strategy']['matrix']['os'],
                         ['ubuntu-latest', 'macos-latest'])
        self.assertTrue(job['name'].startswith('Development tests'))
        commands = {s.get('run'): s for s in job['steps'] if 'run' in s}
        self.assertNotIn('if', commands['cargo test --locked'])
        for command in ('cargo fmt --check',
                        'cargo clippy --all-targets --locked -- -D warnings'):
            self.assertEqual(commands[command]['if'], "runner.os == 'Linux'")

    def test_required_names_run_canonical_coverage_without_exclusions(self):
        self.assertEqual(self.ci['jobs']['gates']['name'],
                         "${{ github.event_name == 'schedule' && 'Nightly ' || '' }}Gates (ubuntu-latest)")
        self.assertEqual(self.ci['jobs']['macos']['name'],
                         "${{ github.event_name == 'schedule' && 'Nightly ' || '' }}Tests (macos-latest)")
        for job in ('gates', 'macos'):
            steps = self.ci['jobs'][job]['steps']
            measurement = [s for s in steps if s.get('uses') == './.ci-tools/.github/actions/core-coverage']
            self.assertEqual(len(measurement), 1)
            self.assertNotIn('if', measurement[0])
            self.assertNotIn('continue-on-error', measurement[0])
        action = read('.github/actions/core-coverage/action.yml')
        commands = [s.get('run', '') for s in action['runs']['steps']]
        self.assertIn('cargo llvm-cov --locked --package runyte --no-report', commands)
        self.assertIn('cargo llvm-cov report --fail-under-lines 89', commands)
        self.assertFalse(any('--ignore-filename-regex' in c for c in commands))
        self.assertTrue(all('if' not in s and 'continue-on-error' not in s
                            for s in action['runs']['steps']))

    def test_stress_and_native_resource_envelopes_remain(self):
        stress = self.ci['jobs']['lifecycle-stress']
        commands = '\n'.join(s.get('run', '') for s in stress['steps'])
        self.assertIn('for attempt in {1..10}', commands)
        for suite in ('local_protocol', 'diagnostic_log', 'persistent_host', 'workspace_bulk'):
            self.assertIn('cargo test --locked --test ' + suite, commands)
        self.assertEqual(stress['strategy']['matrix']['os'], ['ubuntu-latest', 'macos-latest'])
        self.assertEqual(self.ci['jobs']['windows']['env']['CARGO_BUILD_JOBS'], '1')
        self.assertEqual(self.ci['jobs']['windows']['env']['RUST_TEST_THREADS'], '2')

    def test_artifacts_stay_in_run_and_consumers_cannot_rebuild(self):
        for name in ('ru-time', 'frozen-plugin'):
            job = self.ci['jobs'][name]
            self.assertIn('plugin-conformance', job['needs'])
            downloads = [s for s in job['steps']
                         if s.get('uses', '').startswith('actions/download-artifact@')]
            self.assertEqual(len(downloads), 1)
            self.assertEqual(downloads[0]['with']['name'], 'plugin-host-${{ matrix.os }}')
            self.assertNotIn('run-id', downloads[0]['with'])
            self.assertNotIn('github-token', downloads[0]['with'])
            commands = '\n'.join(s.get('run', '') for s in job['steps'])
            self.assertIn('plugin_host.py restore', commands)
            self.assertNotIn('cargo build', commands)

    def test_helpers_follow_workflow_revision_not_scheduled_source(self):
        for name in ('gates', 'macos', 'plugin-conformance', 'ru-time', 'frozen-plugin'):
            steps = self.ci['jobs'][name]['steps']
            helpers = [s for s in steps if s.get('with', {}).get('path') == '.ci-tools']
            self.assertEqual(len(helpers), 1)
            self.assertEqual(helpers[0]['with']['ref'], '${{ github.workflow_sha }}')
            self.assertEqual(helpers[0]['with']['sparse-checkout'], '.github')
            self.assertEqual(helpers[0]['with']['persist-credentials'], 'false')
