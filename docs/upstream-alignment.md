# 上游并轨记录

## 基线

- 上游：`24e0303fc55e4e645a11818954dd5b712fb94510`，2026-11-01 执行 `git fetch origin main --tags` 后确认。
- 分支：`codex/upstream-alignment`，直接建立在该上游提交上。
- 旧定制对照：`6a55ed55`，程序源码与 `6eb47b7b` 相同；旧分支保留。
- 原工作目录的未提交修改没有整批搬入，运行设备没有更换。

## 迁移清单

| 能力 | 上游入口与判断 | 本分支处理 |
| --- | --- | --- |
| IPv6 地址、双栈出口节点 | `easytier-core/src/config/toml.rs` 已有 `get_ipv6`、`get_exit_nodes`；出口接受 `IpAddr` | 复用上游，仅补管理 API 的静态 IPv6 地址往返 |
| IPv6 手动路由、显式出口 DNS | 上游 TOML 与管理 API 缺少旧定制的 `routes_v6`、`vpn_dns` | 已迁移配置读写、CLI、管理 API 和前端序列化；实际路由/DNS 安装待迁移 |
| Android 启停与 FD 挂接 | `mobile_vpn.ts`、`instance/factory/mobile_tun.rs` 已有队列、等待重试、共享 TUN 与挂接完成确认 | 采用上游架构，未覆盖旧协调器；分应用策略仍待接入 |
| Android 分应用与共享 UID | 上游插件已有 `disallowedApplications`，完整策略尚未覆盖 | 待迁移策略存储、UID 解析及选择入口 |
| 物理出口与控制 DNS | 上游已有 `socket` 和上下文 DNS，入口与旧版本不同 | 正在核对真实出口、TUN 排除与失败行为，尚未迁入旧 underlay 实现 |
| Windows DNS/NRPT 与生命周期 | 上游已改为共享 NIC/运行时 host | 待迁移，须适配新的资源所有权，不能整文件覆盖旧生命周期 |
| 历史上游回移补丁 | 新上游中相关模块已重组；`tunnel/quic.rs` 已含 ETQ1、packet-number checksum 和旧版本协商 | 不重复迁入 ETQ1 旧补丁；其余逐项核对，保留混合版本验证任务 |
| 私有分流与运维资料 | 独立运维层 | 没有加入公开源码分支 |

## 配置迁移的兼容约定

- 保留旧 TOML/JSON 字段名 `routes_v6`、`virtual_ipv6`、`vpn_dns`。
- 旧定制 protobuf 的 67–69 已被上游用于 `socket_mark`、`peers`、`vpn_portal_config`，不能复用。新字段使用 77–80，原有上游编号保持不变。
- 旧配置通过 TOML/JSON 导入；不承诺旧定制管理客户端与新管理服务之间的二进制 RPC 兼容。节点传输兼容另行验证。
- IPv6 手动模式独立于 IPv4。旧 JSON 的非空列表继续启用 IPv6 手动模式；新增可选 `enable_manual_routes_v6` 使显式空列表也能无损往返。未指定时保留旧行为。
- `vpn_dns` 在配置边界验证为 IPv4/IPv6 地址，不接受域名或带端口字符串。
- 使用上游 protobuf 代码生成流程；不手工维护生成的前端协议类型。

## 已执行验证

- 无定制上游 Windows/Linux 核心 `cargo check --locked -p easytier --bins --features aes-gcm,mimalloc` 通过。Linux 使用本机 WSL。
- 上游移动 VPN 测试 40 项通过；Windows 下测试脚本的通配符不展开，已改为明确的两个文件名。
- 上游 GUI 前端构建通过。需先构建插件 JS，避免使用旧分支留下的声明产物。
- 配置迁移前，两个前端回归均失败，确认旧字段会被新上游序列化丢弃。
- 迁移后，前端全部 57 项测试通过，包括新增的字段往返、独立 IPv6 模式与 protobuf 编解码回归。
- 配置迁移后的 Windows 核心编译检查通过。
- Rust 配置与配置归一化回归 96 项全部通过，含 TOML、两条 API 投影、浏览器合并/清空、无效输入和 IPv4/IPv6 独立模式。
- 迁移后 GUI 前端构建通过。
- 无定制上游 Android ARM64 Rust 编译检查通过；`custom-protocol` 检查复用了已构建前端资产作为 Tauri 嵌入输入，没有构建或签名 APK。
- 迁移后的 Android ARM64 Rust 编译检查通过。
- CLI 新参数回归通过，验证参数解析、覆盖、无参数时保留旧配置和非法输入拒绝。

Windows 首次基线检查因 7-Zip 不在进程 PATH 中失败，补充现有工具路径后通过；Android 首次因桌面 clang 抢先于 NDK clang 而失败，已修正本轮进程 PATH 后重试。这些是构建环境处理，没有改全局设置或上游网络实现。

基线快照与迁移源码切换时，应使用独立 Cargo target 目录，或清除对应本地工作区包的缓存后再检查，避免同名同版本包留下旧元数据。Linux 源码和 target 放在持久缓存目录，不能放在会随 WSL 退出清空的 `/tmp`。

可复现验证入口：

```text
cargo check --locked -p easytier --bins --features aes-gcm,mimalloc
cargo check --locked --lib --target aarch64-linux-android -p easytier-gui --features custom-protocol
cargo test --locked -p easytier-core --lib --features browser-config config::
cargo test --locked -p easytier --lib --features aes-gcm,mimalloc core::tests::custom_dualstack
pnpm --filter easytier-frontend-lib test:config-ui
pnpm --filter easytier-gui test:mobile-vpn
pnpm --filter tauri-plugin-vpnservice-api build
pnpm --filter easytier-gui build
```

使用项目指定 Rust 1.95、protoc 和 Android NDK；NDK clang 必须在目标构建的 PATH 中先于桌面 clang。上述检查不启动 VPN 实例，不更改系统网络。

## 发布状态

当前仅完成并轨基础和配置迁移，尚不能作为现行定制程序的替代产物。下一步是将路由、出口绑定和 DNS 的实际行为接到新架构，再接入 Android 分应用功能，随后构建、跨端验证和分批发布。
