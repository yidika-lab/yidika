# Security policy

## Supported versions

Yidika is pre-1.0. Security fixes land on `main` and ship in the next release; only the latest release is supported.

| Version | Supported |
|---|---|
| latest release | yes |
| older releases | no |

## Reporting a vulnerability

Do not open a public issue for a security problem.

Report it privately through GitHub: [Security, Report a vulnerability](https://github.com/yidika-lab/yidika/security/advisories/new). Include:

- the affected component (interpreter, AOT backend and its embedded C runtime, `net` server, installers, GUI engine);
- a minimal `.yk` program or request that reproduces it;
- the version (`yidi --version`) and OS.

You will get an acknowledgement within 7 days. Once a fix is ready, it is released and a GitHub Security Advisory is published, crediting you unless you ask otherwise.

## Scope

In scope: memory safety in the C runtime and FFI layer, request smuggling or header injection in the `net` server, path traversal in static file serving, installer integrity (checksum bypass), and crashes reachable from untrusted network input.

Out of scope: a `.yk` program you wrote yourself doing something harmful on your own machine.
