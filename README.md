# Lynx Launcher

Lynx Launcher 是一个 Linux 桌面应用启动器：Rust 平台层读取 XDG
`Desktop Entry`，C++20 原生 host 通过 C ABI 和 N-API 将数据交给
ReactLynx，UI 负责搜索并异步发起启动。窗口由 GLFW/OpenGL 创建，Lynx 以
windowless embedder 方式渲染。

## 3 分钟上手

以下命令在仓库根目录执行。根脚本按自身位置解析路径，也可以从其他工作目录调用。

```sh
nvm install
nvm use
./scripts/bootstrap.sh
./scripts/build.sh
./scripts/test.sh
./scripts/run.sh
```

首次 `bootstrap` 会通过 Habitat 下载 Lynx 工具和第三方源码，缓存约 4 GB，
可能耗时较长。`./scripts/build.sh` 在 SDK 缺失、过期或不完整时会自动执行
bootstrap；显式执行便于首次构建时单独观察 `.logs/bootstrap.log`。项目脚本不运行
`sudo`，也不安装系统软件包。

Linux SDK 的 launcher-specific 体积 profile、实测结果和能力取舍见
[`LYNX_SDK_SIZE.md`](LYNX_SDK_SIZE.md)。

## 已实现能力

- 从 XDG data directories 发现 freedesktop `Desktop Entry` 应用。
- 遵循 locale、`TryExec`、可见性及用户目录优先于系统目录的覆盖规则。
- 解析绝对图标路径、常见 hicolor 尺寸、scalable 图标和 pixmaps。
- 在 ReactLynx 中搜索应用，通过真实 N-API Promise 异步报告启动结果。
- 通过 Lynx windowless API 转发窗口、指针、键盘、滚轮、剪贴板、光标和基础文本输入。
- 以无边框、置顶且不进入任务栏或分页器的 X11 launcher 弹窗运行，失焦后退出。
- 在 X11/XWayland 下读取 GLFW content scale 与 XSettings
  `Gdk/WindowScalingFactor`，同步原生窗口、Lynx DPR、逻辑 viewport 和输入坐标。
- 通过 fontconfig 匹配系统与用户字体，为中文及其他缺失 glyph 提供字体回退。
- 无窗口检查 Rust ABI 与打包运行资源；可选执行首帧图形 smoke。
- 提供搜索、图标渲染、启动链路 E2E，以及重复退出生命周期 stress。

## 架构

```text
ReactLynx UI（TypeScript，搜索与交互状态）
        |
        | NativeModules.Launcher Promise
        v
C++ host（N-API + Lynx C API + GLFW/OpenGL）
        |
        | platform/include/lynx_launcher.h C ABI
        v
Rust platform（XDG 发现、图标解析、进程启动）

C++ host --> 已验证的 pinned Lynx SDK
```

UI 不依赖浏览器 DOM；C++ host 拥有 embedder、图形、输入和任务队列；Rust 拥有
操作系统策略与 `.desktop` 解析。完整边界、数据流和所有权说明见
[ARCHITECTURE.md](ARCHITECTURE.md)。

迁移中的 `host-rs` 是 side-by-side Lynx view tracer：它验证 Rust CLI、support logic、
platform direct interface、staged `liblynx.so` linkage、pinned GLFW/X11/OpenGL 窗口，
加载 packaged core 与 bundle 渲染真实 launcher，并已迁移 pointer、wheel、keyboard、character、
focus、scale handling 和直接 Rust application launch。它仍不替代上图中的默认 C++ host。

## 目录

