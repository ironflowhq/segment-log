import pathlib, shutil, subprocess, sys, tempfile
# Run from the repository root. Each fault must fail at least one test of the driver.
src = pathlib.Path("scripts")
MUTANTS = {
    "peak ignores the start": ('    peak, at = total(), None\n', '    peak, at = 0, None\n'),
    "snapshot trusts the first read": ('        if not own and not remote:\n            return status\n', '        return status\n'),
    "lost events accepted": ('    if any(trial.get("losses", {}).values()):', '    if False:'),
    "replay mismatch accepted": ('        if traced and any(traced["end"][member] != exact[member] for member in RESIDENT):', '        if False:'),
    "remote events of other memories kept": ('    remote = [event for event in parse_events(remote_text) if event[2] == mm_id]', '    remote = parse_events(remote_text)'),
    "own events of another memory accepted": ('    if mm_ids - {mm_id}:', '    if False:'),
    "own remote-context events accepted": ('    if any(event[3] != 1 for event in own):', '    if False:'),
    "unknown trace lines skipped": ('            raise ValueError(f"trace: unexpected line {line!r}")', '            continue'),
    "swap counted as resident": ('RESIDENT = ("MM_FILEPAGES", "MM_ANONPAGES", "MM_SHMEMPAGES")', 'RESIDENT = ("MM_FILEPAGES", "MM_ANONPAGES", "MM_SHMEMPAGES", "MM_SWAPENTS")'),
    "pulse lower bound dropped": ('    if not size <= pulsed["transient_kb"] <= size + PULSE_SLACK_KB:', '    if not pulsed["transient_kb"] <= size + PULSE_SLACK_KB:'),
    "pin not checked": ('    if trial["readings"] and trial["cpu"] is not None and', '    if False and'),
    "owner known-size checks skipped": ('    if checks != want:', '    if False:'),
}
for name, (old, new) in MUTANTS.items():
    with tempfile.TemporaryDirectory() as d:
        d = pathlib.Path(d)
        for f in ("m1-exact-peaks.py", "m1-linux-phase-peaks.py", "test_m1_exact_peaks.py"):
            shutil.copy(src / f, d / f)
        text = (d / "m1-exact-peaks.py").read_text()
        assert text.count(old) == 1, name
        (d / "m1-exact-peaks.py").write_text(text.replace(old, new))
        r = subprocess.run([sys.executable, "-B", "-m", "unittest", "test_m1_exact_peaks"], cwd=d, capture_output=True, text=True)
        failing = r.stderr.count("\nFAIL:") + r.stderr.count("\nERROR:")
        print(f"{'caught' if r.returncode else 'MISSED'}\t{failing} failing\t{name}")
