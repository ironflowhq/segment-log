# CRC tables in unoptimized builds

Hosted Linux CI aborted three times in a row on one branch while the branches above it passed. This record finds the cause in `segment-log-core/src/codec.rs` and measures the fix.

## Result

**The CRC tables were copied at every lookup in an unoptimized build.** The slicing-by-8 tables were a `const`. An unoptimized build copies a `const` array where it is used, so each of the eight lookups for eight input bytes copied the whole 8 KiB table. That is 64 KiB of `memcpy` for 8 bytes of input, and 64 GiB for the 8 MiB group that one codec scenario encodes.

**A `static` removes the copies.** The scenario's processor time falls by about fifty times in a debug build on every host measured. The library's optimized code does not change.

| Host, debug build | Scenario before | Scenario after | Library tests before | Library tests after |
| --- | --- | --- | --- | --- |
| Mac host, Apple M4 | 0.94–1.11 s | 0.02 s | 3.90 s wall, 9.86 s processor | 1.56 s wall, 2.48 s processor |
| Linux container, aarch64 | 0.79 s | 0.01 s | 7.05–8.82 s wall, 15.6–18.3 s processor | 1.22 s wall, 2.91 s processor |
| Hosted Linux runner, AMD EPYC 7763 | over 10 s, aborted | 0.02 s | aborted after 10.4 s | 2.09 s wall |

The scenario is `binary_groups_round_trip_and_encoding_enforces_limits`. Each scenario time is three runs of that scenario alone.

## How it showed

CI runs [42757290131](https://github.com/ironflowhq/segment-log/actions/runs/42757290131) (two attempts) and [42758310337](https://github.com/ironflowhq/segment-log/actions/runs/42758310337) ended with `SIGABRT` about 10.4 s after the tests started. Every other test had passed by then. Only this scenario was unfinished, and its ten-second watchdog ended the process. In passing runs on other runners the same scenario took 1.0 to 1.7 s.

A temporary branch ran the test binary on a hosted runner and attached a debugger after seven seconds. [hosted-stack.txt](hosted-stack.txt) holds the frames: the scenario's thread is in `memcpy` under `crc_update`, under the 8 MiB `encode`. A second thread, in the seeded trace oracle, is in the same place under `decode`.

The runner had an AMD EPYC 7763. How long the copies take there depends on where the table and the stack lie, which changes with every build. That is why one branch failed each time and the branch above it passed.

## Why optimized builds do not change

`cargo rustc -p segment-log-core --release --lib -- --emit=asm -C codegen-units=1` on the Mac host gives 27,552 instruction lines before and after the change. They are the same sequence once local labels are replaced by one name. Only the table's own symbol differs. The benchmarks in the other issue 20 records therefore still describe the code.

## Limits

- The assembly comparison is for aarch64 on macOS. No x86-64 assembly was compared.
- The hosted "before" time is a lower bound, because the watchdog ended the run.
- This is a debug-build cost. It never reached a release build or a measurement.

## Reproduce

```sh
cargo test -p segment-log-core --lib --no-run
time target/debug/deps/segment_log_core-HASH --exact acceptance::codec::binary_groups_round_trip_and_encoding_enforces_limits
```

Run it at the parent commit and at this commit.
