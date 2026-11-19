# Key index page alignment

ADR 0052 rejected aligned key index pages (option B) without a measurement. This record measures what a cold key index lookup costs at an aligned and at an unaligned offset.

A lookup in a sealed segment opens the file, reads one 4,096-byte page at its file position, and closes the file (`segment-log-core/src/storage/real.rs`). Format version 1 puts no page on a 4 KiB boundary. `scripts/key-page-alignment.py` copies that I/O on 2,000 distinct random pages of an 8 GiB file, after dropping the page cache. Each variant ran 5 times, in a rotating order. The unaligned offset is random within the page.

[Format evidence run 43471859438](https://github.com/ironflowx/segment-log/actions/runs/43471859438), at `77270f4` on a hosted ubuntu-24.04 runner. [`environment.txt`](https://github.com/ironflowx/segment-log-evidence/blob/dc1a230a4b0e7bd70fec1963473ddd7cb2733c1c/format/key-page-alignment/environment.txt) records the kernel, the device (local NVMe, ext4) and a readahead of 128 KiB. A hosted runner is a shared virtual machine; these are measurements under its conditions.

Medians over the 5 trials ([`summary.tsv`](https://github.com/ironflowx/segment-log-evidence/blob/dc1a230a4b0e7bd70fec1963473ddd7cb2733c1c/format/key-page-alignment/summary.tsv); every run in [`runs.tsv`](https://github.com/ironflowx/segment-log-evidence/blob/dc1a230a4b0e7bd70fec1963473ddd7cb2733c1c/format/key-page-alignment/runs.tsv)):

| Variant | p50 | p90 | p99 | Device reads | Device KiB |
| --- | --- | --- | --- | --- | --- |
| aligned, default readahead | 131.9 µs | 149.7 µs | 276 µs | 2,002 | 8,008 |
| unaligned, default readahead | 135.8 µs | 152.3 µs | 250 µs | 2,001 | 16,004 |
| aligned, readahead off | 132.1 µs | 149.8 µs | 290 µs | 2,002 | 8,008 |
| unaligned, readahead off | 135.9 µs | 154.5 µs | 303 µs | 2,002 | 15,996 |

- An unaligned page costs the same number of device reads, because the kernel merges the two OS pages into one request, but it moves twice the bytes.
- Its p50 is about 3% (4 µs) higher. The p99 differences are within the run-to-run spread.
- Readahead does not change random page reads: the kernel did not read ahead in either mode.

To rerun: `scripts/key-page-alignment.sh DATA_DIR OUT_DIR` on Linux with passwordless sudo.
