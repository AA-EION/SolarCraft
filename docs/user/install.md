# Installing a test build

CI builds installers on every push to the development branch. Open the repository's **Actions** tab, select the latest green `ci` run, and download from **Artifacts**:

| Artifact | Contents | For |
|---|---|---|
| `SolarCraft-setup-win-x64` | `SolarCraft-<version>-win-x64-setup.exe` | Windows 10 (2004+) / 11 on Intel or AMD |
| `SolarCraft-setup-win-arm64` | `SolarCraft-<version>-win-arm64-setup.exe` | Windows 11 on ARM (Snapdragon) |
| `SolarCraft-macos-universal-dmg` | `SolarCraft-<version>-macos-universal.dmg` | macOS on Apple silicon and Intel |

GitHub wraps each artifact in a `.zip`; unzip it first. Artifacts expire after 30 days.

Before each artifact is uploaded, CI installs or mounts it and runs the app's self-test, which runs the whole project workflow through the GUI's API calls: x64 and native ARM64 on Windows, and arm64 plus x86_64 under Rosetta on macOS. The x86_64 macOS build is **tested under emulation (Rosetta) only** (D-13).

## Windows

The installer is **not code-signed yet**, so SmartScreen shows "Windows protected your PC". Choose **More info → Run anyway**.

By default it installs for the current user only, with no admin rights needed. It creates a Start menu entry and, optionally, a desktop icon. You can uninstall it from **Settings → Apps**. The installer runs in English, Spanish, German or Brazilian Portuguese.

## macOS

The app has an ad-hoc signature only. It has **no Developer ID signature and is not notarized**, so Gatekeeper blocks the first launch.

1. Open the DMG and drag **SolarCraft** to **Applications**.
2. Open SolarCraft. When macOS says it cannot verify the developer, click **Done**.
3. Go to **System Settings → Privacy & Security**, scroll to "SolarCraft was blocked", and click **Open Anyway**. Then confirm.

Alternatively, run this in Terminal once:

```bash
xattr -dr com.apple.quarantine /Applications/SolarCraft.app
```

## What to try

1. Create a project.
2. Use demo weather, or fetch PVGIS (this asks for network consent).
3. Draw a roof and add an obstacle with a height.
4. Simulate.
5. Read the loss breakdown.
6. Export a PDF or DOCX report.

Switch between Easy and Detailed mode at any point.

Please report anything that looks wrong, feels awkward or crashes. Include a screenshot and the platform.
