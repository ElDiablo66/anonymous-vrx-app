# Anonymous VRX

<p align="center">
  <img src="Design/banner.svg" alt="Anonymous VRX. Your VRChat night, on one screen." width="100%">
</p>

<p align="center">
  <b>A VRChat companion for Windows that shows who is out tonight and where.</b><br>
  I built it from scratch in Rust, with a neon sunset look.
</p>

<p align="center">
  <img alt="Windows 10 and 11" src="https://img.shields.io/badge/Windows-10%20%7C%2011-ff4fa3?style=for-the-badge">
  <img alt="Rust" src="https://img.shields.io/badge/Built%20in-Rust-ff9a3c?style=for-the-badge">
  <img alt="Tauri v2" src="https://img.shields.io/badge/Tauri-v2-2de2e6?style=for-the-badge">
  <img alt="Your data stays on your PC" src="https://img.shields.io/badge/Your%20data-stays%20on%20your%20PC-c39bff?style=for-the-badge">
</p>

<p align="center">
  <a href="#get-it">Get it</a> ·
  <a href="#more-screens">More screens</a> ·
  <a href="#what-it-does">What it does</a> ·
  <a href="#privacy">Privacy</a> ·
  <a href="#known-limits">Known limits</a> ·
  <a href="#build-it-yourself">Build it</a> ·
  <a href="#credits">Credits</a>
</p>

<p align="center">
  <img src="Design/screens/around.webp" alt="The Around screen: friends grouped by the world they are in, favourites pinned on top, the activity feed on the right" width="100%">
</p>

<p align="center"><sub>My own app, with "Hide names and pictures" switched on. The grey bars and circles are where my friends' names and pictures would be.</sub></p>

## What this is

Anonymous VRX runs next to VRChat and answers the questions I kept digging through menus for:

- Who is out tonight, and which world are they in?
- Who is with them, and is there room for one more?
- What happened while I was away?
- Where was I last Tuesday at 2 am, and who else was there?

It all sits in one window, so you never have to open the VRChat website.

> This is an independent project. It is not made by, affiliated with or endorsed by VRChat Inc. It uses the same web API as the VRChat website, which VRChat does not officially support for outside tools, so parts of it can break without notice.

## Get it

1. Open [Releases](../../releases) and download the newest `Anonymous-VRX-...-setup.exe`.
2. Run it. Everything is inside that one file.
3. Sign in with your VRChat account inside the app.

You need 64-bit Windows 10 or 11 and a VRChat account. Nothing else has to be installed first.

It installs for your user only and needs no administrator rights. The installer is not code signed, so Windows SmartScreen warns the first time you run it: choose "More info", then "Run anyway".

From version 0.1.1 on the app looks for a newer release here when it starts and installs it; Settings, About has the switch for that. Each update file is signed, and the app only installs one whose signature matches. Version 0.1.0 has no updater: quit the app and run the newer installer over the old one once. Your history and settings stay. To remove it, use "Installed apps" in Windows Settings.

Where it keeps things:

| | |
|---|---|
| History, notes, settings | `%APPDATA%\app.anonymousvrx.desktop` |
| Sign-in cookie | Windows Credential Manager |
| Backups | `Documents\Anonymous VRX backups` |

## More screens

World history, read from VRChat's log files:

<img src="Design/screens/history.webp" alt="World history: every instance visited, when, and for how long" width="100%">

The Done tab of the Inbox:

<img src="Design/screens/inbox.webp" alt="The Inbox, Done tab, with answered notifications" width="100%">

## What it does

### Around right now

This is the screen the app opens on.

| | |
|---|---|
| Friends by room | Friends are grouped by the instance they share, with the world, access type, region, head count and how long each of them has been there |
| Favourites on top | Favourite friends are pinned above everyone else, one block per favourite group |
| Notifications | Windows tells you when a favourite comes online or leaves a private world for one you can join. You pick which favourite groups do that, give single friends an always or never rule, and set quiet hours or quiet while VRChat runs |
| Joining | Join, ask for an invite, invite yourself, or copy the link, each with one click |
| Right-click a friend | Ask for an invite, invite them to your world, or boop, from anywhere they are listed |

### Everyone

| | |
|---|---|
| Friend list | Sortable, with trust colours, VRC+ and 18+ marks, language flags and status colours. A Last seen column, from the app's own records, and a sort that puts the longest-quiet friends first |
| Old names | Hover a name to see what that friend used to be called |
| Friend tools | Send and answer friend requests, boop back, unfriend, move people between favourite groups, rename the groups |
| Block and mute | Block or mute someone from their profile, and see everyone you blocked or muted in one list, with a button to take each one back |
| Your status | Click your own picture to change it |

### History

VRChat forgets most of this. The app keeps it in a file on your PC.

