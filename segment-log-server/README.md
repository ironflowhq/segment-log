# segment-log-server

The socket server for the Ironflow segment log. It serves one log directory over a Unix socket or, on Linux, a vsock port. It passed milestone M6 ([completion record](../docs/evidence/m6/completion.md)), and M7 validates it on the guest deployment. From the 0.1 tag, its flags, exit statuses and process behavior are public API, and it speaks every protocol version from 2 ([ADR 0064](../docs/decisions/0064-the-0-1-compatibility-promise.md)).

```sh
segment-log-server --dir <path> (--unix <socket-path> | --vsock-port <port>) \
  [--linger-us <n>] [--group-byte-cap <n>] [--roll-bytes <n>] [--queue-capacity <n>] \
  [--pending-byte-budget <n>] [--max-connections <n>] [--read-only]
```

Omitted options keep the defaults in [api.md](../docs/architecture/api.md#target-signatures-and-defaults). `--max-connections` defaults to 64. `--read-only` serves a sealed log through `LogReader` and writes nothing ([ADR 0060](../docs/decisions/0060-the-server-reads-a-sealed-log-with-read-only.md)). `--vsock-port` listens on any CID, so a guest's peer and the host loopback (CID 1) both reach it. The bytes on the socket are in [protocol.md](../docs/architecture/protocol.md), and the process behavior is in [ADR 0058](../docs/decisions/0058-one-server-process-serves-one-log.md).

A successful `Seal` is answered, then the server closes every connection and exits 0. `SIGTERM` or `SIGINT` stops it the same way without sealing. Either way, the server first answers the requests it has read.

Exit status 1 means the log did not open, the socket did not bind, or a flag was invalid. The reason is on stderr. Exit status 2 means the writer was poisoned: the server answered the requests it had read, and a restart recovers the log.
