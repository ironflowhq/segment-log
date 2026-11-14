# segment-log-server

The socket server for the Ironflow segment log, in development for milestone M6. It serves one log directory over a Unix socket. vsock comes later in M6. Do not depend on it yet.

```sh
segment-log-server --dir <path> --unix <socket-path> \
  [--linger-us <n>] [--group-byte-cap <n>] [--roll-bytes <n>] [--queue-capacity <n>] \
  [--pending-byte-budget <n>] [--max-connections <n>]
```

Omitted options keep the defaults in [api.md](../docs/architecture/api.md#target-signatures-and-defaults). `--max-connections` defaults to 64. The bytes on the socket are in [protocol.md](../docs/architecture/protocol.md), and the process behavior is in [ADR 0058](../docs/decisions/0058-one-server-process-serves-one-log.md).

Exit status 1 means the log did not open, the socket did not bind, or a flag was invalid. The reason is on stderr.
