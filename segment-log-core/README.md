# segment-log-core

Embedded append-only segment log for Ironflow. Pre-release: the byte layout is format version 1 since M5, and until the first production release a format change increments the version without a reader for older ones ([ADR 0053](../docs/decisions/0053-format-changes-need-no-older-reader-before-production.md)). The API can change.
