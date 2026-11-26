# Contributing

Thanks for wanting to help. Bug reports and ideas go in [issues](https://github.com/ElDiablo66/anonymous-vrx-app/issues). For anything bigger than a small fix, open an issue first so we can agree on the shape of it before you write code.

## Set-up

You need Rust (stable, MSVC), Node.js 24 and the WebView2 runtime that ships with Windows 11.

```powershell
npm install
```

```powershell
npm run tauri dev
```

`npm run dev` on its own opens the interface in a browser tab on made-up demo data. That is usually enough for layout work and needs no VRChat account.

## Before you open a pull request

```powershell
npm run build
```

```powershell
cargo test --manifest-path src-tauri/Cargo.toml
```

The Rust tests talk to a local stand-in server with invented accounts. They never contact VRChat and need no credentials.

## Where things are

| Part | Where |
| --- | --- |
| HTTP client, sign-in, rate limiting, websocket events, data types | `src-tauri/src/vrchat/` |
| The signed-in session, websocket loop, caches | `src-tauri/src/hub.rs` |
| SQLite history | `src-tauri/src/store.rs` |
| Reading VRChat's log files (read only) | `src-tauri/src/gamelog.rs` |
| Windows credential store | `src-tauri/src/secrets.rs` |
| Credits and licence texts the installer carries | `src-tauri/resources/licences/` |
| The window (React) | `src/` |
| Logo, installer art, README pictures | `Design/` |

## Rules

1. Never put a VRChat password, two-factor code, auth cookie or API key in code, tests, logs, issues or screenshots. Tests use the stand-in server.
2. Be gentle with VRChat's servers. Every request goes through the one gate in `src-tauri/src/vrchat/` that spaces requests out and backs off on a 429. No polling loops that bypass it.
3. Original work only: no code, pictures or text copied from other VRChat tools or from VRChat itself, unless its licence allows it and it gets a line in `THIRD-PARTY.md`.
4. Your data stays on your PC. No telemetry, no server of our own, nothing sent anywhere except VRChat's own API and the services a user switches on.

## Style

- Interface text is sentence case, plain and short. Error messages say what happened and what to do.
- Commit messages say what changed in one plain sentence.

## Licence

By contributing you agree that your work is released under the [GNU GPL version 3](LICENSE).
