# MAS 沙箱能力矩阵（2026-09-30 实测）

> **这份文档的唯一作用是把「我们以为的」换成「真机跑出来的」。**
> 上一版结论（`mas-sandbox-findings.md`）里有一条推理链被这份实测推翻了，
> 所以旧文档保留但以本文为准。
>
> 数据来源：`macslim --probe-sandbox`。无 GUI、无 Tauri，直接打印 JSON。
> 沙箱由内核按签名里的 entitlement 施加，跟从 Finder 还是终端启动**无关**，
> 所以这样跑出来的路径可达性与 GUI 里完全一致。同一台机器、同一时刻、
> 两个构建各跑一次。

## 0. 三句话结论

1. **交接文档里「0 B 是因为缺 FDA，让用户去授权」这条结论是错的。**
   真正原因是 App Sandbox 把进程的 `$HOME` 重定向到了应用自己的 container，
   而全代码库用 `dirs::home_dir()` 拼用户路径 —— 于是所有 `~/Library/...`
   都解析到了那个空 container。**这是路径解析问题，授权解决不了。**

2. **进程监控完全可用。** 之前为 0 不是沙箱拦的，是 `sysinfo` 枚举进程走
   `proc_listallpids` 被拦、依赖交了白卷。绕开之后沙箱内拿到 200+ 个进程，
   而且 213/214 带完整 `.app` bundle 路径。

3. **文件访问的合规路线实测全通。** security-scoped bookmark 的创建、解析
   在沙箱里都成功，只差用户在文件选择框里选一次目录。

## 1. 怎么复现

```bash
# MAS 包（已签名、开沙箱）
~/.cargo/shared-target-mas/aarch64-apple-darwin/release/bundle/macos/MacSlim.app/Contents/MacOS/macslim --probe-sandbox

# 完整版（非沙箱）—— 同机对照
cargo run --manifest-path src-tauri/Cargo.toml -- --probe-sandbox

# 签名 / profile 的一致性核对
python3 scripts/create_mas_profile.py --show --app <上面那个 MacSlim.app>
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

沙箱是**按路径白名单**放行，不是「全封 + FDA 放行」。

## 4. 进程：之前为 0 是依赖交白卷，不是沙箱拦

| 枚举方式 | 完整版 | MAS | 是否被沙箱拦 |
| --- | --- | --- | --- |
| `sysinfo`（libproc `proc_listallpids`） | 432 | **0** | ❌ 被拦 |
| `sysctl(KERN_PROC_ALL)` | 414 | **379–464** | ✅ 可用 |
| `proc_pidinfo(PROC_PIDTASKALLINFO)` | — | 214–285 条明细 | ✅ 可用 |
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
（进程名/pid/ppid/uid/常驻内存/累计 CPU 纳秒/线程数/启动时间）→
逐个 `proc_pidpath` 取可执行路径。两次采样相减得 CPU 百分比。

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

## 6. 文件访问授权（security-scoped bookmark）—— 实测链路全通

MAS 版读用户目录的**唯一合规路线**：用户在应用内的标准文件选择框里授权
某个目录，应用用 security-scoped bookmark 把它持久化。

竞品调研指向同一条路：PureSpace（App Store 版，$1.99/月）明确写着
「首次用文件选择框授权 `~/Library`，设置里可撤销」；CleanMyMac 的
App Store 版至今仍教用户开 FDA，**却能**清 User Cache —— 多半也是逐目录
授权，不是靠 FDA。

### 沙箱内实测

```
创建书签  → 成功
解析书签  → 成功
startAccessingSecurityScopedResource → 返回 false
```

**机制在沙箱里是通的，缺的只是「还没有任何用户通过文件选择框授权过那个
目录」。** 这是正确行为，不是 bug —— bookmark 背后必须有用户同意，
`startAccessing` 拿不到访问权是正确的拒绝。

所以「让用户授权一次」从「不确定能不能成」变成了「只差一次点击」。

### 两次查错方向的记录（比结论本身更值得留）

**① 误判「profile 缺 entitlement」。**

补上 `files.bookmarks.app-scope` 后 `URLByResolvingBookmarkData` 仍返回 nil，
于是认定「签名里有但 profile 没授权 = 没生效」，去重新签了 profile。
**那个判断是错的：Mac App Store 的 profile 本来就不带沙箱 entitlement**，
它只带 `application-identifier` / `keychain-access-groups` / `team-id`；
MAS 应用的沙箱权限来自**签名**，由 App Store 在处理包时校验。
拿 Developer ID 的心智模型套到 MAS 上，会把整段排查引向错误方向。

核对方式（`scripts/create_mas_profile.py --show --app <path>`）：

```
profile 授权（App Store 下只有这三类是正常的）：
  com.apple.application-identifier = 5XNDF727Y6.com.vgoapp.macslim
  com.apple.developer.team-identifier = 5XNDF727Y6
  keychain-access-groups = ['5XNDF727Y6.*']

