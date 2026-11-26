# Security Policy

Synector runs elevated by default, supervises processes, executes user-authored scripts and listens
on a local HTTP port. Security reports are taken seriously.

## Supported versions

Security fixes are applied to the latest published release only. Earlier releases do not receive
fixes; upgrade before reporting an issue that may already be fixed.

## Reporting a vulnerability

**Do not open a public issue for a security problem.**

Use GitHub's private vulnerability reporting:

1. Open the repository's **Security** tab.
2. Choose **Advisories** → **Report a vulnerability**.
3. Describe the impact and include reproduction steps.

If private reporting is not available, contact the maintainer directly on GitHub
([@psyattack](https://github.com/psyattack)) and ask for a private channel. Never include working
exploit code, credentials, personal data or unrelated user information in a public issue.

Please include:

- Synector version (shown in the app footer and in Settings → Updates);
- Windows version and build (`winver`);
- whether Synector was running elevated;
- the configuration needed to reproduce the problem (redact scripts, webhook payloads and paths
  that contain personal data);
- relevant log lines from `%LOCALAPPDATA%\Synector\logs\` with secrets removed;
- expected impact and, if possible, a minimal proof of concept.

## Scope

Reports are especially useful for:

- the updater: release selection, asset download, SHA-256 verification, self-replacement and silent
  installer updates;
- the named-pipe IPC surface (`\\.\pipe\Synector_Daemon`): authentication, request validation and
  privilege boundaries between the daemon, GUI and CLI;
- the local webhook server (`127.0.0.1:48123`): request handling, payload limits and endpoints;
- `SYN1:` share codes: checksum verification, bounded decoding, re-identification and bundling of
  presets and overlays;
- elevation and autostart: `runas`, Task Scheduler registration, the masked daemon copy
  (`WindowsServiceHost.exe`) and update-file handling;
- overlay processes and their command-line data;
- unintended disclosure of user secrets in logs, error messages or IPC responses.

## Out of scope

- Attacks that require an already-administrator attacker, physical access or a compromised Windows
  account.
- Bugs in Windows itself, third-party applications, Lua/PowerShell/Batch scripts written by the
  user, or filters/rules the user deliberately configured.
- Social engineering, spam and denial-of-service against third-party services.
- Input simulation being blocked or flagged by a game's anti-cheat system. Synector documents that
  synthetic input may be treated as automation; use it only where the application permits it.

## Safe harbor

Good-faith research is welcome. Keep testing on your own machine and data, do not access other
people's systems or data, do not degrade services, and give the maintainer reasonable time to fix
the issue before public disclosure. Reports that follow this policy will not be pursued.

## Response

This is a small project maintained on a best-effort basis; there is no guaranteed response time or
bounty. Acknowledged reports are investigated, fixed in a release where possible, and credited in
the release notes unless you ask to remain anonymous.
