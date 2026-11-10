import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / 'scripts/m3-verify-roll-syscalls.py'
TRACE = ROOT / 'scripts/fixtures/m3-roll-syscalls.trace'
NEW = r'00000000000000000001\.seg'


class RollOrder(unittest.TestCase):
    def check_trace(self, lines):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'calls.trace'
            path.write_text('\n'.join(lines) + '\n')
            return subprocess.run([sys.executable, str(SCRIPT), str(path)],
                                  stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                  text=True, timeout=5)

    def trace(self):
        return TRACE.read_text().splitlines()

    def roll_line(self, lines, pattern):
        start = next(i for i, x in enumerate(lines) if 'PHASE roll' in x)
        return next(i for i in range(start, len(lines)) if re.search(pattern, lines[i]))

    def reject(self, lines, message):
        result = self.check_trace(lines)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn(message, result.stdout)

    def test_captured_roll_is_accepted(self):
        """Given the calls captured on Linux, when verified, then the roll protocol passes.

        Cases: GROUP-FORMATION, OPEN-DIR
        """
        result = self.check_trace(self.trace())
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn('roll: fdatasync=2 fsync=1 success', result.stdout)

    def test_missing_directory_sync_is_rejected(self):
        """Given a roll without its directory fsync, when verified, then it rejects it.

        Cases: OPEN-DIR
        """
        lines = self.trace()
        del lines[self.roll_line(lines, r' fsync\(\d+<[^>]*/log>\)')]
        self.reject(lines, 'roll: syncs')

    def test_directory_sync_after_the_group_frame_is_rejected(self):
        """Given the directory fsync moved behind the group frame flush, when verified, then it rejects it.

        Cases: OPEN-DIR
        """
        lines = self.trace()
        sync = lines.pop(self.roll_line(lines, r' fsync\(\d+<[^>]*/log>\)'))
        flushes = [i for i, x in enumerate(lines) if re.search(rf'fdatasync\(\d+<[^>]*{NEW}>', x)]
        lines.insert(flushes[1] + 1, sync)
        self.reject(lines, 'roll: no group frame write')

    def test_group_frame_before_the_directory_sync_is_rejected(self):
        """Given the group frame written before the directory fsync, when verified, then it rejects it.

        Cases: OPEN-DIR
        """
        lines = self.trace()
        sync = lines.pop(self.roll_line(lines, r' fsync\(\d+<[^>]*/log>\)'))
        frame = self.roll_line(lines, rf'write\(\d+<[^>]*{NEW}>, "(?!SGLG)')
        lines.insert(frame + 1, sync)
        self.reject(lines, 'roll: no group frame write')

    def test_header_flush_after_the_directory_sync_is_rejected(self):
        """Given the new segment's flush behind the directory fsync, when verified, then it rejects it.

        Cases: OPEN-DIR
        """
        lines = self.trace()
        flush = lines.pop(self.roll_line(lines, rf'fdatasync\(\d+<[^>]*{NEW}>'))
        sync = self.roll_line(lines, r' fsync\(\d+<[^>]*/log>\)')
        lines.insert(sync + 1, flush)
        self.reject(lines, 'roll: no fsync of the log directory')

    def test_directory_sync_of_the_parent_is_rejected(self):
        """Given the parent synced in place of the log directory, when verified, then it rejects it.

        Cases: OPEN-DIR
        """
        lines = self.trace()
        index = self.roll_line(lines, r' fsync\(\d+<[^>]*/log>\)')
        lines[index] = re.sub(r'/log>', '>', lines[index])
        self.reject(lines, 'roll: syncs')

    def test_failed_directory_sync_is_rejected(self):
        """Given a directory fsync that fails, when verified, then it rejects it.

        Cases: OPEN-DIR
        """
        lines = self.trace()
        index = self.roll_line(lines, r' fsync\(\d+<[^>]*/log>\)')
        lines[index] = re.sub(r'= 0 ', '= -1 EIO (Input/output error) ', lines[index])
        self.reject(lines, 'sync did not return 0')

    def test_segment_created_without_o_creat_is_rejected(self):
        """Given a new segment opened without O_CREAT, when verified, then it rejects it.

        Cases: GROUP-FORMATION
        """
        lines = self.trace()
        index = self.roll_line(lines, rf'openat\(.*{NEW}')
        lines[index] = lines[index].replace('O_CREAT|', '')
        self.reject(lines, 'creation of the new segment')

    def test_missing_header_is_rejected(self):
        """Given a new segment that is flushed without its header, when verified, then it rejects it.

        Cases: GROUP-FORMATION
        """
        lines = self.trace()
        index = self.roll_line(lines, rf'write\(\d+<[^>]*{NEW}>, "SGLG')
        del lines[index]
        self.reject(lines, 'roll: segment writes')

    def test_write_to_the_old_segment_during_roll_is_rejected(self):
        """Given the group frame written to the old segment, when verified, then it rejects it.

        Cases: GROUP-FORMATION
        """
        lines = self.trace()
        index = self.roll_line(lines, rf'write\(\d+<[^>]*{NEW}>, "(?!SGLG)')
        lines[index] = lines[index].replace('00000000000000000001.seg', '00000000000000000000.seg', 1)
        self.reject(lines, 'roll: segment writes')

    def test_extra_flush_in_a_commit_is_rejected(self):
        """Given a second fdatasync in the commit after the roll, when verified, then it rejects it.

        Cases: GROUP-FORMATION
        """
        lines = self.trace()
        start = next(i for i, x in enumerate(lines) if 'PHASE commit-1' in x)
        index = next(i for i in range(start, len(lines)) if 'fdatasync(' in lines[i])
        lines.insert(index, lines[index])
        self.reject(lines, 'commit-1: syncs')

    def test_calls_split_across_threads_are_rejoined(self):
        """Given the directory fsync split into unfinished and resumed lines, when verified, then it passes.

        Cases: OPEN-DIR
        """
        lines = self.trace()
        index = self.roll_line(lines, r' fsync\(\d+<[^>]*/log>\)')
        pid, time, call = lines[index].split(None, 2)
        head, tail = call.split(')', 1)
        lines[index:index + 1] = [f'{pid}  {time} {head} <unfinished ...>',
                                  f'{pid}  {time} <... fsync resumed>){tail}']
        result = self.check_trace(lines)
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_markers_out_of_order_are_rejected(self):
        """Given a trace without the roll marker, when verified, then it rejects it.

        Cases: GROUP-FORMATION
        """
        lines = [x for x in self.trace() if 'PHASE roll' not in x]
        self.reject(lines, 'phase markers')


if __name__ == '__main__':
    unittest.main()
