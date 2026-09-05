# tungstate

A reconciliation controller for filesystems.

You declare the shape a folder should have. Tungstate watches it, detects drift
(misplaced files, naming violations, duplicates) and reconciles it safely and
continuously. It also moves files between machines durably: copy one, verify it,
delete the source, reclaim the space, next file, surviving sleep and network loss.

Status: design complete, no code yet. See `docs/DESIGN.md` for the full design and
`docs/SYLLABUS.md` for the build order.

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