| | |
|---|---|
| Activity feed | Who came online, left, changed world or status. Searchable, saved per account. Your own changes can be included with a switch |
| Friend log | Who was added, removed or renamed, including while the app was closed |
| World history | Every instance you visited, for how long, and who was there, read from VRChat's own log files |
| Photos | The photos VRChat's camera saved on your PC, each with the world it was taken in and who was in the instance, looked up in the world history. Search them by world or by who was in the instance at that moment, and up to a day. The files are not changed |
| Here | The instance you are in now and everyone in it, with friends marked |
| Going back | From any past instance you can send yourself an invite, copy a join link or open VRChat's join page. Opening a past visit asks VRChat once whether that instance is still open and how many are in it |

### Inbox

| | |
|---|---|
| Notices | Friend requests, invites, invite requests and group notices, with filters by kind |
| Answers | The app never answers for you. A Done tab keeps what you accepted, declined or dismissed |
| Invites | "Join in VRChat" on an invite takes you straight to that instance |
| Pop-ups | They still arrive when the window is closed to the tray |

### Worlds, avatars and groups

| | |
|---|---|
| Windows | Worlds, avatars and groups open inside the app, and a Back button retraces your steps |
| Groups | Member counts, posts, photos and open instances. You can join and leave from there |
| Favourites | Shows how full each favourite group is. The clean-up asks VRChat about each missing favourite before offering to remove it. Local favourites on your PC have no limit |
| New instance | Make your own instance of any world (Invite, Invite+, Friends, Friends+ or Public, in the region you pick) and join it |
| Notes | Private notes on people, avatars and worlds |
| My tags | Your own tags on avatars, like folders an avatar can be in more than one of. They filter My uploads and favourite avatars, search finds them, and Hide takes an avatar out of the lists without touching it on VRChat |
| My stars | Your own 1 to 5 star rating on any world, and whether the game log has seen you there, both usable as filters on favourite worlds and in world search. They stay on your PC |
| Search | People, worlds and avatars |
| Events | What is on now and coming up across VRChat, the events of your groups, the ones you follow and VRChat's featured ones. Follow one and Windows reminds you before it starts |

### Your own uploads

| | |
|---|---|
| My uploads | Your avatars and worlds in one list. Make them public or private, wear an avatar, delete what you no longer want, copy an ID |

### Look and behaviour

| | |
|---|---|
| Themes | Sunset, night, black and white, red and black, your own colours, or your own picture as the background |
| Wide windows | Settings and the friend screens spread into columns when there is room |
| Tray | Closing the window can leave the app in the tray, where it keeps recording. Starting it a second time brings the running one forward |
| Starting up | It can start with Windows, in the tray, and open its window when VRChat starts |
| After a crash | When VRChat closes without warning while you are in a world, the app offers to take you back to the same instance, or does it by itself if you choose that |
| Moving over from VRCX | Copies the friend activity, friend log, world visits, avatars you wore, notes, avatar tags and local favourites VRCX recorded for your account into this app. VRCX's file is only read, a backup is made first, and running it again adds only what is new |
| Keeping up with VRCX | With the switch on, the app looks at start and once an hour whether VRCX has written anything new and brings over only that, with a backup at most once a day. |
| Notifications in the headset (beta) | While VRChat runs, notifications also show in VR through XSOverlay or OVR Toolkit when one of them is running. Nothing leaves the PC. New, with little testing in a headset: tell me what you see if it misbehaves. The app has no SteamVR overlay of its own yet. |
| Month view of events | Events of a month as a grid of days; pressing a day shows only that day. The first day of the week can be set. |
| VRChat's cache | Shows how full VRChat's download cache is, clears old versions or all of it while the game is closed (by hand, or each time VRChat closes), and sets the size limit in VRChat's `config.json`. |
| Friend map | On Around, the Map view draws one bubble per instance with the friends in it and their names, your favourites and your own instance marked, and lines between friends you often saw together. A list beside it has every instance with its Join button. |
| Friend web | On Around, shows which of your friends are friends with each other as a town: each circle of friends is a district with its people in it, friendships between two circles are a bridge, and a list beside it names the circles and the friends who hold them together. The app asks VRChat about each friend only when you press the button, one at a time with a pause, and keeps the answer on your PC. |
| Room alarm (Beta) | New and not yet tried on many full instances. On a full instance, press "Tell me when there is room" in its window or "Full, wait" on the map. The app asks VRChat about that instance every 45 seconds and tells you when a place is free, with a Join button. It watches three instances at most and stops after an hour. A setting lets it join by itself. |
| Status by rules | In Settings, rules change your VRChat status for you: at certain hours, in Invite, Friends, group or Public instances, in worlds whose name has a word you pick, or when enough friends are in your instance. The first rule that fits decides, and the status you had before comes back afterwards. A status you set yourself, in the app or in VRChat, stays until no rule fits any more. A list in Settings shows what the rules saw and did. On Around, a card opens its instance, and a full one has the same room alarm button. |
| History search | In History, the Find tab searches your own history by a person's name or part of a world's name. For a person it shows how often and how long you were together, when you first and last saw them, and every stay you shared. For a world it shows each visit. A stay opens to show who was there. It reads only what is on your PC. |
| Where a friend hangs out | On a profile's Activity tab: the worlds where you two meet most, who is usually there too, and at which hours and weekdays. Below that, the worlds the app saw them in over the last 90 days. All from this PC's history. |
| Numbers for my worlds | Uploads, Worlds notes visits, favourites and people in your own worlds each day the app runs, and shows what they gained lately with a bar per day. |
| Fonts, dates and name colours | Pick the heading and text font, a 12 or 24 hour clock, dates in words or as year-month-day, and the colour of each trust rank or one colour per person. |
| Profile dress | Shows a person's icon frame and profile effect, and tints their profile window with their banner colour or picture. Each can be switched off. |
| Drawing without the graphics card | A switch for PCs where the window flickers or where VRChat should keep all the graphics memory. Counts from the next start. |
| Backups | History and settings go to a folder in Documents, by hand or by itself once a day (the newest 7 automatic ones are kept, you can change that). The sign-in is never part of a backup |
| VRChat outages | A warning appears when VRChat's status page reports trouble |
| Discord | Optional. Shows the world you are in on your Discord profile, with switches for the world name, the kind of instance, the head count and a Join button. Invite instances show as a private world. Needs your own Discord application ID |

