# Outside software this project uses

None of this is stored in the repository. The table says which licence each piece is published under, as stated on its makers' own pages on 2026-11-26.

## Built with

| What | Used for | Licence its makers state |
|---|---|---|
| Tauri v2 | The app window | Apache License 2.0 or MIT |
| React | The interface | MIT License |
| Instrument Sans, Barlow Condensed, Yellowtail, Rajdhani, Orbitron, Outfit, Space Grotesk | The fonts | SIL Open Font License 1.1, Apache License 2.0 |

The Rust crates and npm packages are listed with their versions in `src-tauri/Cargo.lock` and `package-lock.json`. Each carries its own licence.

## Services

- VRChat's web API and websocket. Not an official interface for outside tools; VRChat's Terms of Service and Community Guidelines apply to anyone using the app.
- Avatar search asks community avatar databases chosen in Settings. Only the search words are sent.

## This repository

Anonymous VRX itself is under the GNU General Public License version 3 only (GPL-3.0-only). The full text is in [LICENSE](LICENSE). The licence texts of the fonts and libraries above ship with the installer, in its `licences` folder.
