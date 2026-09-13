# 运行日志与隐私保护

运行日志用于定位操作失败和等待阶段，不记录终端输入输出、聊天正文、文件内容、认证凭据或完整 URL。

## 文件与配置

Rust 桌面端、Electron 后端和无界面服务默认写入各自应用数据目录下的 `logs/runtime-<进程号>-<启动标识>.log`。远程窗口的业务请求在远端后端记录；SSH 建连过程在本地桌面端记录。分屏事件沿用原路由：原生窗口写本地宿主，普通浏览器写连接的后端。

`VLX_LOG_DIR` 覆盖通用目录；没有设置时，旧的 `VLX_SPLIT_LOG_DIR`、`VLX_MEMORY_LOG_DIR`、`VLX_KNOWLEDGE_LOG_DIR` 和 `VLX_SECURITY_LOG_DIR` 仍可分别指定对应事件的目录。目录中统一使用 `runtime-*.log` 文件名，旧文件不会自动迁移或删除。环境变量在进程启动时读取，修改后需要重启相应进程。

`VLX_LOG_LEVEL` 默认 `INFO`，支持 `TRACE`、`DEBUG`、`INFO`、`WARN`、`ERROR`、`OFF`，不区分大小写。原专项级别变量继续作为额外过滤条件；专项配置不能绕过通用级别限制。排查排队问题时可设置 `DEBUG`，此时包含 RPC 的 `queueMs`。

每个 Rust 日志文件约 10 MiB，单个写入器最多保留当前文件和 4 个轮转文件。写入时定期清理本程序命名的旧运行日志，按 7 天和目录内运行日志约 100 MiB 的总量控制；清理不是实时硬限额，不处理会话录制和其他业务文件。Unix 日志目录权限为 `0700`、文件为 `0600`，拒绝当前目标是符号链接的情况。Windows 使用运行账户和目录继承的 ACL；不自动修改系统 ACL。

Electron 外壳启动失败等本地事件另外写入其 userData 下的 `logs/electron-*.log`，或使用 `VLX_LOG_DIR`；同样按单文件约 10 MiB、最多 5 个文件、7 天和约 100 MiB 清理。外壳不再原样转发子进程 stdout/stderr，后端日志应从后端自己的运行日志读取。

Android 和 iOS 的原生 SSH 连接插件使用平台日志（Android 的 `VelaTerm` 日志标签、iOS 系统日志），只记录固定连接阶段、匿名操作 ID 和耗时。`previousStep` 表示刚结束的阶段，`durationMs` 是该阶段经过的时间；原生日志不自动复制到桌面文件或上传。

关联的 Kotlin 服务默认写入 `logs/server.log`，可用 `VLX_LOG_DIR` 和 `VLX_LOG_LEVEL` 配置位置与应用包级别；轮转按 10 MB、7 天和 100 MB 配置。该服务与桌面端是独立进程。

## 如何定位终端切换慢

先找到 `client_shell_switch`，根据 `operationId` 和 `sessionId` 关联后续事件：

1. `rpc`、`rpc_queue`、`client_request`：配置更新、结束旧进程等请求的执行、排队和客户端总耗时。
2. `pty_kill`：会话锁、终止进程、释放旧资源的阶段；`pty_kill_failed` 单独记录终止调用失败。
3. `client_restart`、`client_pty_spawn`：客户端触发重新挂载和请求新终端。
4. `pty_prepare`、`pty_spawn`：会话准备、启动槽位、PTY 分配、Shell 配置、进程启动。
5. `pty_first_output`、`client_pty_output`：后端首段输出和客户端收到首段输出。

每个阶段有开始和完成记录，包含 `durationMs` 和操作累计的 `elapsedMs`。首段输出只说明收到了字节，不保证登录脚本已经运行完，也不保证浏览器完成了绘制。没有可靠的 Shell 就绪协议时，不推断提示符就绪。

SSH 建连使用 `ssh_connect` 记录 connect、probe、supply、upload、start、forward 等阶段。普通业务请求统一由 `rpc` 记录开始、成功或失败；键盘输入和 resize 不逐次记录。诊断事件不改变请求重试、取消或进程管理规则。

## 记录内容与故障边界

控制台与文件使用相同的 `yyyy-MM-dd HH:mm:ss [LEVEL] [requestId或system] event=...` 格式。受控字段保留在行尾 JSON 中；代码位置和静态警告模板只由编译后的代码提供，模板中的动态异常、路径和对象不会被展开。

字段白名单保留内部 UUID、固定状态和方法、数量、大小、耗时、安全错误分类及经过筛选的 AI 使用量。显式模型名不直接输出，使用 `model=redacted` 和当前进程内的匿名 `modelRef` 关联；`configured_default` 表示请求沿用配置，并不表示已观测到提供方实际选择的模型。受控 AI 审计保留内容大小和 SHA-256，正文 preview 使用固定隐藏提示。摘要仍属于受限诊断信息，不是匿名化证明。

前端错误缓冲和控制台只保留固定错误分类。业务界面的错误处理仍收到原始异常，便于用户处理具体问题；该内容不自动复制进诊断日志。前端诊断请求有并发上限，服务端按连接限频且拒绝任意载荷，失败不递归上报、不自动重发。

Rust 写入队列最多 2048 条，写入在后台线程执行。Rust panic 只记录固定事件和代码行号，不输出 panic 载荷；无法拦截操作系统强制终止。队列满会丢弃事件并增加计数；磁盘错误限频输出固定警告。可信本地调用可通过 `diagnostic_health` 查看 `initialized`、`droppedCount`、`writeFailures`，远程客户端不能调用此查询。接受进入队列不等于已经写入磁盘，强制退出、磁盘故障和积压都可能造成缺口。

HTTP 运行日志记录路由分类、方法、状态和响应准备耗时，不记录查询参数、headers、body 或完整动态路径。文件下载、WebSocket 等长连接的 HTTP 日志不代表流式传输已经结束。Kotlin HTTP 日志使用框架匹配的路由模板，未知路由记录 `unmatched`。

日志默认仅保存本地。会话录制、聊天历史、数据库、知识库来源快照、安全报告和 CLI 正常结果输出仍是各自的业务数据，不归入运行日志。第三方工具自身的日志不受此写入器管理，不应直接打包或上传。
