# Operator docs

Start here to deploy, run and repair a log. Each entry links the only source for its facts.

| Topic | Document | What it gives you |
| --- | --- | --- |
| Storage under the log | [Volume requirements](architecture/volume-requirements.md) | Requirements V1–V11 that the volume must meet for an acknowledgment to mean durable, with a test for each. |
| Qualified stack | [Qualification record](evidence/m7/qualification.md) | The Firecracker stack of [ADR 0059](decisions/0059-m7-qualifies-firecracker-on-a-dedicated-nvme-host.md), its fingerprint, its failure domain and the result of each requirement. Status: not qualified; the whole record reruns in the M7 requalification. |
| Handoff | [Handoff record](evidence/m7/handoff.md) | The surviving volume, the writer and reader mount paths, guest sizing, and the procedure after the guest ends, sealed or not. |
| Server command line | [Server README](../segment-log-server/README.md) | Flags, defaults, signals and exit statuses. The process model is [ADR 0058](decisions/0058-one-server-process-serves-one-log.md); the reader mode is [ADR 0060](decisions/0060-the-server-reads-a-sealed-log-with-read-only.md). |
| Options and defaults | [api.md](architecture/api.md#target-signatures-and-defaults) | Every `Options` field and its default. Each has a server flag. |
| Repair | [Operator repair](architecture/api.md#operator-repair) | `segment-log-server repair verify` and `truncate`: what each changes, its flags, exit codes and JSON output ([ADR 0065](decisions/0065-the-repair-tool-truncates-only-the-newest-segment.md)). |
| Upgrades | [ADR 0064](decisions/0064-the-0-1-compatibility-promise.md) | The compatibility promise from 0.1: which format and protocol versions a build keeps, and why an upgrade of the format is one-way. |

## Rules to keep

- **One writer.** One server process serves one log directory, and its `LOCK` keeps a second writer out. Only one kernel may have the volume at a time ([V5](architecture/volume-requirements.md#v5-one-writing-kernel)).
- **Restart after exit status 2.** The writer was poisoned by an I/O error. A restart opens the log again, and open recovers it ([ADR 0004](decisions/0004-io-errors-poison-the-writer.md)).
- **Plan capacity.** The log deletes no history, and a full volume poisons the writer ([V9](architecture/volume-requirements.md#v9-capacity-and-a-timely-alert), [ADR 0037](decisions/0037-retain-complete-logs-through-current-milestones.md)).
- **Check the stack before every open.** A stack that differs from its qualified fingerprint has no evidence behind it ([V10](architecture/volume-requirements.md#v10-the-running-stack-matches-the-qualified-fingerprint)).
- **Guard the socket.** The protocol has no authentication. The socket's mode and directory, or the vsock topology, decide who can append and seal ([ADR 0058](decisions/0058-one-server-process-serves-one-log.md)).