| 路径 | 所有权 |
| --- | --- |
| `platform/` | Rust 应用发现、启动、图标解析、稳定 C ABI 及测试。 |
| `lynx-sys/` | 最小 Lynx raw binding 与 native library path probe。 |
| `host-rs/` | Rust host 迁移 tracer；当前支持 resource/link check、popup GL shell、真实 Lynx view、input，以及 async application launch。 |
| `ui/` | ReactLynx/TypeScript UI，Rspeedy 输出 bundle。 |
| `host/` | C++20 embedder、GLFW/OpenGL、N-API bridge、CMake 与 native tests。 |
| `scripts/` | 首选的 bootstrap、构建、测试、运行和图形测试入口。 |
| `patches/lynx/` | 构建 SDK 时临时应用的 allowlisted Linux windowless 补丁。 |
| `third_party/lynx/` | pinned Lynx Git submodule 及其 SDK 构建中间产物。 |
| `.build-home/` | Git 忽略的 Habitat/Corepack 缓存、SDK archive cache 和 verified SDK。 |
| `.logs/` | Git 忽略的 bootstrap 与 teardown stress 日志。 |

## 已验证基线

| 组件 | 版本 |
| --- | --- |
| Linux | 6.18.42-1-lts x86_64 |
| Rust | rustc/cargo 1.93.0 |
| Node.js | 22.14.0 |
| pnpm | 10.34.5，通过 Corepack |
| Python | 3.12.2 |
| CMake | 4.4.2 |
| Ninja | 1.13.2 |
| GCC / Clang | GCC 16.2.1 / Clang 22.1.8 |

`rust-toolchain.toml` 固定 Rust `1.93.0`，`.nvmrc` 固定 Node.js
`22.14.0`，`ui/package.json` 固定 pnpm `10.34.5`。Node.js
`22.14.0` 满足 UI 的 `^20.19.0 || >=22.12.0` engine 约束，同时避免
Rspeedy/Corepack 行为随本机版本漂移。

## 依赖

运行脚本前由系统提供以下依赖；本项目不提供发行版安装命令。

- Linux x86_64、Git、curl、Bash、Python 3.9+、`unzip`、`sha256sum`、
  `cmp`、`flock`、`mktemp`、`timeout`、`setsid`、`readlink`、`grep` 和 `tee`。
- C/C++20 compiler、CMake 3.16+，以及 Ninja 或 Make 等 native build tool。
- OpenGL development files，以及 GLFW 所需的 X11 development headers；多数发行版对应
  `X11`、`Xrandr`、`Xinerama`、`Xcursor`、`Xi`。
- fontconfig development files；patched Lynx SDK 构建时需要 headers 与 linker metadata，
  运行时需要 `libfontconfig.so.1` 和至少一款覆盖所需字符的已安装字体。
- Rustup；它读取 `rust-toolchain.toml` 并提供 rustfmt、Clippy 和 Cargo。
- 支持 `.nvmrc` 的 Node version manager 和 Corepack；本地 Node 可能需要先启用 Corepack。
- 图形运行和图形测试需要 X11 或 XWayland session，并设置 `DISPLAY`。native X11
  root capture 不要求额外截图工具；niri/XWayland 的可见像素门禁要求 `niri` 与 `grim`
  都在 `PATH`，脚本会在启动 driver 前明确检查。

## 构建与运行

### 首次构建

```sh
nvm install
nvm use
./scripts/bootstrap.sh
./scripts/build.sh
```

构建使用 frozen pnpm lockfile 生成 `ui/dist/main.lynx.bundle`，再构建根 Rust workspace
与 C++ host。workspace 同时产出 platform static library 和 side-by-side Rust tracer；只会
把 verified SDK directory 传给 CMake/Cargo。

### 增量构建

全项目增量构建仍使用根入口：

```sh
./scripts/build.sh
```

已有 `ui/dist/main.lynx.bundle` 时，可只走 host/Rust/CMake 构建并执行 CTest：

```sh
./host/build.sh
```

runtime resource target 每次构建都执行 content-aware `copy_if_different`。即使切回
mtime 更早的 verified SDK，内容也会正确刷新；相同内容不会改写。CTest 覆盖该
older-source/newer-target 回归场景。

### 运行

```sh
./scripts/run.sh
./scripts/run.sh --help
./scripts/run.sh --check-resources
```

`run.sh` 从 executable 相对位置寻找完整 runtime，缺失时会提示先运行
`scripts/build.sh`。`--check-resources` 不创建窗口；host 还支持 `--bundle PATH`、
`--lynx-core PATH`、`--icu PATH`、`--run-for SECONDS` 和
`--exit-after-first-frame`。

