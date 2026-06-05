# Contributing to MemSpark

MemSpark is intentionally small: one optimization goal, one core library, one CLI, and one Slint UI. Contributions should keep that shape.

## Development Setup

Install the stable Rust toolchain and use a Windows environment for full functionality. Non-Windows builds can check most Rust code paths, but Windows API behavior must be validated on Windows.

Useful commands:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
cargo test
cargo build --release
```

## Scope Rules

- Keep `memspark-core` as the boundary for Windows API calls and optimization behavior.
- Do not add extra optimization modes unless the project direction changes explicitly.
- Keep user-facing claims measurable. Do not promise impossible memory results on every machine.
- Do not add DLL injection, remote thread injection, driver loading, or hidden background scheduling.
- Keep report and developer log fields backward-compatible where practical.

## Pull Request Checklist

- Explain the behavior change and risk.
- Include screenshots for UI changes.
- Include before/after report or log samples for optimizer changes.
- Run formatting, clippy, tests, and release build before requesting review.

## Security-Sensitive Changes

Changes around process termination, administrator elevation, Windows privileges, or report/log paths need extra care. Explain the safety boundary and failure behavior in the pull request.
