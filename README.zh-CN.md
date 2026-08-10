<p align="center">
  <img src="./assets/brand/ladoflow-mark-256.png" width="176" alt="LadoFlow Logo">
</p>

<h1 align="center">LadoFlow</h1>

<p align="center">
  <strong>把身旁的闲置屏幕，变成流畅、私密的真正副屏。</strong>
</p>

<p align="center">USB 优先 · 全程本地 · 无需账号 · 开源透明</p>

<p align="center">
  <a href="./README.md">English</a> ·
  <a href="./README.es.md">Español</a> ·
  <a href="./README.zh-CN.md">简体中文</a>
</p>

> [!IMPORTANT]
> LadoFlow 目前处于**基础建设/预览前阶段**，尚未发布可用的副屏版本。下面会严格区分“已经开始实现的基础”与“计划支持的平台”。

## 我们要做什么

LadoFlow 的目标，是让 Windows、macOS 和 Linux 电脑把 Android 平板/手机、iPad 和 iPhone 当作扩展显示器使用。

第一阶段只把 USB 有线链路做稳，再加入可信局域网连接。最终体验应当是：安装电脑端，打开平板 App，插上连接线，自动识别并扩展桌面——核心功能不依赖账号、云中转或订阅。

## 当前真实状态

| 模块 | 当前状态 | 最终目标 |
| --- | --- | --- |
| 共享通信协议 | M1 消息与有界二进制帧已实现 | 版本化控制、视频、输入和遥测消息 |
| 共享运行时 | 能力协商、会话、重连策略、遥测、帧调度和内存 loopback 已实现 | 被所有主机端与显示端复用的跨平台运行时 |
| 桌面主机端 | Tauri 2 主机已包含诊断、托盘常驻、单实例、loopback 与按目标系统编译的原生适配器 | 同一外壳按目标系统接入原生服务 |
| macOS 主机端 | 权限/显示器发现、真实 ScreenCaptureKit IOSurface 探测和本地 `.app` 已实现 | 长时 capture/VideoToolbox 管线、原生虚拟显示适配和公证 Host |
| Windows 主机端 | 已实机验证采集、GPU H.264 和输入回传；单屏 IddCx 驱动、LocalSystem 生命周期服务及受限 IPC 客户端已通过构建验证 | 可信安装、自动选择虚拟屏、干净系统恢复测试和正式签名 |
| Linux 主机端 | 仅完成架构设计 | Wayland、X11、DRM 兼容路径 |
| Android 显示端 | 已实现 Kotlin/Compose、LDFL v1、AOA 与鉴权 USB tether、MediaCodec Surface 硬解、生命周期恢复、遥测和触摸/鼠标/键盘回传；JVM 与 Android 14 模拟器测试通过，尚缺实体线缆闭环 | 在支持的手机和平板上完成签名发行与兼容性验证 |
| iOS/iPadOS 显示端 | 仅完成架构设计 | Swift 原生接收、硬解和触控回传 |
| USB 传输 | Windows 与 Android 已实现直接 AOA 和前台限定的 USB tether 备用链路，包括有界 I/O、双向配对鉴权、有序 LDFL 控制/媒体、实时 H.264、输入回传和新会话恢复；尚未完成 Android 实机闭环 | 直连、鉴权、断线自动恢复 |
| Wi-Fi/局域网 | USB 稳定后开发 | 显式配对的本地连接 |

所有进度以可重复测试为准，详见[路线图](./docs/roadmap.md)。

## 为什么叫 LadoFlow

`Lado` 在西班牙语中表示“侧面、身旁”，`Flow` 表示流动、流畅和工作节奏。LadoFlow 想表达的是：**让身旁那块屏幕自然融入你的工作流。**

Logo 由两个相邻的圆角屏幕和一条连续路径组成：

- 大屏代表 Windows、macOS 或 Linux 主机；
- 小屏代表平板或手机；
- 青色路径隐约组成字母 `L`，代表本地画面与输入链路；
- 珊瑚色端点代表已经连接的显示设备；
- 图形没有使用任何平台或第三方产品标志。

完整说明和素材见[品牌指南](./docs/brand.md)。

## 技术原则

- 虚拟显示驱动、编解码、渲染、USB 和输入注入坚持使用平台原生能力。
- Rust 只共享真正稳定的协议、会话、能力协商和质量控制逻辑。
- Android 使用 Kotlin 原生界面与系统硬件解码。
- iOS/iPadOS 使用 Swift 和 Apple 原生多媒体框架。
- “流畅”必须通过端到端延迟、帧间隔、丢帧和重连测试证明，不能只看演示视频。

进一步阅读：[架构](./docs/architecture.md)、[协议](./docs/protocol.md)、[开发环境](./docs/development.md)。

## 完整下载或只取一个平台

所有平台放在同一个 monorepo，是为了让协议、Desktop、Android 和 CI 在同一
提交中保持兼容；开发单个平台时不必把所有文件都检出。

只取 Android：

```powershell
git clone --depth 1 --filter=blob:none --sparse https://github.com/tianrking/LadoFlow.git
Set-Location LadoFlow
git sparse-checkout set apps/android docs
Set-Location apps/android
```

只取 Desktop 与 Windows 原生组件：

```powershell
git clone --depth 1 --filter=blob:none --sparse https://github.com/tianrking/LadoFlow.git
Set-Location LadoFlow
git sparse-checkout set apps/desktop crates platform/windows docs
```

恢复完整工作树：

```powershell
git sparse-checkout disable
```

普通 clone 下载的是源代码和 Git 历史，不会包含 Android SDK、Gradle/Rust/
Node 缓存、APK、EXE 或 DMG。普通用户在正式版本发布后应直接下载对应平台的
GitHub Release，无需 clone。完整解释见[按平台获取源码](./docs/source-checkout.md)。

## 运行当前桌面基础

安装 Rust 1.97.1、Node.js LTS、pnpm 10.26.0，以及对应系统的
[Tauri 前置依赖](https://v2.tauri.app/start/prerequisites/)，然后执行：

```bash
pnpm install --frozen-lockfile
pnpm check
pnpm dev:desktop
```

当前应用会完成真实协议协商，并把合成帧送入有界媒体队列，同时展示帧率、丢帧与延迟遥测；它是可测试的主机基础，还不是可用的扩展屏。详见[开发环境](./docs/development.md)与[平台接手说明](./docs/platform-handoff.md)。

## 许可证

代码采用 [MIT License](./LICENSE)。项目名称和 Logo 用于识别官方项目，重新分发时不得暗示获得 LadoFlow 官方背书。
