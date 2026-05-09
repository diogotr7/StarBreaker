---

title: Zero to Hero tutorial
description: Windows power-user setup for StarBreaker, Blender, and Star Citizen asset exploration.
-------------------------------------------------------------------------------------------------------------------

# Zero to Hero tutorial

> Community tutorial by [DirectorGunner](https://github.com/DirectorGunner).

This guide walks through a Windows 11 setup for working with StarBreaker, Blender, and supporting Star Citizen asset tools. It favors a power-user workflow where tools, caches, exports, and work files live under `D:\dev` instead of being scattered across the system drive.

## Folder layout used in this guide

```text
D:\dev
├─ cmake
├─ dotnet
├─ installers
├─ node
├─ python
│  ├─ Python312
│  ├─ pip-cache
│  └─ venvs\scdev
├─ rust
│  ├─ .cargo
│  └─ .rustup
├─ scdata
│  ├─ exports
│  ├─ logs
│  ├─ p4k
│  │  └─ 4.4.1-LIVE-9457020\Data.p4k
│  └─ work
└─ starcitizen
   ├─ StarBreaker
   ├─ Blender-Tools
   ├─ unp4k
   ├─ Cryengine-Converter
   ├─ SCTextureConverter
   └─ _workspace
```

Replace paths as needed if you use a different drive or folder layout.

## 1. Install base tools

Open **PowerShell as Administrator**.

### Install Git

```powershell
winget install --id Git.Git -e --source winget
```

After install, confirm Git is on `PATH`:

```powershell
git --version
```

If `git` is not found, open **Edit the system environment variables** from the Start menu, then go to:

```text
Environment Variables → System variables → Path → Edit
```

Add this entry if missing:

```text
C:\Program Files\Git\cmd
```

Close and reopen PowerShell.

### Install Visual Studio Build Tools

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --source winget --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

Make sure the following components are installed:

* Desktop development with C++
* MSVC v143 VS 2022 C++ x64/x86 build tools
* Windows 11 SDK
* C++ CMake tools for Windows

If those were not installed, run the Visual Studio installer manually:

```powershell
& "C:\Program Files (x86)\Microsoft Visual Studio\Installer\setup.exe"
```

Choose **Modify** for Build Tools and add the missing components.

## 2. Install Rust on `D:\dev`

Use **Developer PowerShell for VS 2022** for Rust verification and builds.

Create Rust folders:

```powershell
mkdir D:\dev\rust -Force
mkdir D:\dev\rust\.cargo -Force
mkdir D:\dev\rust\.rustup -Force
```

Set Rust environment variables:

```powershell
setx CARGO_HOME "D:\dev\rust\.cargo"
setx RUSTUP_HOME "D:\dev\rust\.rustup"
```

Add Cargo to your user or system `Path` manually using Windows Environment Variables:

```text
D:\dev\rust\.cargo\bin
```

Restart **Developer PowerShell for VS 2022** and verify:

```powershell
echo $env:CARGO_HOME
echo $env:RUSTUP_HOME
```

Install Rust:

```powershell
winget install --id Rustlang.Rustup -e --source winget
```

Verify:

```powershell
where.exe rustup
where.exe cargo
where.exe rustc
rustup --version
cargo --version
rustc --version
```

### If Rust install fails

If the installer fails because of firewall, antivirus, or a blocked installer, remove partial installs:

```powershell
winget uninstall --id Rustlang.Rustup -e
winget list Rust
winget list Rustlang.Rustup
Remove-Item "$env:USERPROFILE\.cargo" -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.rustup" -Recurse -Force -ErrorAction SilentlyContinue
```

Fix the blocker, then retry the Rust install.

## 3. Install Python on `D:\dev`

Run:

```powershell
winget install --id Python.Python.3.12 -e --source winget --interactive
```

In the installer choose **Customize installation** and enable:

* pip
* py launcher
* Add Python to environment variables

Use this install path:

```text
D:\dev\python\Python312
```

Create pip cache and venv folders:

```powershell
mkdir D:\dev\python\pip-cache -Force
mkdir D:\dev\python\venvs -Force
setx PIP_CACHE_DIR "D:\dev\python\pip-cache"
```

Restart Developer PowerShell and verify:

```powershell
where.exe python
where.exe py
where.exe pip
python --version
py --version
pip --version
```

If Windows aliases intercept Python, disable them here:

```text
Settings → Apps → Advanced app settings → App execution aliases
```

Turn off:

```text
App Installer - python.exe
App Installer - python3.exe
```

Create a shared development virtual environment:

```powershell
D:\dev\python\Python312\python.exe -m venv D:\dev\python\venvs\scdev
```

Allow local activation scripts for your Windows user:

```powershell
Set-ExecutionPolicy -Scope CurrentUser -ExecutionPolicy RemoteSigned
```

Activate the venv:

```powershell
D:\dev\python\venvs\scdev\Scripts\Activate.ps1
```

Verify the prompt shows `(scdev)` and run:

```powershell
python --version
pip --version
where.exe python
where.exe pip
```

Upgrade pip tooling:

```powershell
python -m pip install --upgrade pip setuptools wheel
```

Check the environment:

```powershell
pip --version
python -m pip check
python -c "import sys; print(sys.executable)"
```

Deactivate when done:

```powershell
deactivate
```

## 4. Create work folders

```powershell
mkdir D:\dev\installers -Force
mkdir D:\dev\dotnet -Force
mkdir D:\dev\cmake -Force
mkdir D:\dev\scdata -Force
mkdir D:\dev\scdata\p4k -Force
mkdir D:\dev\scdata\exports -Force
mkdir D:\dev\scdata\work -Force
mkdir D:\dev\scdata\logs -Force
```

## 5. Install .NET SDKs on `D:\dev`

Download the installer script:

```powershell
Invoke-WebRequest -Uri "https://dot.net/v1/dotnet-install.ps1" -OutFile "D:\dev\installers\dotnet-install.ps1"
```

Install .NET 8:

```powershell
powershell -ExecutionPolicy Bypass -File D:\dev\installers\dotnet-install.ps1 -Channel 8.0 -InstallDir D:\dev\dotnet
```

Install .NET 9:

```powershell
powershell -ExecutionPolicy Bypass -File D:\dev\installers\dotnet-install.ps1 -Channel 9.0 -InstallDir D:\dev\dotnet
```

Set `DOTNET_ROOT`:

```powershell
setx DOTNET_ROOT "D:\dev\dotnet"
```

Add this path manually in Windows Environment Variables:

```text
D:\dev\dotnet
```

Restart Developer PowerShell and check:

```powershell
where.exe dotnet
dotnet --info
Test-Path D:\dev\dotnet\dotnet.exe
Get-ChildItem D:\dev\dotnet
D:\dev\dotnet\dotnet.exe --info
```

If `C:\Program Files\dotnet\dotnet.exe` is taking priority, move `D:\dev\dotnet` above `C:\Program Files\dotnet` in the system `Path`.

## 6. Install CMake on `D:\dev`

```powershell
Invoke-WebRequest -Uri "https://github.com/Kitware/CMake/releases/download/v4.3.1/cmake-4.3.1-windows-x86_64.zip" -OutFile "D:\dev\installers\cmake-4.3.1-windows-x86_64.zip"
Expand-Archive -Path "D:\dev\installers\cmake-4.3.1-windows-x86_64.zip" -DestinationPath "D:\dev\installers\cmake-extract" -Force
Copy-Item "D:\dev\installers\cmake-extract\cmake-4.3.1-windows-x86_64\*" "D:\dev\cmake" -Recurse -Force
```

Verify:

```powershell
Test-Path D:\dev\cmake\bin\cmake.exe
```

Add this to system `Path`:

```text
D:\dev\cmake\bin
```

Check:

```powershell
where.exe cmake
cmake --version
```

Developer PowerShell may place Visual Studio’s bundled CMake first. If you need the `D:\dev` CMake for a session, run:

```powershell
$env:Path = "D:\dev\cmake\bin;$env:Path"
where.exe cmake
```

## 7. Clone repositories

Regular PowerShell is fine for this step.

```powershell
mkdir D:\dev\starcitizen
cd D:\dev\starcitizen

git clone https://github.com/diogotr7/StarBreaker.git
git clone https://github.com/scorg-tools/Blender-Tools.git
git clone https://github.com/dolkensp/unp4k.git
git clone https://github.com/markemp/Cryengine-Converter.git
git clone https://github.com/Madfish71/SCTextureConverter
```

## 8. Install Visual Studio Code and extensions

Install VS Code normally. Then add its `bin` folder to `Path`. Example:

```text
C:\Microsoft VS Code\bin
```

Restart Developer PowerShell and verify:

```powershell
where.exe code
code --version
code --list-extensions
```

Install recommended extensions:

```powershell
code --install-extension ms-python.python --force
code --install-extension ms-python.vscode-pylance --force
code --install-extension rust-lang.rust-analyzer --force
code --install-extension tamasfe.even-better-toml --force
code --install-extension ms-dotnettools.csdevkit --force
code --install-extension GitHub.vscode-pull-request-github --force
code --install-extension jacqueslucke.blender-development --force
```

Optional helpful extensions:

```powershell
code --install-extension eamodio.gitlens --force
code --install-extension yzhang.markdown-all-in-one --force
code --install-extension DavidAnson.vscode-markdownlint --force
code --install-extension bierner.markdown-mermaid --force
code --install-extension redhat.vscode-yaml --force
code --install-extension redhat.vscode-xml --force
code --install-extension ms-azuretools.vscode-docker --force
code --install-extension ms-vscode.powershell --force
code --install-extension EditorConfig.EditorConfig --force
code --install-extension streetsidesoftware.code-spell-checker --force
code --install-extension openai.chatgpt --force
```

Verify:

```powershell
code --list-extensions
```

## 9. Install Node.js on `D:\dev`

```powershell
mkdir D:\dev\node -Force
mkdir D:\dev\node\npm-cache -Force
mkdir D:\dev\node\npm-global -Force
```

Download and extract Node.js:

```powershell
Invoke-WebRequest -Uri "https://nodejs.org/dist/v22.21.1/node-v22.21.1-win-x64.zip" -OutFile "D:\dev\installers\node-v22.21.1-win-x64.zip"
Expand-Archive -Path "D:\dev\installers\node-v22.21.1-win-x64.zip" -DestinationPath "D:\dev\installers\node-extract" -Force
Copy-Item "D:\dev\installers\node-extract\node-v22.21.1-win-x64\*" "D:\dev\node" -Recurse -Force
```

Verify:

```powershell
D:\dev\node\node.exe --version
D:\dev\node\npm.cmd --version
```

Add these to your user `Path`:

```text
D:\dev\node
D:\dev\node\npm-global
```

Restart PowerShell and verify:

```powershell
where.exe node
where.exe npm
where.exe npx
node --version
npm --version
npx --version
```

Configure npm:

```powershell
npm config set cache "D:\dev\node\npm-cache" --location=user
npm config set prefix "D:\dev\node\npm-global" --location=user
```

Verify:

```powershell
npm config get cache
npm config get prefix
npm --version
npm config list
```

## 10. Install Codex CLI and VS Code AI tools

Install Codex CLI globally:

```powershell
npm install -g @openai/codex
```

Verify:

```powershell
where.exe codex
codex --version
```

Open VS Code, sign in to GitHub, then install/sign into your preferred AI tools, such as Codex or Claude Code.

## 11. Create the VS Code workspace

```powershell
mkdir D:\dev\starcitizen\_workspace -Force
notepad D:\dev\starcitizen\_workspace\starcitizen-tools.code-workspace
```

Paste this workspace JSON, adjusting paths if needed:

```json
{
  "folders": [
    { "name": "StarBreaker", "path": "D:/dev/starcitizen/StarBreaker" },
    { "name": "Blender-Tools", "path": "D:/dev/starcitizen/Blender-Tools" },
    { "name": "unp4k", "path": "D:/dev/starcitizen/unp4k" },
    { "name": "Cryengine-Converter", "path": "D:/dev/starcitizen/Cryengine-Converter" },
    { "name": "SCTextureConverter", "path": "D:/dev/starcitizen/SCTextureConverter" },
    { "name": "scdatatools", "path": "D:/dev/starcitizen/scdatatools" },
    { "name": "qtvscodestyle", "path": "D:/dev/starcitizen/qtvscodestyle" },
    { "name": "scdata", "path": "D:/dev/scdata" }
  ],
  "settings": {
    "terminal.integrated.defaultProfile.windows": "Developer PowerShell for VS 2022",
    "terminal.integrated.profiles.windows": {
      "Developer PowerShell for VS 2022": {
        "source": "PowerShell",
        "args": [
          "-NoExit",
          "-ExecutionPolicy",
          "Bypass",
          "-Command",
          "& 'C:/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/Common7/Tools/Launch-VsDevShell.ps1' -Arch amd64"
        ]
      },
      "PowerShell": {
        "source": "PowerShell"
      }
    },
    "terminal.integrated.env.windows": {
      "SC_DEV_ROOT": "D:/dev",
      "SC_DATA_ROOT": "D:/dev/scdata",
      "SC_P4K_ROOT": "D:/dev/scdata/p4k",
      "SC_EXPORT_ROOT": "D:/dev/scdata/exports",
      "SC_WORK_ROOT": "D:/dev/scdata/work"
    },
    "python.defaultInterpreterPath": "D:/dev/python/venvs/scdev/Scripts/python.exe",
    "files.exclude": {
      "**/target": true,
      "**/bin": true,
      "**/obj": true,
      "**/__pycache__": true
    }
  }
}
```

Open the workspace:

```powershell
code D:\dev\starcitizen\_workspace\starcitizen-tools.code-workspace
```

## 12. Create safe development branches

Create development branches for the cloned GitHub repositories:

```powershell
cd D:\dev\starcitizen\StarBreaker
git status
git checkout -b dev-mcp-blender-workflow

cd D:\dev\starcitizen\Blender-Tools
git status
git checkout -b dev-direct-p4k-workflow

cd D:\dev\starcitizen\unp4k
git status
git checkout -b dev-direct-p4k-workflow

cd D:\dev\starcitizen\Cryengine-Converter
git status
git checkout -b dev-sc-asset-pipeline

cd D:\dev\starcitizen\SCTextureConverter
git status
git checkout -b dev-sc-texture-pipeline
```

If you have private/offline StarFab-related source folders, copy them to:

```text
D:\dev\starcitizen\scdatatools
D:\dev\starcitizen\qtvscodestyle
```

Initialize them as local-only Git repos with no remote.

For `scdatatools`:

```powershell
cd D:\dev\starcitizen\scdatatools
git init
git config user.name "Local Developer"
git config user.email "local@example.invalid"
git checkout -b local-private-baseline
"__pycache__/", "*.pyc", "*.pyo", "*.pyd", ".venv/", "venv/", ".env", "*.log", ".DS_Store", "Thumbs.db", ".vscode/", ".idea/" | Set-Content -Encoding utf8 .gitignore
git add .
git commit -m "Local private baseline import"
git checkout -b dev-private-review
git remote -v
```

For `qtvscodestyle`:

```powershell
cd D:\dev\starcitizen\qtvscodestyle
git init
git config user.name "Local Developer"
git config user.email "local@example.invalid"
git checkout -b local-private-baseline
"__pycache__/", "*.pyc", "*.pyo", "*.pyd", ".venv/", "venv/", ".env", "*.log", ".DS_Store", "Thumbs.db", ".vscode/", ".idea/" | Set-Content -Encoding utf8 .gitignore
git add .
git commit -m "Local private baseline import"
git checkout -b dev-private-review
git remote -v
```

`git remote -v` should print nothing for these private/local-only folders.

Verify branches:

```powershell
cd D:\dev\starcitizen\StarBreaker; git status; git branch --show-current
cd D:\dev\starcitizen\Blender-Tools; git status; git branch --show-current
cd D:\dev\starcitizen\unp4k; git status; git branch --show-current
cd D:\dev\starcitizen\Cryengine-Converter; git status; git branch --show-current
cd D:\dev\starcitizen\SCTextureConverter; git status; git branch --show-current
cd D:\dev\starcitizen\scdatatools; git status; git branch --show-current
cd D:\dev\starcitizen\qtvscodestyle; git status; git branch --show-current
```

Expected working branches:

```text
StarBreaker            dev-mcp-blender-workflow
Blender-Tools          dev-direct-p4k-workflow
unp4k                  dev-direct-p4k-workflow
Cryengine-Converter    dev-sc-asset-pipeline
SCTextureConverter     dev-sc-texture-pipeline
scdatatools            dev-private-review
qtvscodestyle          dev-private-review
```

## 13. Verify the workspace terminal

Reopen the workspace:

```powershell
code D:\dev\starcitizen\_workspace\starcitizen-tools.code-workspace
```

In the VS Code terminal, verify:

```powershell
echo $env:SC_P4K_ROOT
echo $env:SC_EXPORT_ROOT
echo $env:SC_WORK_ROOT
cl
where.exe cl
where.exe link
rustc --version
cargo --version
```

## 14. Build StarBreaker and MCP

In the VS Code terminal:

```powershell
cd D:\dev\starcitizen\StarBreaker
git branch --show-current
git status --short
cargo build --release -p starbreaker
```

Test:

```powershell
.\target\release\starbreaker.exe --help
```

Build the MCP binary:

```powershell
cargo build --release -p starbreaker-mcp
```

List outputs:

```powershell
Get-ChildItem .\target\release -Filter "*.exe" | Select-Object Name, FullName
```

Test MCP:

```powershell
.\target\release\starbreaker-mcp.exe --help
```

## 15. Select a Star Citizen build for the session

Copy your `Data.p4k` into a versioned folder, for example:

```text
D:\dev\scdata\p4k\4.4.1-LIVE-9457020\Data.p4k
```

Set it for the current terminal session:

```powershell
$env:SC_BUILD = "4.4.1-LIVE-9457020"
$env:SC_DATA_P4K = Join-Path $env:SC_P4K_ROOT "$env:SC_BUILD\Data.p4k"
echo $env:SC_DATA_P4K
Test-Path $env:SC_DATA_P4K
```

`Test-Path` should return `True`.

## 16. Explore P4K paths

From `D:\dev\starcitizen\StarBreaker`, list top-level Spaceships folders:

```powershell
.\target\release\starbreaker.exe p4k list --filter 'Data/Objects/Spaceships/**' | ForEach-Object { ($_ -split '\s+')[0] } | ForEach-Object { if ($_ -match '^Data[\\/]+Objects[\\/]+Spaceships[\\/]+([^\\/]+)') { $matches[1] } } | Sort-Object -Unique
```

List manufacturer folders under `Ships`:

```powershell
.\target\release\starbreaker.exe p4k list --filter 'Data/Objects/Spaceships/Ships/**' | ForEach-Object { ($_ -split '\s+')[0] } | ForEach-Object { if ($_ -match '^Data[\\/]+Objects[\\/]+Spaceships[\\/]+Ships[\\/]+([^\\/]+)') { $matches[1] } } | Sort-Object -Unique
```

List RSI ship folders:

```powershell
.\target\release\starbreaker.exe p4k list --filter 'Data/Objects/Spaceships/Ships/RSI/**' | ForEach-Object { ($_ -split '\s+')[0] } | ForEach-Object { if ($_ -match '^Data[\\/]+Objects[\\/]+Spaceships[\\/]+Ships[\\/]+RSI[\\/]+([^\\/]+)') { $matches[1] } } | Sort-Object -Unique
```

Search for Aurora paths:

```powershell
.\target\release\starbreaker.exe p4k list --filter 'Data/Objects/Spaceships/Ships/RSI/**' | Select-String -Pattern 'Aurora' -CaseSensitive:$false
```

Because the list may be long, save it:

```powershell
.\target\release\starbreaker.exe p4k list --filter 'Data/Objects/Spaceships/Ships/RSI/**' | Select-String -Pattern 'Aurora' -CaseSensitive:$false | Out-File -Encoding utf8 "D:\dev\scdata\work\aurora_rsi_ship_paths_4.4.1-LIVE-9457020.txt"
notepad "D:\dev\scdata\work\aurora_rsi_ship_paths_4.4.1-LIVE-9457020.txt"
```

## 17. Resolve and export an Aurora MR example

Resolve the generic Aurora entity:

```powershell
.\target\release\starbreaker.exe entity loadout RSI_Aurora --p4k "$env:SC_DATA_P4K" *> "D:\dev\scdata\work\loadout_RSI_Aurora_4.4.1-LIVE-9457020.txt"
```

Resolve the MR variant:

```powershell
.\target\release\starbreaker.exe entity loadout RSI_Aurora_MR --p4k "$env:SC_DATA_P4K" *> "D:\dev\scdata\work\loadout_RSI_Aurora_MR_4.4.1-LIVE-9457020.txt"
```

Export the Aurora MR as a decomposed package:

```powershell
.\target\release\starbreaker.exe entity export RSI_Aurora_MR D:\dev\scdata\exports\aurora_mr_decomposed --p4k "$env:SC_DATA_P4K" --kind decomposed --materials textures --lod 1 --mip 2 *> "D:\dev\scdata\work\export_RSI_Aurora_MR_decomposed_4.4.1-LIVE-9457020.txt"
```

Create a file-tree log:

```powershell
Get-ChildItem "D:\dev\scdata\exports\aurora_mr_decomposed" -Recurse | Select-Object FullName, Length | Out-File -Encoding utf8 "D:\dev\scdata\work\export_RSI_Aurora_MR_decomposed_filetree_4.4.1-LIVE-9457020.txt"
```

Logs are saved in:

```text
D:\dev\scdata\work
```

The decomposed package is saved in:

```text
D:\dev\scdata\exports\aurora_mr_decomposed
```

## 18. Install the StarBreaker Blender add-on

Close Blender before running this command.

```powershell
$src='D:\dev\starcitizen\StarBreaker\blender_addon\starbreaker_addon'; $dst="$env:APPDATA\Blender Foundation\Blender\5.1\scripts\addons\starbreaker_addon"; New-Item -ItemType Directory -Force -Path (Split-Path $dst) | Out-Null; if (Test-Path $dst) { Write-Host "Destination already exists: $dst"; Write-Host "Do not overwrite yet. Check what is there first." } else { New-Item -ItemType Junction -Path $dst -Target $src }
```

Open Blender, then go to:

```text
Edit → Preferences → Add-ons
```

Search for **StarBreaker** and enable it.

## 19. Import the decomposed package into Blender

In Blender:

```text
3D Viewport → press N → StarBreaker tab → Import StarBreaker Package
```

Select:

```text
D:\dev\scdata\exports\aurora_mr_decomposed\Packages\RSI Aurora MR_LOD1_TEX2\scene.json
```

After import:

1. Disable viewport overlays to reduce black helper-line clutter.
2. In the Outliner, select the Aurora package root.
3. In the StarBreaker sidebar animation menu, find **Landing Gear Retract**.
4. Click **Last**.

The landing gear should retract if the matching animation data was exported and applied successfully.
