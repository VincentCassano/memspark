# Security Policy

## Supported Versions

The active `main` branch is the only supported development line until MemSpark reaches a stable release.

## Reporting a Vulnerability

Do not open a public issue for a suspected vulnerability involving privilege escalation, unsafe process handling, or path traversal. Report it privately to the maintainer through the repository security contact once the GitHub repository is created.

Please include:

- MemSpark version or commit hash.
- Windows version.
- Reproduction steps.
- Expected and actual behavior.
- Relevant report or developer log excerpt with private data removed.

## Security Boundary

MemSpark does not inject code into other processes and does not modify other process memory. It uses Windows process, working-set, and system memory-list APIs. Administrator elevation is requested only for operations that require it.

MemSpark can close or terminate non-system background processes when the 25% memory target is not reached by working-set trimming and system memory release. This behavior is visible in the report and developer log.