签名里的沙箱权限（MacSlim.app）：
  [有] com.apple.security.app-sandbox
  [有] com.apple.security.files.bookmarks.app-scope
  [有] com.apple.security.files.user-selected.read-write
  [有] com.apple.security.network.client
```

**② 把「解析失败」和「拿不到访问权」合成一句话。**

两者在上一层的症状完全一样（`read_dir` EPERM），合成一句「授权失败」
会让人按错误方向查。现在按步报错，并用一个二分诊断区分：同一份书签
**不带** security-scope 选项再解析一次 —— 能解析说明创建掩码写错
（产出的不是 scoped 书签），不能解析说明问题在沙箱层面。
这两种猜错的修法完全相反。

### 实现要点

- `objc2-foundation` 0.3.2 既没导出 NSURL 的书签方法，`src/generated/` 还缺
  `mod.rs`（作为直接依赖编译不过），所以只依赖 `objc2` 运行时、方法自己声明
- `SecurityScope` 持有 `startAccessing`、析构时 `stopAccessing`，配对不多不少
- 解析成功但 `start` 返回 false 时返回 `None`，**绝不**返回一个「看起来成功
  但读什么都 EPERM」的对象
- 授权项只认**真实 home** 下的路径；书签解析结果若逃出真实 home 一律不用
  —— 我们要在那条路径下执行删除
- 授权清单覆盖判定按**路径分量**（`~/Library` 覆盖 `~/Library/Caches`，
  但 `~/.npm` 不会顺带覆盖 `~/.npm-cache`）

### 仍未验证

**`NSOpenPanel` 弹窗 → 用户选目录 → 建书签 → 落盘 → 下次启动解析并读**
这条链的后半段没有任何自动化覆盖：`openPanel` 必须由用户在 GUI 里操作，
探针跑不到那个状态。**这不是「大概能行」，是「还没试过」。**

## 7. 沙箱放行的可用能力（无需任何授权）

这几项实测可读/可用，是 MAS 版「不空」的地基：

| 能力 | 依赖 | 实测 |
| --- | --- | --- |
| 系统健康（CPU/内存/磁盘） | `sysinfo` / `vm_stat` | ✅ 读数可信 |
| 应用列表 + 体积 + 卸载 | 读 `/Applications` | ✅ 40 项，Xcode 8.80 GB |
| 只读进程监控 | `sysctl` + `proc_pidinfo` + `proc_pidpath` | ✅ 214–285 进程，213 个带 bundle 路径 |
| 按 `.app` 聚合运行中的应用 | 同上 | ✅ 依赖 exe 路径，可用 |
| `/Library/Application Support` | 直接列举 | ✅ 22 项（尚未接入扫描） |

## 8. 仍然未知 / 需要人来测

1. **`NSOpenPanel` → 授权 → 下次启动仍有效**（见 §6 末尾）
2. **授权后的缓存扫描与删除** —— 要接进 `cache_scanner` 的删除路径，
   属于破坏性操作，不在授权跑通前动
3. **FDA 到底能不能在沙箱里放行文件访问** —— 无法自动验证（授权要用户在
   系统设置里点）。但 Apple 的文档立场是保守的：App Store 应用即使拿到
   FDA，**沙箱仍然强制执行自己的文件限制**。所以产品决策不该把 FDA 当主路径
4. **PrivacyInfo 的 required-reason 是推断的**（`FileTimestamp / C617.1`、
   `DiskSpace / E174.1`）。ASC 会逐条核对实际使用，猜错直接拒审。
   建议跑通后用 `fs_usage` 确认真的调用了那些 API

## 9. 下一步（按此顺序）

1. **接 `NSOpenPanel` + 授权 UI**（唯一需要人点一次的环节，底层代码已就位）
2. **授权根接进 `cache_scanner` 的删除路径** —— 授权跑通后再动
3. **零授权能力的补齐** —— `/Library/Application Support` 扫描、
   `/Applications` 签名与公证状态校验
4. **MAS 版「应用程序」页恢复** —— 按 `.app` 聚合，依赖 exe 路径（已可用）
5. **复测 + 写上架文案**（**不要**自称「减配版 / Lite」；审核指南 4.0 把
   「功能太少」列为下架原因第一位）
6. **`fs_usage` 核对 PrivacyInfo 的 required-reason**
7. 上传前必须用户确认
