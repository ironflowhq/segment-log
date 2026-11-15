# segment-log-server

The socket server for the Ironflow segment log, in development for milestone M6. It serves one log directory over a Unix socket or, on Linux, a vsock port. Do not depend on it yet.

```sh
segment-log-server --dir <path> (--unix <socket-path> | --vsock-port <port>) \
  [--linger-us <n>] [--group-byte-cap <n>] [--roll-bytes <n>] [--queue-capacity <n>] \
  [--pending-byte-budget <n>] [--max-connections <n>]
```

Omitted options keep the defaults in [api.md](../docs/architecture/api.md#target-signatures-and-defaults). `--max-connections` defaults to 64. `--vsock-port` listens on any CID, so a guest's peer and the host loopback (CID 1) both reach it. The bytes on the socket are in [protocol.md](../docs/architecture/protocol.md), and the process behavior is in [ADR 0058](../docs/decisions/0058-one-server-process-serves-one-log.md).

A successful `Seal` is answered, then the server closes every connection and exits 0. `SIGTERM` or `SIGINT` stops it the same way without sealing. Either way, the server first answers the requests it has read.

Exit status 1 means the log did not open, the socket did not bind, or a flag was invalid. The reason is on stderr. Exit status 2 means the writer was poisoned: the server answered the requests it had read, and a restart recovers the log.
