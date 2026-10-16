# riftlite

A lightweight desktop companion for **League of Legends** and **Teamfight Tactics** on Windows.
It is one native Rust process (egui/OpenGL). There is no embedded browser, no script runtime, no account, no injection and no elevation.

## Features

| Feature | Data source |
|---|---|
| League/TFT install detection | `%ProgramData%\Riot Games\RiotClientInstalls.json`, Riot Client product metadata, `HKCU\…\Uninstall\Riot Game <product>.live`, or a League folder chosen in Settings |
| Running detection (Riot Client, League client, League game, TFT client/game) | Win32 process snapshot |
| Launch League / TFT | `RiotClientServices.exe --launch-product=<id> --launch-patchline=live` |
| League client (LCU) connection | `LeagueClientUx.exe` command line (`--app-port`, `--remoting-auth-token`), with `lockfile` fallback |
| Gameflow phase, queue, map, players of the starting game | LCU `/lol-gameflow/v1/gameflow-phase`, `/lol-gameflow/v1/session` |
| Profile: Riot ID, level, ranked queues, champion mastery, challenges, open missions | LCU `/lol-summoner/v1/current-summoner`, `/lol-ranked/v1/current-ranked-stats`, `/lol-champion-mastery/v1/{puuid}/champion-mastery`, `/lol-challenges/v1/summary-player-data/local-player`, `/lol-missions/v1/missions` (on demand) |
| LoL match history, 20 at a time, with a full 10-player scoreboard per game | LCU `/lol-match-history/v1/products/lol/current-summoner/matches?begIndex=&endIndex=`, `/lol-match-history/v1/games/{gameId}` |
| Player lookup by Riot ID: rank, mastery, history, spectate | LCU `/lol-summoner/v1/alias/lookup`, `/lol-ranked/v1/ranked-stats/{puuid}`, `/lol-match-history/v1/products/lol/{puuid}/matches`, `/lol-spectator/v1/spectate/launch` |
| TFT match history (last 20): placement, traits, board | LCU `/lol-match-history/v1/products/tft/{puuid}/matches` |
| Auto-accept the ready check (off by default) | LCU `/lol-matchmaking/v1/ready-check` → `POST /lol-matchmaking/v1/ready-check/accept`, queried only while the gameflow phase is `ReadyCheck`; a check you already accepted or declined is left alone |
| Champ select: both teams with ranks, bans, ARAM bench with one-click swap, your record on the pick, your strongest available picks | LCU `/lol-champ-select/v1/session`, `/lol-champ-select/v1/pickable-champion-ids`, `/lol-champ-select/v1/session/bench/swap/{id}`, `/lol-ranked/v1/ranked-stats/{puuid}` |
| Runes and summoner spells: Riot's recommended pages for the pick, applied on lock-in (or by button) | LCU `/lol-perks/v1/recommended-pages/champion/{id}/position/{pos}/map/{map}` → one page named `riftlite: <champion>` via `/lol-perks/v1/pages` + `/lol-perks/v1/currentpage`; spells via `PATCH /lol-champ-select/v1/session/my-selection` |
| Swiftplay: recommended runes and spells for both pre-picked champions | LCU `/lol-lobby/v1/lobby/members/localMember/player-slots` |
| Item sets on lock-in: OP.GG's most played build for your mode and lane, in Starting / Core (in order) / Boots / Full build / Situational blocks, plus your best recent game as an extra block; your own build alone when OP.GG is off, unreachable or has under 100 games; any past or just-finished game's build on demand; remove them all again | OP.GG's public MCP server `https://mcp-api.op.gg/mcp`, tool `lol_get_champion_analysis` (sends only champion id, mode and lane); your own LCU match history → `<League>/Config/Champions/<Champion>/Recommended/riftlite_meta_<id>_<mode>.json` / `riftlite_<gameId>.json` |
| OP.GG rune page and skill order for the pick, next to Riot's recommended pages | Same OP.GG answer |
| Champion browser: Riot's recommended runes per position and for ARAM, your stats and mastery per champion | LCU `/lol-perks/v1/recommended-champion-positions`, recommended pages, your history |
| Live game: scoreboard with ranks, item value per player/team/lane, objective timers, CS/min benchmark, recorded skill order | Riot Live Client Data API `https://127.0.0.1:2999/liveclientdata/allgamedata` (players, items with prices, event feed, ability levels) |
| In-game overlay: clock, KDA, CS/min vs your average, team and lane item-value difference, dragon/Elder/Baron/inhibitor timers, "level up Q" hint, enemy respawns; enemy ranks on the loading screen and for the first 90 s | Same Live Client Data API plus gameflow session ranks; separate always-on-top, click-through, never-activated window; Ctrl+Shift+O or the tray menu hides it |
| Post-game: result, LP change, scoreboard, insights versus your own averages in that queue | LCU `/lol-end-of-game/v1/eog-stats-block`, `/lol-ranked/v1/current-lp-change-notification`, your history |
| Skill orders | Recorded by riftlite from the Live Client API while you play; `%LOCALAPPDATA%\riftlite\skills.json` |
| Champion, item, summoner spell and rune names | Riot Data Dragon (`ddragon.leagueoflegends.com`), cached per patch; ids Data Dragon lacks (mode-only items, stat shards) from the client's `/lol-game-data/assets/v1/*.json` |
| Tray icon (open / hide overlay / quit), start with Windows (minimized), settings | `Shell_NotifyIconW`, `RegisterHotKey`, `HKCU\…\Run`, `%LOCALAPPDATA%\riftlite\settings.json` |

