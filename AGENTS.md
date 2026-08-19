# Lynx Launcher Agent Instructions

## 作用域

本文件适用于仓库根目录及其下所有项目文件，但不适用于 Git submodule
`third_party/lynx/` 内部。不要复制或改写上游大型 `AGENTS.md`；进入 submodule 只为只读
诊断时，遵守其中更具体的上游指令。项目对 Lynx 的改动必须落在 `patches/lynx/` 和根构建
机制中，不能把 submodule dirty worktree 当作最终改动。

## 项目心智模型

数据和调用方向固定如下：

```text
ReactLynx UI
    | NativeModules.Launcher Promise
    v
C++ host / N-API / Lynx windowless / GLFW + OpenGL
    | platform/include/lynx_launcher.h C ABI
    v
Rust platform / XDG discovery / icon lookup / process launch
```

- Rust 拥有 Linux 应用策略、`.desktop` 解析、图标定位和进程启动。
- C++ host 拥有 Rust ABI adapter、N-API、Lynx embedder、runtime resources、窗口、输入、
  OpenGL 和任务队列。
- ReactLynx UI 拥有数据验证、搜索、显示和交互状态，不拥有 OS policy。
- `third_party/lynx/` 是 pinned implementation dependency，不是第四个应用层。
- 详细数据流、线程和依赖锁定见 `ARCHITECTURE.md`。

## 目录所有权

| 路径 | 允许的职责 |
| --- | --- |
| `platform/src/lib.rs` | XDG discovery、Desktop Entry policy、Exec parser、icon lookup、launch。 |
| `platform/src/ffi.rs` | Rust C ABI implementation、panic containment、opaque handle ownership。 |
| `platform/include/lynx_launcher.h` | C ABI public contract；必须与 `ffi.rs` 同步。 |
| `platform/tests/` | discovery、Exec 安全、icon 和 ABI ownership 回归。 |
| `lynx-sys/` | 最小 Lynx raw binding、native linkage 和后续严格 C shim。 |
| `host-rs/` | side-by-side Rust host；当前只拥有 CLI、support logic 和无窗口 resource/link check。 |
| `host/src/main.cc` | Lynx/GLFW host、N-API Promise、线程、输入和 lifecycle。 |
| `host/src/support.*` | 可独立测试的 path、file、URI、UTF-8 支持逻辑。 |
| `host/tests/`、`host/cmake/` | native tests、X11 E2E driver、resource refresh checks。 |
| `ui/src/platform.ts` | 唯一的 UI native-module boundary。 |
| `ui/src/applications.ts` | native data validation 和纯应用列表逻辑。 |
| `ui/src/App.tsx`、`ui/src/App.css` | ReactLynx state、elements、events 和视觉。 |
| `scripts/` | 根级 reproducible workflow；优先修改这里，不建立平行入口。 |
| `patches/lynx/` | 受 allowlist 管理的临时上游 integration patches。 |

## 固定工具链

- Linux x86_64 是唯一已 bootstrap 和验证的平台。
- `rust-toolchain.toml` 固定 Rust `1.93.0`，含 rustfmt 和 Clippy；根 Cargo workspace 使用
  `Cargo.lock` 与 `--locked`。
- `.nvmrc` 固定 Node.js `22.14.0`。不要用“兼容的任意 Node”替代 exact pin。
- `ui/package.json` 固定 `packageManager: pnpm@10.34.5`；通过 Corepack 调用并使用
  `ui/pnpm-lock.yaml` frozen install。不要直接使用系统 `pnpm` 漂移版本。
- `third_party/lynx` 当前 gitlink 是
  `a573c3b8280180b59ca3da3e33d7a50192334cce`；该 checkout 提供 Habitat `0.3.149` 和
  pinned DEPS graph。
- host 使用 C++20、CMake 3.16+、verified Lynx SDK、系统 fontconfig development files 和
  pinned Lynx DEPS 中的 GLFW。
- 不要让脚本运行 `sudo` 或安装系统软件包；缺依赖时报告具体 command。

## 首选根脚本

脚本从自身位置解析路径，不要求 caller 的 current directory。除窄范围诊断外，使用以下
入口，不手工拼装替代流程：

| 目的 | 命令 |
| --- | --- |
| 构建 pinned/patch SDK | `./scripts/bootstrap.sh` |
| 全项目构建 | `./scripts/build.sh` |
| 无图形完整测试 | `./scripts/test.sh` |
| 首帧图形测试 | `LYNX_LAUNCHER_SMOKE=1 ./scripts/test.sh` |
| 搜索、图标、启动 E2E | `./scripts/e2e-launch.sh` |
| 生命周期 stress | `./scripts/teardown-stress.sh` |
| 运行 | `./scripts/run.sh` |
| 无窗口 ABI/resource check | `./scripts/run.sh --check-resources` |

