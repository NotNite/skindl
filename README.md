# skindl

Bomb Rush Cyberfunk GameBanana downloader. Hijacks the Concursus URI scheme.

## Why?

I don't like Concursus. Also, I need a small programming project to get used to a split keyboard.

## How to use

- Get the executable from [the releases](https://github.com/NotNite/skindl/releases)
- Launch the executable and select your BepInEx folder
- Click "Concursus Mod Manager" when downloading a mod
- Profit (hopefully)

## TODO

- [x] Registering the URI handler
- [x] Downloading mods
  - [x] zip
  - [x] cbb
  - [x] 7z
  - [x] rar
- [x] Better error handling & idiot proofing
  - [x] Show errors instead of insta closing
  - [x] Move game path file to somewhere harder to accidentally delete
- [x] GUI with eframe
- [x] GitHub Actions/Releases
- [ ] Support for other mod types
  - [x] CrewBoom
  - [ ] DripRemix
  - [ ] BombRushRadio
