# Lynx SDK Size Profile

本文记录 Lynx Launcher 对 pinned Linux x64 Lynx SDK 的体积分析、精简策略、
兼容性约束和验证结果。优化由项目拥有的
`patches/lynx/0003-linux-launcher-size-profile.patch` 实现，不直接修改或提交
`third_party/lynx/` submodule worktree。

## 结论

最终 `liblynx.so` 从 `41,143,432` bytes 降到 `21,688,440` bytes，减少
`47.3%`。发布用 stripped runtime 的 zstd 体积从约 `15.75 MiB` 降到
`9.49 MiB`，减少 `39.8%`。

语言层 host 不是主要体积来源。收益主要来自收窄 `liblynx.so` 的动态导出面，
其次是移除 launcher 不使用的开发与运行时编译能力。

## 测量基线

测量环境和契约：

- Lynx gitlink：`a573c3b8280180b59ca3da3e33d7a50192334cce`。
- baseline source key：
  `a573c3b8280180b59ca3da3e33d7a50192334cce-ccbe8330483d836b46f953183406a7e0adcdac4d596b56e1f7bd7e25beb9e8c5`。
- final source key：
  `a573c3b8280180b59ca3da3e33d7a50192334cce-e0b679b95f70e9fe0163490deb3c190f3151e8674f1ea0cf555abb60a69c45e6`。
- 两个 `liblynx.so` 都是 official、stripped Linux x64 SDK 产物。
- SDK ZIP 由 `platform/linux:package_sdk` 生成并经过项目 provenance 校验。

### SDK 结果

| 指标 | Baseline | Visibility only | Final | Final reduction |
| --- | ---: | ---: | ---: | ---: |
| `liblynx.so` | 41,143,432 B | 23,880,272 B | 21,688,440 B | 47.3% |
| SDK ZIP | 16,456,978 B | 10,608,940 B | 9,753,198 B | 40.7% |
| Dynamic definitions | 60,334 | 1,553 | 1,414 | 97.7% |
| ELF `.text` | 23,711,566 B | 16,562,014 B | 14,951,262 B | 36.9% |

Visibility-only 实验说明动态符号表不是唯一收益。恢复 hidden visibility 后，内部
符号不再作为 linker GC roots，`.text`、unwind metadata、PLT/GOT 和 relocation 也同步
缩小。功能裁剪在此基础上继续减少约 `2.19 MB`。

### Runtime 结果

runtime 包含以下项目自带文件，但不捆绑系统 X11、OpenGL、fontconfig、freetype 和
glibc 等动态库：

```text
lynx-launcher
liblynx.so
lynx_core.js
resources/icudtl.dat
resources/lynx_core.js
resources/main.lynx.bundle
```

| Runtime form | Baseline | Final | Reduction |
| --- | ---: | ---: | ---: |
| Current build, unpacked | 49,664,515 B | 30,209,523 B | 39.2% |
| Release-stripped, unpacked | 43,737,115 B | 24,282,123 B | 44.5% |
| Release-stripped, zstd | 16,514,092 B | 9,946,474 B | 39.8% |

`liblynx.so` 本身已 stripped。release-stripped 行额外对当前带 debug info 的
`lynx-launcher` 执行 `strip --strip-unneeded`；zstd 使用默认压缩级别，主要用于稳定的
前后对比，不代表最终发行格式。

## 优化内容

### Hidden-by-default visibility

Linux SDK 原配置为：

```text
disable_visibility_hidden = true
```

这会导出大量 Lynx、Clay、Skia、libc++ 和其他内部 C++ 符号。size profile 将其改为
`false`。公开 Lynx CAPI、value API 和 weak N-API 已带显式 default visibility，现有
host 所需的 63 个 Lynx CAPI 和 14 个 weak N-API 入口仍可链接。

这一项单独把动态定义数从 `60,334` 降到 `1,553`，并贡献了绝大部分体积收益。

### LLVM unwinder isolation

Lynx SDK 静态链接自己的 libc++ ABI 和 LLVM `libunwind.a`，而 launcher 使用系统
libstdc++。只恢复 hidden visibility 会隐藏 libc++ ABI，却仍导出 `_Unwind_*`；动态
装载后，系统 `__cxa_throw` 会错误调用 Lynx 的 unwinder，导致异常路径输出：

```text
terminate called after throwing an instance of 'std::runtime_error'
terminate called recursively
```

Linux shared target因此使用：