`./host/build.sh` 仅用于已有 `ui/dist/main.lynx.bundle` 后的 host/Rust/CMake 增量构建与
CTest；它不能代替 UI 验证。

## 最小验证矩阵

| 改动类型 | 交付前最小验证 |
| --- | --- |
| 仅 Markdown 文档 | 检查本地 links、文中 scripts/path；`git diff --check` |
| Rust discovery、Exec、icon、launch | `./scripts/test.sh` |
| C ABI 或 header | `./scripts/test.sh`；确认 ABI/resource check 实际执行 |
| UI data validation、search 纯逻辑 | `./scripts/test.sh` |
| UI layout、input、icon rendering、launch interaction | `./scripts/test.sh`；首帧 smoke；`./scripts/e2e-launch.sh` |
| C++ support、CMake、runtime resource copy | `./scripts/test.sh` |
| GLFW/input/render/task queue/lifecycle | `./scripts/test.sh`；首帧 smoke；相关 E2E；`./scripts/teardown-stress.sh` |
| `scripts/bootstrap.sh`、SDK provenance、gitlink、Lynx patch | `./scripts/bootstrap.sh`；`./scripts/test.sh`；首帧 smoke；`./scripts/teardown-stress.sh` |
| E2E driver 或 process cleanup | `./scripts/e2e-launch.sh`，用 `LYNX_LAUNCHER_E2E_ITERATIONS` 重复 |

## 最终门禁

- 所有改动都必须运行 `git diff --check`。
- 所有非文档改动都必须运行 `./scripts/test.sh`；不要用单层测试冒充最终门禁。
- 影响可见 UI、输入、图标、启动、renderer 或 lifecycle 的改动，在可用 X11/XWayland
  环境中还必须运行 `LYNX_LAUNCHER_SMOKE=1 ./scripts/test.sh`、相关 E2E 和
  `./scripts/teardown-stress.sh`。
- 影响 Lynx pin/patch/provenance 的改动还必须确认
  `git -C third_party/lynx status --short` 为空，HEAD 等于 parent index gitlink。
- 没有 `DISPLAY` 时不得声称图形门禁通过；明确报告未执行项，不用 native Wayland、
  mock 或 `--check-resources` 替代。

## Rust 约束

- 保持 OS-specific behavior 在 `cfg` boundary 内；不支持的平台返回
  `UnsupportedPlatform`，不要静默模拟 Linux。
- XDG directory priority 和 desktop ID masking 是行为契约。高优先级同 ID 条目即使
  hidden/invalid 也不能意外让低优先级条目复活。
- launch 只能用 `std::process::Command::new(program).args(arguments)`。命名 reaper
  thread 负责 `wait`，UI thread 只等待 spawn success/failure，不等待应用退出。
- 新行为应进入 `platform/tests/`，尤其覆盖不可信 `.desktop` input 和优先级边界。
- 保持 `cargo fmt`、locked tests 和 `cargo clippy ... -D warnings` 全部通过。

## `.desktop` 安全规则

`.desktop` 文件是外部、不可信输入。禁止把 `Exec` 拼成 shell command，禁止
`sh -c`、`system`、`popen` 或任何等价 shell interpretation。

- 解析为 program 与 argv，field-code expansion 后直接交给 `Command`。
- 保持 NUL、invalid escape、unterminated quote、unsupported field code、危险 token
  placement 和 executable name 中 `=` 的拒绝逻辑。
- `%f/%F/%u/%U` 当前不接收 file/URI arguments；`%i`、`%c`、`%k` 的 expansion 必须
  保持参数边界，不能二次解释。
- 保持 `Type`、`Hidden`、`NoDisplay`、`Terminal`、`OnlyShowIn`、`NotShowIn`、
  `TryExec`、locale 和 XDG precedence 语义。
- 修改 parser 或 launch 前先阅读 `platform/tests/discovery.rs` 中的 shell-injection marker
  regression；不得通过放宽测试绕过安全约束。

## Rust C ABI 约束

- `platform/src/ffi.rs` 与 `platform/include/lynx_launcher.h` 是一个 contract；签名、status、
  struct layout 或 ownership 改动必须同步，并判断是否递增 `LYNX_LAUNCHER_ABI_VERSION`。