迁移 tracer 可单独执行：

```sh
./host/build/lynx-launcher-rs --check-resources
./host/build/lynx-launcher-rs --run-for 10
./host/build/lynx-launcher-rs --exit-after-first-frame
```

它会额外确认实际加载的 Lynx symbol 来自 executable 同目录的 staged `liblynx.so`。
window mode 使用 CMake 构建的 pinned GLFW static archive，先提交独立静态 shell frame，
再由 Rust-owned fetcher、builder、view、client 和 weak N-API `Launcher` module 加载 staged
core 与 bundle。`[host-rs] first shell GL frame presented` 仍不代表 Lynx readiness；只有
`[host-rs] first screen layout completed` 与 `[host-rs] first GL frame presented` 同时出现后，
`--exit-after-first-frame` 才会退出。`getApplications()` 返回同一 Rust `Launcher` 的 platform
direct snapshot；`launchApplication()` 立即返回真实 Promise，在 N-API worker 调用该 launcher，
再由 JS-thread completion resolve/reject。默认图形入口仍是 `scripts/run.sh`。

## 测试

### 普通测试

```sh
./scripts/test.sh
```

该入口依次执行 Rust workspace format check、locked Clippy（warnings denied）和全部
Rust tests；UI tests、typecheck 和 production build；host build、CTest；最后执行 C++
与 Rust 两套资源检查。默认路径不需要 display。

### 首帧 smoke

仅在 X11/XWayland session 中启用：

```sh
LYNX_LAUNCHER_SMOKE=1 ./scripts/test.sh
LYNX_LAUNCHER_SMOKE=1 LYNX_LAUNCHER_SMOKE_TIMEOUT=45s ./scripts/test.sh
```

进程必须在 timeout 前同时报告 first-screen layout 和首个 GL present。
该入口还会运行 Rust tracer 的 popup、真实 launcher 输入、application Promise、focus-loss
退出和双条件首帧自动退出检查；静态 shell marker 继续单独断言，不能冒充 readiness。
Rust smoke 使用隔离 XDG fixture 和可观察的 1.25 system scale，精确核对 3 个 application
snapshot 的 ID、名称与顺序；通过共享 X11 driver 聚焦搜索框、输入 `cobalt`、确认只有目标
绿色 icon 和过滤后像素、点击目标，并在 discovery 后删除 fixture executable，确认真实 spawn
failure 的 rejected Promise 与 native detail 驱动现有 `ActionError`。输入检查使用 X server 的
32-bit modular signed-delta 时间顺序验证真实 key
down/up（含 49.7 天 wrap 自测），再覆盖 scroll DPR 换算、key repeat，以及 focus loss 对 held
modifier 和 primary pointer 的合成释放；background 后禁止任何后续 input dispatch。另有
startup focus-loss 用例验证 runtime userdata 安装前的 FocusOut 不会丢失。同一 native module
seam 还会注入一次测试专用 discovery 错误。重复次数同时应用于 first-frame 与 bounded input
模式；每轮都执行完整过滤、launch rejection、`ActionError` 像素、真实键入、scroll/repeat、
held-input cancellation 和 focus teardown，并要求 clean shutdown；日志保存在
`.logs/ticket-06/rust-desktop-smoke/`。

desktop integration gate 还通过 XFixes cursor image fingerprint 核对普通区域的 arrow
fallback 与搜索输入框的 I-beam，并执行 10 次 cursor teardown。clipboard read/write 使用独立
X selection namespace：Wayland session 优先启动 standalone `Xwayland`，否则回退 `Xvfb`；
两者都不可用时 gate 会明确失败，不会在用户的 `DISPLAY` 上接管 `CLIPBOARD`，也不会静默
跳过。CI 若已提供专用 X server，可设置 `LYNX_LAUNCHER_TEST_ISOLATED_DISPLAY=1` 明确声明
当前 `DISPLAY` 可用于 selection ownership。隔离测试用外部 owner 提供 `cobalt`，经真实
Ctrl+V 验证 Lynx get callback，再经 Ctrl+A/C 和独立 reader 验证 set callback。