## Limitations

- TFT data (ranked and match history) comes only through the League client's LCU. When TFT runs in the standalone TFT client without the League client, there is no LCU to read from.
- No tier lists, pro builds, augment tier lists, jungle paths, matchup win rates or news. Aggregated builds come from OP.GG (with source credit, as OP.GG asks) and cover ranked solo/duo, flex, ARAM (also used for ARAM Mayhem, which OP.GG does not track separately), URF and Nexus Blitz, aggregated over all ranks; Arena and other modes fall back to your own builds. Benchmarks, suggestions and insights use your own games only.
- No ultimate/summoner-spell cooldown tracking, ward/trinket timers, ARAM relic timers or jungle camp timers. The Live Client API does not expose them, and riftlite does not read game memory. Objective timers cover dragon, Elder, Baron and inhibitors, from the game's event feed. Inhibitor respawns are timed on Summoner's Rift only; elsewhere they show time since destroyed.
- No clip recording.
- Skill-order hints need at least one earlier game on that champion played with riftlite running.
- Champion and item names follow Data Dragon's `en_US` data. Live-game item names and the client's recommended-page labels come from Riot in the client's locale.

The overlay is drawn over the game only in **Borderless** or **Windowed** display mode. Exclusive fullscreen would need DLL injection, which riftlite does not do.

## Security model

- **Riot loopback TLS is pinned.** The LCU and Live Client Data API certificates must be issued and signed by Riot's "LoL Game Engineering Certificate Authority", which is embedded from `assets/riotgames.pem`. Certificate verification is never disabled.
- **No code-loading surface.** There is nothing like Electron's RunAsNode, `NODE_OPTIONS` or `--inspect`, no `file://` protocol handler, and no remotely served UI.
- **Never touches game processes.** riftlite does not inject into, write to, or read the memory of any game process. The League client's command line is read with `PROCESS_QUERY_LIMITED_INFORMATION` only.
- **Writes only what you enable.** Client writes are limited to: accepting a ready check, riftlite's own rune page (never your pages), summoner spells, a bench swap, Swiftplay slot runes, spectate requests, and item-set files prefixed `riftlite_`. Each is a setting or a button.
- **No admin rights.** It needs no administrator rights and installs no certificates or services.
- **No telemetry and no ads.** Network traffic goes only to 127.0.0.1 (Riot's local APIs), Data Dragon, and, when "Use OP.GG builds" is on, `mcp-api.op.gg` with the picked champion's id, mode and lane.

## Build

```
cargo build --release
```

The output is `target\release\riftlite.exe`, a single file with no installer. Start it with `--minimized` to start minimized.