- ABI 只暴露 `#[repr(C)]` value、opaque handles、显式 status 和 length-delimited UTF-8
  `LynxSlice`；slice 不保证 NUL termination。
- 每个成功分配的 launcher/list/icon/error 必须由 matching destroy function 恰好释放一次；
  destroy 接受 NULL。borrowed slice 只活到 owner 被 destroy。
- status-returning call 在工作前清空 output/error slot；不要覆盖仍 live 的 handle。
- panic 必须在 ABI boundary 由 `catch_unwind` containment，绝不 unwind 进 C++。
- C++ 消费 handle 时优先保持现有 RAII deleter；输入 string 必须按明确 length 转换。

## C++ Host 约束

- 不把 `.desktop` policy、XDG discovery 或 launch command construction 移入 C++。
- `NativeModules.Launcher.getApplications()` 和 `launchApplication(id)` 返回真实 N-API
  Promise。错误必须 reject，不要改成同步 throw/return 或吞掉 Rust status。
- runtime paths 默认相对 executable，不能依赖 caller current directory。继续使用
  `launcher_host` 的 file/path/URI/UTF-8 支持逻辑及其 tests。
- resource fetcher 当前只服务 packaged local `lynx_core.js`；不要未经 provenance 验证
  直接开放任意 filesystem/network fetch。
- 保持 C++20 和 CMake target dependency；不要从 `third_party/lynx/out/Default` 链接 loose
  SDK files。

## Rust Host Tracer 约束

- `host-rs` 当前是 side-by-side 无窗口 tracer，不是默认 host。`scripts/run.sh`、smoke、E2E
  和 teardown 继续使用 `host/build/lynx-launcher`，直到独立 parity gate 完成。
- tracer 必须从 executable 相对位置读取 staged runtime，并核验 `lynx_log_init` 实际来自同目录
  的 `liblynx.so`；不能用只读资源文件冒充 native linkage 验证。
- `lynx-sys` 只暴露已核对的窄 binding。五个 C++ float-reference wrapper 属于后续 renderer
  阶段，必须作为严格 `extern "C"` by-value shim 落在该模块，不能硬编码 C++ reference ABI。
- Rust host 直接使用 `platform` Rust interface；现有 C ABI 在 C++ host 移除前继续保留并测试。

## ReactLynx 约束

- 只使用 Lynx elements、events 和 APIs，不引入 browser DOM、`window`、`document` 或 web
  component assumptions。
- 所有 native access 保持集中在 `ui/src/platform.ts`；UI 消费前由
  `validateApplications` 验证 array、item、nonblank/unique ID、name 和 `iconUri`。
- Native API shape 改动必须同步 `ui/src/native-modules.d.ts`、C++ N-API implementation、
  validation/tests 和架构文档。
- 搜索按 trimmed、case-insensitive application name 工作；missing/blank/failed icon 保持
  deterministic initial fallback。
- 保留 ReactLynx event handler 所需的现有 `'background only'` directives。不要因熟悉
  React DOM 而改写为 browser event shape。

## Lynx Submodule 与 Patch

- parent index 固定 `third_party/lynx` SHA；禁止 `git submodule update --remote`，禁止跟随
  `develop` 或其他 moving branch。
- 禁止直接在 submodule 留 dirty changes，禁止把手工 edit 当项目交付。需要上游改动时，
  生成项目拥有的 patch，加入 `scripts/_common.sh` 的 fixed ordered allowlist，并重新验证。
- `scripts/bootstrap.sh` 先拒绝已有 dirty worktree，再在 build 前 `git apply --check` 和
  temporary apply。EXIT trap 在成功、失败、INT、TERM 后按逆序 reverse，并检查 pinned
  HEAD 与 clean worktree。不要删除、绕过或延后该 cleanup。
- bootstrap 的 exclusive `.build-home/bootstrap.lock` 必须覆盖 submodule、shared cache、
  stable log、reverse patch 和最终 clean check；不要缩小 lock critical section。
- gitlink update 是明确的 dependency upgrade。必须重新评估或删除现有 patch，生成新
  source key，并跑完整 SDK、ABI/resource、首帧和 teardown 门禁。
- Linux SDK 的 fontconfig patch 使 `liblynx.so` 依赖系统 `libfontconfig.so.1`；不要退回
  `SkFontMgr_New_Custom_Directory`，它不实现 per-glyph fallback。

## Resource Provenance

