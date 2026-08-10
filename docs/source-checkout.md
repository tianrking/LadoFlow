# Getting the source without downloading every platform

LadoFlow is a monorepo. The shared protocol, desktop hosts, Android display,
native Windows components, documentation, and CI live in one Git repository so
one commit can describe an interoperable product version.

A normal clone checks out every tracked source file. It does **not** include
build caches, SDKs, `node_modules`, Rust `target` output, APKs, installers, or
other generated artifacts; those paths are ignored. End users should download
signed packages from [GitHub Releases](https://github.com/tianrking/LadoFlow/releases)
when releases become available instead of cloning the source.

## Complete checkout

Use this when changing shared protocol code or more than one platform:

```bash
git clone https://github.com/tianrking/LadoFlow.git
cd LadoFlow
```

To omit old Git history while retaining the complete current source tree:

```bash
git clone --depth 1 https://github.com/tianrking/LadoFlow.git
cd LadoFlow
```

## Android-only checkout

Git partial clone avoids downloading unneeded file contents, while sparse
checkout materializes only the Android project and protocol documentation:

```bash
git clone --depth 1 --filter=blob:none --sparse https://github.com/tianrking/LadoFlow.git
cd LadoFlow
git sparse-checkout set apps/android docs
cd apps/android
```

The root-level Git and build metadata remains visible by design. The Android
application itself builds from `apps/android`; the Rust and desktop trees are
not required for an Android-only Gradle build.

## Desktop checkout

For the common Windows, macOS, or Linux Tauri host:

```bash
git clone --depth 1 --filter=blob:none --sparse https://github.com/tianrking/LadoFlow.git
cd LadoFlow
git sparse-checkout set apps/desktop crates docs
```

Windows host and virtual-display work also needs the native Windows tree:

```bash
git sparse-checkout add platform/windows
```

The root `Cargo.toml`, `Cargo.lock`, `package.json`, and `pnpm-lock.yaml` are
included automatically, so commands still run from the repository root.

## Change or expand a sparse checkout

Replace the selected platform set at any time:

```bash
git sparse-checkout set apps/android docs
git sparse-checkout set apps/desktop crates platform/windows docs
```

Restore the complete working tree without cloning again:

```bash
git sparse-checkout disable
```

`git pull` continues to update the selected source normally. Removing `--depth
1` retains complete history; removing both `--depth 1` and `--filter=blob:none`
performs a conventional full clone.

## 中文说明

LadoFlow 使用单一 monorepo，是为了让共享协议、Desktop、Android 和原生
驱动在同一个提交与同一套 CI 中保持兼容。普通 `git clone` 会检出所有已
跟踪的源代码，但不会包含 Android SDK、Gradle 依赖缓存、`node_modules`、
Rust `target`、APK、EXE、DMG 等构建产物。

只开发 Android：

```powershell
git clone --depth 1 --filter=blob:none --sparse https://github.com/tianrking/LadoFlow.git
Set-Location LadoFlow
git sparse-checkout set apps/android docs
Set-Location apps/android
```

只开发通用 Desktop：

```powershell
git clone --depth 1 --filter=blob:none --sparse https://github.com/tianrking/LadoFlow.git
Set-Location LadoFlow
git sparse-checkout set apps/desktop crates docs
```

开发 Windows Host 和虚拟显示驱动时再增加：

```powershell
git sparse-checkout add platform/windows
```

需要恢复完整源码时执行：

```powershell
git sparse-checkout disable
```

普通用户不需要 clone 仓库。正式版本发布后，应直接从 GitHub Releases
下载自己平台对应的已签名安装包；CI 中的 debug APK、unsigned APK、未签名
Windows 安装包只用于开发和验证。
