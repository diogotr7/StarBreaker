---
title: Install
description: Install StarBreaker on Windows or Linux.
---

Grab the latest release from [GitHub](https://github.com/diogotr7/StarBreaker/releases/latest).
Each release has the CLI for Windows (`starbreaker-cli-<version>-windows-x86_64.zip`)
and Linux (`starbreaker-cli-<version>-linux-x86_64.tar.gz`), plus installers for the
desktop app. Unpack the CLI anywhere.

## Requirements

- Windows 10 / 11, or Linux x86_64
- A legitimate Star Citizen install (you need your own `Data.p4k`)

StarBreaker finds `Data.p4k` in the standard RSI install locations (on Linux, the
LUG Wine/Proton prefix under `~/Games/star-citizen`). Set `SC_DATA_P4K` only to
use a different archive, such as a PTU build or a copy elsewhere.

## Verifying the install

Open a terminal in the unpacked folder and run:

```bash
starbreaker --help
```

You should see the list of available commands. If not, check that you are
running it from the directory that contains `starbreaker` (`starbreaker.exe`
on Windows).
