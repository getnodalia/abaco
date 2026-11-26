# Windows packaging

`package.ps1` builds the MSI and the portable zip for one architecture (`x64`, `x86`, `arm64`);
`.github/workflows/release-windows.yml` runs it for x64 on a Windows runner. Nothing here is
signed unless the secrets `sign.ps1` reads are set (see its header); unsigned builds make
SmartScreen warn the first time they run.

| File | What it is |
|---|---|
| `abaco.wxs` | The MSI package: files, Start menu shortcut, the `.abaco` file type, "Open with" and `abaco` in the Run dialog |
| `install-policy.wxi` | Install for one account or for everyone, remember the folder of an existing copy, refuse to move a copy between scopes |
| `installer-ui.wxs` | The shared Nodalia setup dialogs. A snapshot of the same file Pliego ships, with the app name changed; it was not regenerated with `craft-installer` for Ábaco |
| `install-policy.json` | What `check-install-policy.ps1` expects of the compiled MSI |
| `check-install-policy.ps1`, `test-install-policy.ps1` | Metadata checks of the compiled MSI (never installs it); copies of Pliego's |
| `sign.ps1` | Optional code signing |

The installed names are `abaco.exe` (the window) and `abaco-cli.exe`, which is what the Nodalia
Launcher looks for. Cargo builds them as `abaco-app.exe` and `abaco.exe`, so `package.ps1`
renames them. The window's executable embeds no icon resource yet: the Start menu shortcut, the
Apps list and the `.abaco` file type use `abaco.ico`, which the MSI installs beside it.

Not checked: this has not been run on Windows. The MSI has not been built, installed or removed
by hand; the first release build is its first run.
