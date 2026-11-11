"""The command installer skips tools that are already present and does not prompt."""

import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts" / "ci-ensure-commands.sh"


def write_exe(path, text):
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IEXEC)


class EnsureCommands(unittest.TestCase):
    def run_script(self, args, uid, extra=None):
        with tempfile.TemporaryDirectory() as directory:
            bindir = Path(directory) / "bin"
            bindir.mkdir()
            log = Path(directory) / "apt.log"
            write_exe(bindir / "id", '#!/bin/sh\n[ "$1" = "-u" ] && echo "$FAKE_UID" && exit 0\nexit 1\n')
            write_exe(bindir / "sudo", "#!/bin/sh\nexit 1\n")
            write_exe(bindir / "apt-get", '#!/bin/sh\nprintf "%s\\n" "$*" >> "$APT_LOG"\nexit 0\n')
            present = bindir / "iostat"
            write_exe(present, "#!/bin/sh\nexit 0\n")
            env = os.environ.copy()
            env["PATH"] = str(bindir)
            env["FAKE_UID"] = str(uid)
            env["APT_LOG"] = str(log)
            if extra:
                env.update(extra)
            result = subprocess.run(
                [str(SCRIPT), *args],
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                timeout=5,
            )
            recorded = log.read_text() if log.exists() else ""
            return result, recorded

    def test_present_command_does_not_call_apt(self):
        result, recorded = self.run_script(["iostat:sysstat"], uid=0)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(recorded, "")

    def test_missing_command_as_root_installs_that_package_once(self):
        result, recorded = self.run_script(
            ["iostat:sysstat", "missing-tool:fakepkg", "other-missing:fakepkg"],
            uid=0,
        )
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("installing missing tools: fakepkg", result.stdout)
        install = next(line for line in recorded.splitlines() if line.startswith("install "))
        self.assertIn("fakepkg", install)
        self.assertNotIn("sysstat", install)
        self.assertEqual(install.count("fakepkg"), 1)

    def test_absolute_command_that_exists_is_present(self):
        with tempfile.TemporaryDirectory() as directory:
            tool = Path(directory) / "time"
            write_exe(tool, "#!/bin/sh\nexit 0\n")
            result, recorded = self.run_script([f"{tool}:time"], uid=0)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(recorded, "")

    def test_missing_command_without_sudo_fails_before_apt(self):
        result, recorded = self.run_script(["missing-tool:fakepkg"], uid=1000)
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("passwordless sudo", result.stdout)
        self.assertEqual(recorded, "")

    def test_bad_spec_is_rejected(self):
        result, _recorded = self.run_script(["only-a-command"], uid=0)
        self.assertEqual(result.returncode, 2, result.stdout)

    def test_no_arguments_is_rejected(self):
        result = subprocess.run(
            [str(SCRIPT)],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=5,
        )
        self.assertEqual(result.returncode, 2, result.stdout)


if __name__ == "__main__":
    unittest.main()