也可单独执行并调整重复次数（默认 10）：

```sh
./scripts/rust-shell-smoke.sh
LYNX_LAUNCHER_RUST_SHELL_ITERATIONS=10 ./scripts/rust-shell-smoke.sh
```

### 搜索 + 图标 + 启动 E2E

```sh
./scripts/e2e-launch.sh
LYNX_LAUNCHER_E2E_ITERATIONS=10 ./scripts/e2e-launch.sh
LYNX_LAUNCHER_E2E_HOST=rust ./scripts/e2e-launch.sh
LYNX_LAUNCHER_E2E_HOST=rust LYNX_LAUNCHER_E2E_SCENARIO=unknown \
  LYNX_LAUNCHER_E2E_ITERATIONS=10 ./scripts/e2e-launch.sh
```

测试创建三个隔离的临时 `.desktop` fixture，不启动已安装应用。它通过纯观察的 root 或
compositor capture 比较两个不同中文 glyph 的渲染区域，拒绝重复缺字方框；再通过 GLFW X11 key callbacks
输入 ASCII query，验证筛选目标的独特色 hicolor SVG 已到达最终 icon region，点击卡片并
确认只有目标 desktop ID 的 marker 出现。默认仍验证 C++ host。Rust 模式的 `success`、
`spawn-failure`、`unknown` 和 `immediate-defocus` 场景可独立选择；`all` 为默认。success helper
写入 started 后至少运行 5 秒，测试要求 Promise resolved 时 exited 尚不存在，证明 Promise
不等待 child。failure 与 unknown 分别验证真实 spawn detail 和 `ApplicationNotFound`，两者都
必须显示 `ActionError`。`LYNX_LAUNCHER_E2E_SUCCESS_ITERATIONS`、
`LYNX_LAUNCHER_E2E_SPAWN_FAILURE_ITERATIONS`、`LYNX_LAUNCHER_E2E_UNKNOWN_ITERATIONS` 和
`LYNX_LAUNCHER_E2E_IMMEDIATE_DEFOCUS_ITERATIONS` 可在 `all` 模式分别覆盖重复次数。

### Teardown stress

```sh
./scripts/teardown-stress.sh
LYNX_LAUNCHER_TEARDOWN_ITERATIONS=20 ./scripts/teardown-stress.sh
LYNX_LAUNCHER_TEARDOWN_TIMEOUT=45s ./scripts/teardown-stress.sh
```

默认执行 10 次 bounded launch 和 10 次 first-frame close。每次都必须 present 一帧，
且日志不得出现 `destroyed thread host`、`Maybe leaked`、`post an unknown task`、
`LoadJSSource load js error`。每次并发 invocation 使用独立的
`.logs/teardown-stress/run.XXXXXX/`，保留逐次日志和 `summary.txt`；summary 也记录
`DestroyLayoutNodeBeforeRemoveFromParent` 与 `target view: ... not found` 的残余计数。

## Lynx Pin、补丁与 verified SDK

- `third_party/lynx` 的 gitlink 固定为
  `a573c3b8280180b59ca3da3e33d7a50192334cce`。bootstrap 只 checkout Git index 中的
  exact SHA，不使用 `git submodule update --remote`，也不跟随 moving `develop`。
- 固定 allowlist 当前依次包含
  `patches/lynx/0001-linux-windowless-teardown.patch` 和
  `patches/lynx/0002-linux-fontconfig-fallback.patch`。未知 patch、symlink patch、顺序变化或
  submodule 本地修改都会失败，不会被覆盖。
- bootstrap 在隔离的 `.build-home/` HOME 内执行 `tools/hab sync . --target clay`，随后
  按固定顺序临时 apply patch，再执行
  `python3 platform/linux/build_release.py --target-cpu x64`。EXIT trap 在成功、失败和
  signal 后都按逆序 reverse patch，并验证 pinned HEAD 与 clean worktree。
- teardown patch 让 service 在 Clay task runners 存活时销毁，并阻止 pending closure
  cleanup 误判或保留 reentrant work。它还为 `enable_unittests=true` 定义 Linux-only
  `embedder_task_runner_unittests`，但该 target 不是 packaged SDK dependency。