```text
-Wl,--exclude-libs=libunwind.a
```

Lynx 内部仍使用配套 unwinder，但 `_Unwind_*` 不再成为进程级动态 ABI。现有
`rust_launcher_rejects_empty_icu` CTest 覆盖该回归：host 必须拒绝空的 ICU 资源并报告
`resource is empty`。

### Feature trimming

最终 profile 关闭：

```text
enable_inspector = false
build_lepus_compile = false
skia_use_wuffs = false
skia_enable_skottie = false
```

影响如下：

- 不提供 Lynx Inspector、LogBox、screenshot service 和相关开发工具。
- 不在运行时把原始 Lepus/template source 编译为 bytecode；launcher 只加载 Rspeedy
  预编译的 `main.lynx.bundle`。
- 不解码 GIF/Wuffs desktop icon；当前 discovery candidates 只覆盖 PNG、SVG 和 XPM。
- 不提供 Skottie/Lottie 渲染；当前 UI 和最终 link closure 均未使用。

## 保留能力

以下能力是当前产品契约，不参与裁剪：

- QuickJS 和 weak N-API；`NativeModules.Launcher` 依赖它们。
- `enable_lepusng_worklet`；当前 Linux GN 会用它决定 N-API 是否启用。
- SVG、Expat；desktop icon E2E 使用真实 SVG。
- SkShaper、SkParagraph、ICU；普通文本、中文 shaping 和 segmentation 依赖它们。
- Skia/fontconfig fallback；缺失 glyph 必须按系统字体配置回退。
- Clay standalone、headless GL 和 windowless embedder。
- PNG/image pipeline、GLFW/X11/OpenGL 3.3 baseline。

## 未采用的选项

- `enable_lto = true`：当前 pinned build 配置在 Linux 上不接入 `-flto`，单独修改该
  GN arg 是 no-op，不能计入优化结果。
- `enable_lepusng_worklet = false`：会在 Linux 上连带关闭 N-API，破坏 Launcher native
  module。
- 关闭 SVG、SkShaper、ICU 或 fontconfig：会直接破坏图标、普通文本或中文 fallback。
- `skia_enable_optimize_size = true`：会裁剪 SIMD、path renderer 和部分渲染策略，视觉与
  性能风险高于本轮目标。
- 只导出 launcher 当前使用的 77 个符号：会让 shipped SDK headers 与实际动态 ABI
  不一致。本轮保留所有显式公开入口。

## Reproduction

构建与 provenance：

```sh
./scripts/bootstrap.sh
eval "$(fnm env --shell bash)"
fnm use 22.14.0
./scripts/build.sh
```

基础测量：

```sh
stat -c '%n %s' host/build/liblynx.so
size -A host/build/liblynx.so
nm -D --defined-only host/build/liblynx.so | wc -l
nm -D host/build/liblynx.so | rg '_Unwind_'
```

最后一个命令必须没有输出。构建产物的 SHA256 为：

```text
2f325f56c3dd71876bed1a906deaec883594ed3954a949a63b543a64cb18216e
```

source key 会随 ordered patch set 内容变化；文中的 key 和 digest 是本次 pinned revision
的测量记录，不应跨 Lynx upgrade 当作固定常量。

## Validation

最终 profile 已通过：

```sh
./scripts/bootstrap.sh
./scripts/test.sh
LYNX_LAUNCHER_SMOKE=1 ./scripts/test.sh
LYNX_LAUNCHER_E2E_ITERATIONS=3 ./scripts/e2e-launch.sh
./scripts/teardown-stress.sh
```

结果：

- Rust discovery `7/7`。
- UI `15/15`，typecheck 和 production bundle build 通过。
- CTest `10/10`，包括 empty ICU exception regression。
- 首帧 GL smoke 通过。
- popup、中文 glyph、SVG icon、搜索和启动 E2E `3/3`。
- teardown bounded `10/10`、first-frame `10/10`。
- teardown 四个 forbidden strings 均未出现。

## Future Work

后续若继续压缩，应一次只改变一个机制并重新跑完整门禁：

1. 为 Linux 增加经过 header 审计的 version script，进一步收窄显式导出。
2. 正式接入 ThinLTO 与 `--icf=safe`；当前 Linux GN 尚无有效 wiring。
3. 单独评估 Skia optimize-size 模式的视觉、CPU 和首帧影响。
4. 在正式发行流程中 strip launcher；当前开发构建保留 debug info。