- source key 固定为 `<pinned-lynx-sha>-<ordered-patch-set-sha256>`。
- 只有校验过 SHA256 且 `lib/liblynx.so` 与 patched output 匹配的
  `lynx_sdk_linux_x64.zip` 可以进入 `.build-home/sdk-cache/<source-key>/`。
- 只有 `.build-home/sdk/<source-key>/` 下与 archive 逐 byte 比对过的 `lib/`、`include/`、
  `data/icudtl.dat`、`lynx_core.js` 可以传给 CMake。
- 禁止使用或复制 `third_party/lynx/out/Default` 的 loose `liblynx.so`、`icudtl.dat`、
  headers 或 core JS；packaged ICU 可能与 loose file 不同。
- runtime resources 由 CMake `copy_if_different` target 刷新。不要手工复制到
  `host/build/`，也不要用 mtime 判断 provenance。
- UI bundle 必须由 pinned pnpm/frozen lockfile 生成，再由同一 CMake resource target
  复制到 runtime。

## X11 与 XWayland

- 当前 CMake 明确设置 `GLFW_USE_WAYLAND=OFF`；GLDirect baseline 是 X11 OpenGL 3.3。
- Wayland desktop 只能通过 XWayland 运行，并要求 `DISPLAY`。不要声称 native Wayland
  support，也不要只翻转 CMake switch。
- native Wayland 需要重新验证 clipboard、IME/text input、cursor、scale、context ownership、
  first-frame、E2E 和 teardown，属于独立架构改动。

## 并发与生命周期

- platform thread 创建 GLFW/Host、pump events，并运行 process-global Lynx UI runner；每个
  process 只能有一个 active host。
- UI tasks 和 renderer tasks 使用 mutex-protected deadline queues；跨线程 post 用
  `glfwPostEmptyEvent` 唤醒 event loop。
- 第一个取得 OpenGL context 的 render thread 成为 stable owner；禁止其他 thread
  make-current、present 或 clear-current。
- shutdown 顺序不可随意调整：cancel input；background/release view 和 client；停止接收
  renderer tasks；在 runner 仍 live 时 drain renderer/UI queues；再释放 renderer、fetcher、
  Rust handles、cursor、window/GLFW resources。
- 停止接收任务后不能保留或 reenter 新 work。涉及该顺序的改动必须执行 teardown stress。
- E2E cleanup signal 前必须继续核验 executable、process group 和 process start identity，
  最后 `wait` child，避免 PID reuse 误杀。

## 已知上游日志与门禁禁词

当前 pinned SDK 的成功运行中已知存在以下上游 noise；其出现本身不等于项目失败：

- bootstrap 的 `COMMANDLINE_TOOL_BASE_DIR is not set` NOTICE。
- `NetLoaderManager::SetupCache!` warning。
- 查找未注册 optional `LynxResourceModule`、`LynxAccessibilityModule`、`LynxSetModule` 的
  warnings。
- successful teardown 可能以 ERROR level 打印 `load original lynx so`、duplicated
  `timing_key`、`ElementManager::WillDestroy`、`~HostGlobal`、`~JSIContext` 和
  `LYNX free quickjs runtime start`。

不要扩大此名单或静默过滤新日志。以 command exit status、首帧条件和脚本明确规则为准。
teardown summary 会统计 `DestroyLayoutNodeBeforeRemoveFromParent` 与
`target view: ... not found`，当前不因非零计数单独失败，但新增或增长时必须检查。

以下四个字符串是 teardown gate 禁词，任何一次出现都失败：

```text
destroyed thread host
Maybe leaked
post an unknown task
LoadJSSource load js error
```

此外，timeout、未出现 `[host] first GL frame presented`、host non-zero exit、
`[lynx-error ...]` 或 test command non-zero 都是失败，不能归类为已知 warning。

## 生成物与 Git

- 不提交 `.build-home/`、`.logs/`、`host/build/`、`platform/target/`、`ui/node_modules/`、
  `ui/dist/`、`*.tsbuildinfo`、`quickjs_cache/` 或 submodule `out/Default` 产物。
- `Cargo.lock`、`ui/pnpm-lock.yaml`、gitlink 和 `patches/lynx/*.patch` 是 source，不是
  cache；有意变更时保留并验证。
- 未经用户明确授权不得 `git commit`、amend、push 或创建 PR。
- 不 revert、reset、checkout 或覆盖用户及其他 agent 的无关改动。遇到相关文件并发冲突时
  停止并说明；无关 dirty files 保持不动。
- 交付前检查 `git status --short`，只报告本任务实际修改和未执行的验证。
