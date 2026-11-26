# Security

If you find a security problem, please don't open a public issue. Use [Report a vulnerability](https://github.com/ElDiablo66/anonymous-vrx-app/security/advisories/new) under the Security tab, which reaches me privately.

The cases that matter most here:

- anything that could expose a VRChat sign-in: the password, a two-factor code or the auth cookie kept in Windows Credential Manager;
- an update file the app would install although its signature doesn't match;
- content from VRChat (names, bios, world descriptions) that runs as code inside the window;
- a crafted log file or path that makes the app read or write outside its own folders.

Never send me your password, codes or cookie, not even to prove a bug.

## Watch out for fake copies

The only official downloads are on this repository's [releases page](https://github.com/ElDiablo66/anonymous-vrx-app/releases). Each release lists the installer's SHA-256. A copy from anywhere else may have been changed to steal VRChat accounts. Because the code is under the GPL, anyone who hands out a changed build has to publish its source, so ask for it.

## Supported versions

Only the [latest release](https://github.com/ElDiablo66/anonymous-vrx-app/releases/latest) gets fixes. A fix ships as a new release, and the built-in updater picks it up.

## What happens after you report

This is a one-person project, so I answer on a best-effort basis, usually within 7 days. I'll confirm the problem, tell you which release will carry the fix and credit you in the release notes unless you'd rather not be named.
