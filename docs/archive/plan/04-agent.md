# Agent 规划

## CLI 和配置

目标：

- 提供单二进制 Agent，支持普通运行和 systemd 服务运行。

命令：

- `xlstatus-agent run --config /etc/xlstatus/agent.yaml`
- `xlstatus-agent service install`
- `xlstatus-agent service uninstall`
- `xlstatus-agent service start`
- `xlstatus-agent service stop`
- `xlstatus-agent config validate`
- `xlstatus-agent version`

配置字段：

- `server_url`
- `agent_id`
- `client_secret`
- `tls`
- `insecure_tls`
- `report_interval_seconds`
- `ip_report_interval_seconds`
- `dns_servers`
- `nic_allowlist`
- `disk_mount_allowlist`
- `enable_gpu`
- `enable_temperature`
- `skip_connection_count`
- `skip_process_count`
- `disable_auto_update`
- `disable_force_update`
- `disable_command_execute`
- `disable_nat`
- `disable_send_query`
- `custom_ip_apis`

失败场景：

- `server_url`、`client_secret`、`agent_id` 缺失时启动失败。
- 配置文件权限过宽时输出警告，包含 secret 的文件建议 0600。
- 远程配置保存失败时不切换内存配置。

验收标准：

- 初次安装命令生成配置并注册 systemd。
- 配置 reload 使用原子替换，认证字段不会出现撕裂读。

## 指标采集

目标：

- Linux x86_64 上稳定采集服务器状态。

采集项：

- HostInfo：系统平台、平台版本、CPU 型号、内存总量、磁盘总量、Swap 总量、架构、虚拟化、启动时间、Agent 版本、GPU 名称。
- HostState：CPU 使用率、内存使用、Swap 使用、磁盘使用、总入站/出站流量、入站/出站速度、Uptime、load1/load5/load15、TCP/UDP 连接数、进程数、温度列表、GPU 使用率。

实现建议：

- 优先使用 `sysinfo`、`heim` 替代库或直接读取 `/proc` 和 `/sys`。
- 网络速度通过两次网卡计数差值计算。
- 磁盘按 allowlist mount 聚合。
- 温度从 `/sys/class/thermal` 和 hwmon 读取。
- GPU 第一版支持 NVIDIA SMI 可用场景，其他厂商后续扩展。

失败场景：

- 某采集项失败只记录该项错误，不中断上报。
- 计数器回绕或 Agent 重启时重置速度计算基准。

验收标准：

- 默认 3 秒上报一次状态。
- 单次采集超时不会阻塞下一个周期。

## 任务执行

目标：

- Agent 执行 Dashboard 下发的运维和探测任务。

任务类型：

- HTTP GET：带超时，不跟随无限重定向，返回状态、延迟、证书摘要。
- ICMP Ping：返回平均延迟和成功状态。
- TCP Ping：返回连接延迟。
- Shell Command：使用系统 shell 执行。
- Exec：非交互命令，支持 args、cwd、env、stdin、timeout、max_output。
- ApplyConfig：保存远程配置并延迟 reload。
- ForceUpdate：下载新版本并重启。

失败场景：

- `disable_send_query` 打开时拒绝 HTTP/TCP/ICMP。
- `disable_command_execute` 打开时拒绝 Command、Exec、Terminal、File。
- 命令超时必须杀进程组并回收。
- 输出超过限制时截断并标记。

验收标准：

- 每个 TaskResult 带 task_id、type、success、delay、data。
- 结果发送串行化，避免并发写 gRPC stream。

## Web Terminal

目标：

- 提供浏览器到 Agent 的交互式 shell。

实现：

- Linux 使用 PTY。
- 默认 shell 查找顺序：`zsh`、`fish`、`bash`、`sh`。
- 支持输入、输出、窗口 resize、关闭。

失败场景：

- 无可用 shell 时返回错误。
- Dashboard 或浏览器断开时关闭 PTY 并杀掉子进程。

验收标准：

- 可以在前端打开终端，执行命令并看到输出。
- session 关闭后无残留 shell。

## 文件管理和传输

目标：

- 提供远程目录列表、读取、写入、删除和大文件传输。

能力：

- `fs.list`：目录列表，包含名称、类型、大小、权限、mtime、symlink。
- `fs.read`：支持 offset、length、utf8/base64。
- `fs.write`：支持 utf8/base64、mode、create_dirs、if_match_sha256。
- `fs.delete`：支持递归删除，但拒绝文件系统根目录。
- `fs.transfer`：通过 IO stream 传输最大 100 MiB 文件。

失败场景：

- 路径必须为绝对路径。
- 拒绝根目录删除。
- 拒绝 Windows ADS 路径，Windows 支持放到后续阶段。
- 上传 hash 不匹配时删除临时文件并返回错误。

验收标准：

- 小文件可通过 MCP/REST 工具读写。
- 大文件通过临时 URL 上传下载，不进入 JSON body。

## NAT 隧道

目标：

- Agent 将 Dashboard 的 NAT 请求转发到本机或内网目标。

数据流：

1. Dashboard 根据 Host 匹配 NAT 配置。
2. Dashboard 下发 NAT task，包含 stream_id 和目标 host。
3. Agent 建立本地 TCP 连接。
4. Dashboard 和 Agent 通过 IO stream 双向转发字节。

失败场景：

- `disable_nat` 打开时拒绝。
- 本地连接失败时返回 502。
- 任一端关闭时释放另一端连接。

验收标准：

- 可通过 Dashboard 域名访问 Agent 内网 HTTP 服务。

## 跨平台计划

第一版：

- Linux x86_64 完整支持。

后续：

- Linux arm64。
- Windows 服务、ConPTY、WMI/perf counters。
- macOS launchd、系统指标和温度限制说明。