- fontconfig patch 同时启用 Skia 与 Clay txt 的 fontconfig backend，并补齐缺失的 GN system
  library target，使 SkParagraph 的 per-glyph fallback 能匹配系统和用户字体。上游 Linux SDK
  提供等价 fallback 后应删除该 patch。
- bootstrap 用 `.build-home/bootstrap.lock` 的 exclusive `flock` 串行保护 submodule、
  shared cache、日志和 reverse cleanup；
  `LYNX_LAUNCHER_BOOTSTRAP_LOCK_TIMEOUT` 是正整数秒数，默认等待 1800 秒。
- SDK provenance key 是 `<pinned-lynx-sha>-<patch-set-sha256>`。archive cache 位于
  `.build-home/sdk-cache/<source-key>/`，verified extraction 位于
  `.build-home/sdk/<source-key>/`。
- `lynx_sdk_linux_x64.zip` 是 launcher 唯一接受的 SDK 来源。bootstrap 校验生成的
  SHA256、确认 archive 内 `lib/liblynx.so` 与 patched build output 一致、拒绝危险路径，
  并验证 `lib/`、`include/`、`data/icudtl.dat`、`lynx_core.js` 的完整布局。复用 extraction
  时，每个文件都与 ZIP entry 逐 byte 比较，同时核对 archive SHA256 和完整 source key。
- `third_party/lynx/out/Default` 的 loose files 只是中间产物，绝不传给 CMake。尤其 packaged
  `data/icudtl.dat` 可能不同于 loose `out/Default/icudtl.dat`，以 verified ZIP 为准。

## 运行产物

| 路径 | 内容 |
| --- | --- |
| `ui/dist/main.lynx.bundle` | Rspeedy production bundle。 |
| `host/build/lynx-launcher` | 默认 C++ 可执行文件。 |
| `host/build/lynx-launcher-rs` | side-by-side Rust resource/link 与 popup GL shell tracer。 |
| `host/build/liblynx.so` | verified SDK shared library。 |
| `host/build/lynx_core.js` | engine 默认 `$ORIGIN/lynx_core.js` lookup。 |
| `host/build/resources/main.lynx.bundle` | host 显式加载的 UI bundle。 |
| `host/build/resources/lynx_core.js` | host 显式 resource path。 |
| `host/build/resources/icudtl.dat` | verified packaged ICU data。 |
| `.logs/bootstrap.log` | 最近一次串行 bootstrap 完整输出。 |
| `.logs/teardown-stress/run.XXXXXX/` | 每次 stress 的独立日志与 summary。 |

## 已知限制

- 只有 Linux x64 已 bootstrap 并验证。
- host 固定 `GLFW_USE_WAYLAND=OFF`，要求 X11/XWayland、`DISPLAY` 和 OpenGL 3.3；尚不支持
  native Wayland。
- XSettings 窗口缩放在启动时读取；运行中修改系统缩放需要重启 launcher，且当前 X11
  baseline 不提供逐显示器 fractional scaling。
- 只实现 Desktop Entry specification 的实用子集。Terminal 应用与 file/URI launch
  arguments 被有意忽略，D-Bus activation 当前 fallback 到 `Exec`。
- icon lookup 不检测当前 theme、不读取 `index.theme` inheritance，也未实现完整 HiDPI
  algorithm；找不到或加载失败时 UI 显示应用名称首字母。
- 文本输入只转发 character events，尚无完整 IME composition protocol。
- 字体回退依赖宿主机 fontconfig 配置和已安装字体；系统没有覆盖目标字符的字体时仍会显示
  缺字方框。
- resource fetcher 只提供 packaged local Lynx core，不支持任意 network resources。
- process-global windowless UI runner 限制每个进程只能有一个 active launcher host。
- 尚无 installer、desktop integration 或 binary distribution 流程。
- 默认测试只做无图形资源验证；首帧 rendering 必须显式启用 smoke，并具备可用的
  X11/XWayland display。
