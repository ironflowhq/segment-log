# segment-log-core

Embedded append-only segment log for Ironflow. The byte layout is format version 3, the baseline of 0.1, the first production release. From the 0.1 tag, every later build reads every format version from 3, and the API follows 0.x semver ([ADR 0064](https://github.com/ironflowx/segment-log/blob/v0.1.1/docs/decisions/0064-the-0-1-compatibility-promise.md)).

The API's meaning, errors and defaults are in [api.md](https://github.com/ironflowx/segment-log/blob/v0.1.1/docs/architecture/api.md), the only source for them.
