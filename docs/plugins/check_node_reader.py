# SPDX-License-Identifier: MPL-2.0
"""Deterministic Node fixture reader checks; no Node, pipes or schema dependency."""
import unittest

from node_reader import ResponseReader, stderr_snapshot


class FakeInput:
    def __init__(self, chunks):
        self.chunks = list(chunks)
        self.now = 0
        self.waits = []
        self.validated = []
        self.diagnostic_reads = 0
        self.stderr = b''
        self.reader = ResponseReader(self.wait, self.read, self.validated.append,
                                     self.diagnostics, 1024, lambda: self.now)

    def wait(self, remaining):
        self.waits.append(remaining)
        if not self.chunks or self.chunks[0][0] > remaining:
            self.now += remaining
            return False
        delay, self.next_chunk = self.chunks.pop(0)
        self.now += delay
        return True

    def read(self, maximum):
        assert len(self.next_chunk) <= maximum
        return self.next_chunk

    def diagnostics(self):
        self.diagnostic_reads += 1
        return None, self.stderr


class ReaderTests(unittest.TestCase):
    def test_registration_allows_cold_start_without_widening_ordinary_replies(self):
        source = FakeInput([(4, b'{"type":"register"}\n'), (4, b'{"type":"response"}\n')])
        self.assertEqual(source.reader.registration(0), {'type': 'register'})
        with self.assertRaisesRegex(AssertionError, 'timed out; phase=response'):
            source.reader.read(source.now + 3)
        self.assertEqual(source.waits, [10, 3])

    def test_registration_charges_launch_time_and_fragments_to_one_budget(self):
        source = FakeInput([(3, b'{"type":'), (4, b'"register"}\n')])
        source.now = 4  # Process creation and hello already consumed four seconds.
        with self.assertRaisesRegex(AssertionError, 'launch_elapsed=10.000s'):
            source.reader.registration(0)
        self.assertEqual(source.waits, [6, 3])
        self.assertEqual(source.now, 10)

    def test_registration_silence_eof_and_expired_launch_have_distinct_failures(self):
        silent = FakeInput([])
        with self.assertRaisesRegex(AssertionError, 'timed out; phase=registration'):
            silent.reader.registration(0)
        self.assertEqual(silent.now, 10)
        with self.assertRaisesRegex(AssertionError, 'exited before a response'):
            FakeInput([(1, b'')]).reader.registration(0)
        expired = FakeInput([(0, b'{"type":"register"}\n')])
        expired.now = 11
        with self.assertRaisesRegex(AssertionError, 'launch_elapsed=11.000s'):
            expired.reader.registration(0)
        self.assertEqual(expired.waits, [])

    def test_readiness_after_deadline_does_not_admit_a_late_frame(self):
        source = FakeInput([(0, b'{"type":"register"}\n')])
        def late(_remaining):
            source.now = 11
            return True
        source.reader.wait = late
        with self.assertRaisesRegex(AssertionError, 'timed out'):
            source.reader.registration(0)
        self.assertEqual(source.validated, [])

    def test_coalesced_deadline_reply_is_returned_without_more_descriptor_readiness(self):
        source = FakeInput([(0, b'{"type":"request"}\n{"type":"response"}\n')])
        self.assertEqual(source.reader.read(3), {'type': 'request'})
        self.assertEqual(source.reader.read(9, 'publication deadline'), {'type': 'response'})
        self.assertEqual(source.waits, [3])
        self.assertEqual(source.diagnostic_reads, 0)
        self.assertEqual(len(source.validated), 2)

    def test_split_utf8_and_frames_keep_one_absolute_deadline(self):
        source = FakeInput([(1, b'{"text":"\xc3'), (1, b'\xa9"}\n')])
        self.assertEqual(source.reader.read(3), {'text': '\u00e9'})
        self.assertEqual(source.waits, [3, 2])
        self.assertEqual(source.diagnostic_reads, 0)

    def test_partial_reads_do_not_restart_the_response_budget(self):
        source = FakeInput([(2, b'{'), (2, b'"later":true}\n')])
        with self.assertRaisesRegex(AssertionError, 'timed out; phase=registration; elapsed=3.000s'):
            source.reader.read(3, 'registration')
        self.assertEqual(source.waits, [3, 1])
        self.assertEqual(source.now, 3)
        self.assertEqual(source.diagnostic_reads, 1)

    def test_timeout_reports_bounded_partial_output_and_stderr(self):
        source = FakeInput([(0, b'x' * 400)])
        source.stderr = b'e' * 1500 + b'private-tail'
        with self.assertRaises(AssertionError) as failed:
            source.reader.read(3, 'registration')
        message = str(failed.exception)
        self.assertIn('child_status=None; partial_bytes=400; queued_messages=0', message)
        self.assertIn("partial_prefix=" + repr(b'x' * 256), message)
        self.assertIn("stderr_prefix=" + repr(b'e' * 1024), message)
        self.assertNotIn('private-tail', message)
        self.assertLess(len(message), 1600)

    def test_eof_and_invalid_frames_keep_their_distinct_failures(self):
        with self.assertRaisesRegex(AssertionError, 'Node exited before a response'):
            FakeInput([(0, b'')]).reader.read(3)
        with self.assertRaises(ValueError):
            FakeInput([(0, b'not-json\n')]).reader.read(3)
        with self.assertRaisesRegex(AssertionError, 'frame exceeds'):
            FakeInput([(0, b'x' * 1024 + b'\n')]).reader.read(3)

    def test_stderr_snapshot_performs_only_one_bounded_nonblocking_read(self):
        calls = []
        def blocked(maximum):
            calls.append(maximum)
            raise BlockingIOError()
        self.assertEqual(stderr_snapshot(blocked), b'')
        self.assertEqual(calls, [1024])
        def failed(_maximum):
            raise OSError(9, 'unrestricted diagnostic must not be copied')
        self.assertEqual(stderr_snapshot(failed), b'<stderr unavailable: errno=9>')


if __name__ == '__main__':
    unittest.main(verbosity=2)
