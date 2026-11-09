import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / 'scripts/m1-verify-syscalls.py'
TRACE = ROOT / 'docs/evidence/m1/slice10/linux/syscalls.trace'


class SyncTargets(unittest.TestCase):
    def check_trace(self, text):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'calls.trace'
            path.write_text(text)
            return subprocess.run([sys.executable, str(SCRIPT), str(path)],
                                  stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                  text=True, timeout=5)

    def test_original_calls_are_accepted(self):
        """Given the captured calls, when verified, then the protocol passes.

        Cases: GROUP-FORMATION, OPEN-DIR
        """
        result = self.check_trace(TRACE.read_text())
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_unrelated_file_cannot_replace_synced_targets(self):
        """Given correct counts but wrong files, when verified, then it rejects them.

        Cases: GROUP-FORMATION, OPEN-DIR
        """
        text = re.sub(r'((?:fsync|fdatasync)\(\d+<)[^>]+>',
                      r'\1/wrong/unrelated-file>', TRACE.read_text())
        result = self.check_trace(text)
        self.assertNotEqual(result.returncode, 0, result.stdout)

    def test_parent_sync_cannot_be_replaced_by_another_log_directory_sync(self):
        """Given a repeated log sync replacing its parent, when verified, then it rejects it.

        Cases: OPEN-DIR
        """
        text = TRACE.read_text()
        segment = PurePosixPath(re.search(r'fdatasync\(\d+<([^>]+\.seg)>', text)[1])
        text = text.replace(f'<{segment.parent.parent}>', f'<{segment.parent}>')
        result = self.check_trace(text)
        self.assertNotEqual(result.returncode, 0, result.stdout)


if __name__ == '__main__':
    unittest.main()
