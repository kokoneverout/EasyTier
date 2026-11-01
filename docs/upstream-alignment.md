# 上游并轨记录

## 基线与源码

- 上游基线：24e0303fc55e4e645a11818954dd5b712fb94510，2026-11-01 重新获取 main 后固定。
- 分支：codex/upstream-alignment，直接从该上游提交建立。
- 旧对照：6a55ed55，程序源码与 6eb47b7b 相同，旧分支保留。
- 原工作目录中的私有配置、运维记录和未提交源码没有整体搬入。生产设备尚未切换。

## 定制清单

| 能力 | 采用的上游入口 | 保留内容及删除条件 |
| --- | --- | --- |
| 双栈地址、出口选择与转发 | easytier-core 配置、peer manager | 复用上游，只补管理 API 静态 IPv6 往返；上游覆盖同一字段后可删 |
| IPv6 手动路由、显式 VPN DNS | TOML、CLI、管理 API、GUI、runtime services | 补字段与转换，IPv6 模式独立于 IPv4并保留显式空列表；等价语义进入上游后可删 |
| 路由安装与维护 | CorePacketPlane、NicCtx、共享 NIC claims | 接入手动 IPv6 路由、手动 cost、失败重试和消失路由恢复；两类 NIC 均获上游覆盖后可删 |
| 物理出口 | NativeInstanceHost、原有 TCP/UDP factory | 实例级适配，排除 TUN，覆盖 STUN/打洞控制 socket，源不可用则报错；上游支持同等实例策略后可删 |
| 控制 DNS | request-scoped resolver | 查询时读当前物理 DNS、过滤 VPN DNS，优先物理网络服务器；复用 UDP/TCP 与超时，上游覆盖后可删 |
| Windows VPN DNS/NRPT | WindowsDNSManager、NicCtx 生命周期 | 单一所有者、失败与停止清理、缓存刷新；显式 DNS 与 Magic DNS 互斥；上游支持等价所有权后可删 |
| Android 分应用 | 上游插件、VpnService、共享 TUN | 包名存储、UID/共享 UID、自身排除、卸载处理、选择页；上游产品功能覆盖后可删 |
| Android 启停 | mobile_vpn.ts 单一队列 | 接入策略、请求 ID、原生错误与按 UID 重建；失败恢复一次，用户停止优先；上游协调器覆盖后可删 |
| 签名与产物 | 上游工具链、Gradle、mobile workflow | 外部文件/secrets 签名入口、证书和产物哈希核验；不在源码中保存签名材料 |

没有搬回旧 virtual_nic、manual connector、打洞实现、全局 underlay resolver 或整套移动协调器。ETQ1、packet-number checksum、旧版本协商及连接恢复直接采用新上游。旧定制中强制所有控制 DNS 走 TCP、固定 700ms/一次尝试的策略不再保留。

应用选择页面与 UID 策略仍是需要的功能。本轮保留现有页面，没有把页面拆分、图标队列重构或私有 YAML 模板改造作为迁移前置条件。Mihomo 分流和私有部署层继续独立维护。

## 兼容约定

- 保留 routes_v6、virtual_ipv6、vpn_dns 的 TOML/JSON 名称。
- 旧定制 protobuf 的 67–69 与新上游 socket_mark、peers、vpn_portal_config 冲突，新增字段采用 77–80。旧配置通过 TOML/JSON 导入；旧管理二进制不能直接操作新服务。
- IPv6 手动模式独立于 IPv4。旧 JSON 非空列表仍推断为启用；enable_manual_routes_v6 可保留显式空列表。
- DNS 在配置边界验证为 IPv4/IPv6 地址。前端协议类型按上游流程生成。
- Android 保留 com.kkrainbow.easytier、原有策略 SharedPreferences 名称与 schema 1。首次启动就完成策略初始化，旧安装无策略时保留全应用行为，新安装使用空选择，已有策略不覆盖。
- Android ARM64 APK 的证书 SHA256 与旧安装包一致：589080507c25cc5f136f489ada340bce870df8c97db73c0691ec2a1f2eb9fecc。

## 提交

- 06038d79：Windows 下移动测试命令。
- 82aa6598：配置、CLI、管理 API 与序列化。
- 9d45bd7b：host/socket、双栈路由、DNS 生命周期适配。
- ed50d99c：分应用接入上游生命周期与配置 UI。
- afcdc742：签名材料移出源码。
- 40353f87：可重复的隔离网络回归与上游 CI 签名适配。
- 22690d25：安装/升级策略初始化顺序及回归。
- 3b05ec6f：显式源地址的 STUN/打洞 socket 也绑定物理设备，增加隔离行为回归。
- 9f40a381：核对实际策略后才向界面报告应用成功。

GitHub Actions 仅保留 custom-build.yml：前端验证、Windows/Linux 核心及 Android ARM64 签名 APK。删除另外 13 套继承工作流、Docker 构建文件和不再使用的 pnpm/WASM 配置；仅保留 Rust 与前端两个准备动作。仓库里原来启用的 8 个继承工作流已禁用，避免默认分支继续额外触发。

用户已重新启用 Actions，并明确要求停止在其电脑上构建。本机编译进程已停止，后续测试与发布构建全部交给 GitHub Actions。

## 验证

已通过：配置与归一化 Rust 96 项、CLI 1 项、前端 57 项、移动协调器/策略初始化 51 项、Android 原生策略 11 项。核心运行时首轮选定测试 11 项通过；最终核心回归随发布构建复核。

隔离 WSL 中，新旧发布程序的 TCP、QUIC 双向连接通过。独立网络/挂载命名空间内，双栈 TUN 数据、默认路由接管后的物理传输、IPv4/IPv6 路由补回、STUN、UDP DNS 和 DNS TCP 回退均通过。未接入生产网络，未改变宿主路由或 DNS。

复验入口：

    cargo test --locked -p easytier-core --lib --features browser-config config::
    cargo test --locked -p easytier --lib --features aes-gcm,mimalloc core::tests::custom_dualstack
    pnpm --filter easytier-frontend-lib test:config-ui
    pnpm --filter easytier-gui test:mobile-vpn
    pnpm --filter tauri-plugin-vpnservice-api build
    pnpm --filter easytier-gui build

Android 工程生成后，在 gen/android 下运行：

    ./gradlew :tauri-plugin-vpnservice:testDebugUnitTest --no-daemon

Linux，两个目录均需包含 core 与 cli：

    sudo bash scripts/upstream-alignment/network-smoke.sh OLD_BIN_DIR NEW_BIN_DIR OUTPUT_DIR

采用 Rust 1.95、protoc 35.1、Android NDK 26。NDK clang 在 Android 构建 PATH 中先于桌面 clang。三端使用独立的云端 runner，产物记录提交、构建选项和 SHA256。Android 使用已通过测试的前端资产及原有仓库签名 secrets，证书不匹配则拒绝交付。

## 发布状态

源码迁移已完成，功能源码冻结于 9f40a381。Windows 首轮本地发布构建成功；首个本地签名 ARM64 APK 已生成并核对证书。上述历史产物不作为本轮最终升级包，最终交付采用同一次云端流程生成的三端产物。当前没有将候选程序替换到生产设备。

本地检查不等同于 Windows 实际 NRPT 切换或 Android 实机后台/升级验收。真实切换保留现行程序与配置，先准备独立五分钟自动回退，再逐设备验证；当前 Windows 连接放在最后。
