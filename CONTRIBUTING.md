# Contributing

Use [TASKS.md](TASKS.md) for the local backlog. Keep each feature in a separate commit, and update its task status and implementing commit after verification. Start with a focused issue or change. Keep document/math code independent of UI and input backends. Add tests for geometry, file compatibility, and state-transition bugs. Run formatting, clippy, and tests before a pull request.

Tablet bug reports should include OS, tablet model, driver version, Windows Ink setting, display scaling, monitor mapping, and which diagnostics are missing. Attach only a minimal non-sensitive drawing when necessary.

Performance changes should include a reproducible workload and before/after measurements. Optimize after profiling. Keep unsafe code confined to documented platform boundaries.

Contributions are provided under GPL-3.0-or-later, consistent with the project license. Do not contribute proprietary code, artwork, icons, or copied interfaces.
