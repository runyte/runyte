# SPDX-License-Identifier: MPL-2.0
"""Required Unix acceptance: a missing binary, empty suite or skip is a failure."""
import os
from pathlib import Path
import unittest

if os.name == 'nt':
    raise SystemExit('Use the windows_context_acceptance Rust ConPTY driver on Windows')
binary = Path(os.environ['RUNYTE_CONTEXT_TEST_BINARY']).resolve(strict=True)
if not os.access(binary, os.X_OK):
    raise SystemExit(f'Not executable: {binary}')
suite = unittest.TestSuite(
    unittest.defaultTestLoader.discover(str(Path(__file__).parent), pattern=pattern)
    for pattern in ('test_native_mcp.py', 'test_runyte.py', 'test_native_pty.py',
                    'test_workspace_readiness.py')
)
result = unittest.TextTestRunner(verbosity=2).run(suite)
raise SystemExit(0 if result.wasSuccessful() and result.testsRun and not result.skipped else 1)
