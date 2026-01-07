# Release & Semantic Versioning Guide

This project utilizes an automated CI/CD pipeline via GitHub Actions to build, bundle, and release the **Tape Delay** plugin. Following the steps below ensures that cross-platform binaries are generated correctly and versioned according to [Semantic Versioning (SemVer)](https://semver.org/) standards.

---

## 1. Triggering a New Release

The build workflow is triggered specifically by **Git Tags**. When you are ready to publish a new version, create a tag starting with a lowercase `v`.

### Step A: Create a Tag
Use the `-a` flag for an annotated tag. This allows you to include a brief summary of the changes.
```bash
git tag -a v1.0.4 -m "Fixed crackle polarity and gain staging"
```

### Step B: Push to GitHub
Pushing the tag specifically (not just the branch) initiates the GitHub Action.
```bash
git push origin v1.0.4
```

---
## 2. Automated Versioning Logic
The workflow includes a `get_version` step that handles the naming conventions automatically:
- Stripping the Prefix: The script strips the `v` from the tag name (e.g., `v1.2.3` becomes `1.2.3`).
- Release Naming: The GitHub Release will be titled "**Release v1.2.3**".
- Filename Generation: Artifacts are renamed for clarity, resulting in filenames like:
  - `tape_delay-v1.2.3-win64.zip` 
  - `tape_delay-v1.2.3-macos.zip`

---
## 3. Bundle & Zip Handling
Because VST3 plugins are handled differently across operating systems, the workflow applies specific packaging logic to ensure the plugins remain functional after download.

### Windows (x64)

The Windows build generates a `.vst3` file (essentially a renamed DLL). These are zipped into a standard archive for easy extraction into the `Common Files/VST3` directory.
### macOS (Universal)

On macOS, a VST3 is a Bundle (a specific directory structure). Direct uploads to GitHub often mangle folder permissions or strip metadata.

### IMPORTANT: Preserving Permissions
The workflow uses `zip -ry` for macOS:
- The `-r` flag ensures the entire folder structure is captured.
- The `-y` flag preserves symbolic links and the executable bit. Without this, the plugin will fail to load in DAWs because the binary inside the bundle will not have permission to execute.

---

## 4. Post-Release: Gatekeeper Bypass (macOS)
Since these builds are not code-signed or notarized, macOS users must clear the "quarantine" flag after moving the plugin to their `/Library/Audio/Plug-Ins/VST3/` folder:
```bash
sudo xattr -rd com.apple.quarantine /Library/Audio/Plug-Ins/VST3/tape_delay.vst3
```
