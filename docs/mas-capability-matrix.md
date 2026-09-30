# MAS 沙箱能力矩阵（2026-09-30 实测）

> **这份文档的唯一作用是把「我们以为的」换成「真机跑出来的」。**
> 上一版结论（`mas-sandbox-findings.md`）里有一条推理链被这份实测推翻了，
> 所以旧文档保留但以本文为准。
>
> 数据来源：`macslim --probe-sandbox`。无 GUI、无 Tauri，直接打印 JSON。
> 沙箱由内核按签名里的 entitlement 施加，跟从 Finder 还是终端启动**无关**，
> 所以这样跑出来的路径可达性与 GUI 里完全一致。同一台机器、同一时刻、
> 两个构建各跑一次。

## 0. 一句话结论

**交接文档里「0 B 是因为缺 FDA，让用户去授权」这条结论是错的。**
真正的原因是 App Sandbox 把进程的 `$HOME` 重定向到了应用自己的 container，
而全代码库用 `dirs::home_dir()` 拼用户路径 —— 于是所有 `~/Library/...`
都解析到了那个空 container。**这是路径解析问题，授权解决不了。**

好消息是实测推翻悲观结论的同时也推翻了另外两条：

- 沙箱**不是**「全封」，是**按路径白名单**放行
- 进程监控**完全可用**（之前为 0 是依赖交白卷，不是沙箱拦的）

## 1. 怎么复现

```bash
# MAS 包（已签名、开沙箱）
~/.cargo/shared-target-mas/aarch64-apple-darwin/release/bundle/macos/MacSlim.app/Contents/MacOS/macslim --probe-sandbox

# 完整版（非沙箱）—— 同机对照
cargo run --manifest-path src-tauri/Cargo.toml -- --probe-sandbox
```

## 2. 根因：$HOME 被重定向

| | 完整版 | MAS 版 |
| --- | --- | --- |
| `$HOME` | `/Users/edwinhao` | `/Users/edwinhao/Library/Containers/com.vgoapp.macslim/Data` |
| passwd 里的真实 home | `/Users/edwinhao` | `/Users/edwinhao`（探针用 `nix::unistd::User::from_uid` 取，**不是** `$HOME`） |

代码里受影响的拼接点（都用 `dirs::home_dir()`）：
`cache_scanner.rs:175`、`cache_cleaner.rs:12`、`fda.rs:39`、
`residue_policy.rs:25`、`app_scanner.rs:431`、`uninstaller.rs:311`、
`docker.rs:497`。

`fda.rs` 里那句「用户级探针 `~/Library/Caches` 是否可列举」尤其误导：
沙箱里 `~` 是 container，`Data/Library/Caches` **确实存在且可读**（条目 0），
所以探针会报「有权限」，而真实用户缓存一个字节也读不到。

## 3. 路径可达性逐项实测

| 探针 | 路径 | MAS | 完整版 | 说明 |
| --- | --- | --- | --- | --- |
| `own_container` | `…/Containers/com.vgoapp.macslim/Data` | readable (11) | readable (11) | 对照组，证明探针机制本身没坏 |
| `applications` | `/Applications` | **readable (40)** | readable (40) | 应用列表 / 卸载 / 体积分析 |
| `user_applications` | `~/Applications` | blocked | readable | |
| `user_caches` | `~/Library/Caches` | **blocked** | readable | 缓存清理，0 B 的真凶 |
| `user_logs` | `~/Library/Logs` | blocked | readable | |
| `xcode_derived` | `~/Library/Developer/Xcode` | blocked | readable | 完整版实测 8.80 GB |
| `npm_cache` | `~/.npm` | blocked | readable | |
| `cargo_registry` | `~/.cargo` | blocked | readable | |
| `trash` | `~/.Trash` | readable (0) | readable (0) | 0 是「本来就空」，不是被拦 |
| `downloads` | `~/Downloads` | blocked | readable (1023) | |
| `documents` | `~/Documents` | blocked | readable (68) | |
| `other_containers` | `~/Library/Containers` | blocked | readable (928) | 应用残留清理 |
| `homebrew` | `/usr/local` | blocked | readable | |
| `system_caches` | `/Library/Caches` | blocked | readable | |
| `system_app_support` | `/Library/Application Support` | **readable (22)** | readable (22) | 沙箱放行的第二个系统目录 |

