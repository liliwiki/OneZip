# Security Policy

OneZip reads, writes, and deletes files. "Extract and delete the archive" truncates or punches holes in the archive while extracting, and extracting while downloading releases downloaded data as it is used. It also registers a shell extension that Windows Explorer loads.

## Reporting a vulnerability

1. Do not publish working exploit details, crafted archives that trigger the problem, credentials, or private files in a public Issue.
2. Prefer [GitHub private vulnerability reporting](https://github.com/liliwiki/OneZip/security/advisories/new).
3. Include the affected OneZip version, the operation and archive format, the Windows version, reproduction conditions, the expected impact, and a minimal non-sensitive log.
4. For vulnerabilities in HatchZip, Peel, sevenz-rust2, lzma-rust2, or another upstream component, also report the issue to that project.

Problems that matter most here:

- Writing outside the chosen output folder (path traversal, links, reserved names).
- Losing data when deleting archives in place, for example a crash or power cut leaving neither the archive nor the extracted file intact.
- Anything that lets an archive or a download run code, or makes the shell extension misbehave inside Explorer.

## Operational safety

- Keep a backup of archives you cannot replace before extracting them in place.
- Treat downloaded archives as untrusted until their source has been checked.
- Check the SHA-256 published on the Release page before running a downloaded build.
- The development certificate `CN=OneZip Dev` that the installer trusts is used only to sign the sparse package. Remove it from "Trusted People" (`certlm.msc`) if you uninstall OneZip and no longer need it.

The MIT License provides the software without warranty. Users remain responsible for their data and for the archives they open.