## Privacy

- "Hide names and pictures" in Settings, or Ctrl+Shift+H, covers people's names, pictures, statuses and bios with plain bars, your own included, along with the names and pictures in My uploads. Nothing is blurred, so there is nothing to sharpen back. I added it so I could take screenshots without showing my friends.
- Your password goes to VRChat once, at sign-in, and the app never writes it anywhere. Two-factor works with an authenticator app, email or a recovery code.
- The sign-in cookie is kept in the Windows credential store.
- History, notes and settings are files on your PC. I run no server, so there is nowhere for them to be sent.
- Avatar search asks community avatar databases and sends them the search words only.
- Discord rich presence is off until you switch it on, and it only talks to the Discord app on your PC.

## How it treats VRChat's servers

- Every request carries a User-Agent that names this app and its version.
- All requests go through one gate that spaces them out.
- A 429 answer pauses everything, honours `Retry-After` and gives up after three tries.
- The friend list is fetched once at sign-in. After that, changes come from VRChat's websocket.
- World names, favourites and group data are cached, so the same question is not asked twice.
- World history makes no requests at all. It comes from log files already on your disk.

## Known limits

- Windows only.
- VRChat does not support outside tools on its web API, so a change on their side can break a screen until I update the app.
- With "Hide names and pictures" on, Windows notifications and hover tooltips still show names.
- This is an early version and I have only tested it on my own PC and account.

## Build it yourself

You need Rust (stable, MSVC), Node.js 24 and the WebView2 runtime that ships with Windows 11.

```
npm install
npm run tauri dev      # run with live reload
npm run tauri build    # release build and installer
```

The installer lands in `src-tauri/target/release/bundle/nsis/`.

The tests use a local stand-in server and invented accounts, so they never contact VRChat and need no credentials:

```
cd src-tauri
cargo test
```

`npm run dev` on its own opens the interface in a browser tab on made-up demo data, which is useful for layout work.

<details>
<summary>Where things are</summary>

- `src-tauri/src/vrchat/`: HTTP client, sign-in, rate limiting, websocket events, data types.
- `src-tauri/src/hub.rs`: the signed-in session, websocket loop, caches.
- `src-tauri/src/store.rs`: SQLite history.
- `src-tauri/src/gamelog.rs`: reads VRChat's log files, read only.
- `src-tauri/src/secrets.rs`: Windows credential store.
- `src-tauri/resources/licences/`: credits and licence texts the installer carries.
- `src/`: the window (React).
- `Design/`: the logo, the banner, the installer art and the pictures in this file.

</details>

## Credits

This app is built on other people's work. Thank you to them.

| | By | Used for | Licence its makers state |
|---|---|---|---|
| [Tauri](https://tauri.app), [React](https://react.dev) | Their contributors | The window and the interface | MIT, Apache-2.0 |
| Instrument Sans, Barlow Condensed, Yellowtail, Rajdhani, Orbitron, Outfit, Space Grotesk | Their designers | The fonts | OFL-1.1, Apache-2.0 |
| VRChat | VRChat Inc. | The game this app is a companion to | |

The installer carries the full licence texts in a `licences` folder, and the app lists the same credits under Settings, Credits. [THIRD-PARTY.md](THIRD-PARTY.md) has more detail.

## Help and licence

If something breaks, open an [issue](../../issues) and say what you clicked and what happened. Leave your VRChat name, password and any codes out of it.

I have not chosen a licence for this code yet. The projects it is built on keep their own licences, listed above.

<p align="center"><sub>Made by El Diablo 59.</sub></p>