`blocked` 的判定是 `exists() == true` 但 `read_dir()` 失败 —— 沙箱允许你看到
目录存在、却拒绝列举里面的条目，这正是被拦时的表现。**不存在**的目录判为
`absent` 而不是 `blocked`：没装 Xcode 的机器上 `~/Library/Developer/Xcode`
压根不存在，把它算成「被拦」会让引导指向一个无关的权限。

## 4. 进程：之前为 0 是依赖交白卷，不是沙箱拦

| 枚举方式 | 完整版 | MAS | 是否被沙箱拦 |
| --- | --- | --- | --- |
| `sysinfo`（libproc `proc_listallpids`） | 432 | **0** | ❌ 被拦 |
| `sysctl(KERN_PROC_ALL)` | 414 | **379–397** | ✅ 可用 |
| `proc_pidinfo(PROC_PIDTASKALLINFO)` | — | 214–220 条明细 | ✅ 可用 |
| `proc_pidpath` | — | **213/214** | ✅ 可用 |
| `kill(getpid(), SIGCONT)` | true | true | ✅ 可用（对照组，恒真，不能当权限证据） |

**根因**：`sysinfo` 0.33.1 枚举进程走 `proc_listallpids`（见
`src/unix/apple/macos/process.rs:740`），这个调用被 App Sandbox 拦掉。
所以「进程管理 0 项」是**依赖交了白卷**，不是「沙箱读不到进程」。
顺带说明：sysinfo 有个 `apple-sandbox` feature，但那个分支是**返回全 0 的
桩**（`src/unix/apple/app_store/process.rs`，每个方法都 `None`/`0`），
它只保证 MAS 构建能编译。

**绕法**（`src-tauri/src/process_snapshot.rs`）：
`sysctl(KERN_PROC_ALL)` 拿 PID → 逐个 `proc_pidinfo` 取明细
（进程名/pid/ppid/uid/常驻内存/累计 CPU 纳秒/线程数）→ 逐个 `proc_pidpath`
取可执行路径。两次采样相减得 CPU 百分比。

拿到的可执行路径是完整 bundle 路径，实测样例：
`/Applications/OpenCode.app/Contents/Frameworks/OpenCode Helper.app/Contents/MacOS/OpenCode Helper`

**因此 MAS 版可以有**：进程监控（只读）、按 `.app` 聚合的「运行中的应用」。
**仍然不能有**：终止进程 —— 沙箱不允许给别的进程发信号，没有任何 entitlement
能放行。看得到 ≠ 管得动，这两件事不矛盾。

## 5. 交接文档里的两个疑点已结清

| 疑点 | 结论 |
| --- | --- |
| 「缓存清理 0 B，请用户去开 FDA」 | **方向错**。是 `$HOME` 重定向，不是权限。FDA 引导会把用户领去做一件无效的事 |
| 「查『应用程序 0』是否 600 秒缓存问题」（`app_scanner.rs:288`） | **不是**。那一页按 `.app` bundle 聚合进程，与进程枚举同源；600 秒 TTL 只作用于 `/Applications` 的磁盘派生快照，与运行态无关 |

## 6. 仍然未知 / 需要人来测

**FDA 到底能不能在沙箱里放行文件访问，目前无法自动验证** ——
授权动作必须用户在系统设置里点，探针跑不到那个状态。

Apple 的文档立场是保守的：App Store 应用即使拿到 FDA，**沙箱仍然强制
执行自己的文件限制**（"Since your app is sandboxed, the sandbox still
enforces its own file restrictions regardless of Full Disk Access"）。
CleanMyMac 的 App Store 版至今仍在教用户开 FDA，且它的 AS 版**能**清
User Cache —— 这两件事说明它用的多半是 security-scoped bookmark 逐目录授权
（见 `PureSpace` 的做法），而不是 FDA。

**所以产品决策应该是：不把 FDA 当主路径，改走 security-scoped bookmark。**
这条路 Apple 有官方文档、有在 App Store 上活下来的先例、不需要用户去系统设置。
它需要用户在应用内通过标准文件选择框授权一次（可持久化、可撤销）。

## 7. 下一步（按此顺序）

1. **接入只读进程监控与「运行中的应用」** —— 零额外授权，纯代码活
2. **修 `fda.rs` 的语义** —— 现在这个探测会报「有权限」而实际读不到，是错的
3. **security-scoped bookmark 授权层** —— 解锁真实 home 下的缓存/开发缓存/下载
4. **零授权也能做的能力** —— `/Applications` 体积与签名分析、`/Library/Application Support` 扫描
5. 复测 + 写上架文案（**不要**自称「减配版 / Lite」）
6. 上传前必须用户确认
