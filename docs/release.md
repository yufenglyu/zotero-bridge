# 构建与发布

## 构建

```sh
cargo build --release
# 产物：target/release/zsb（Windows: zsb.exe）
```

发布构建已启用 `lto = true`、`codegen-units = 1`（Cargo.toml release profile）。

## 当前状态

M0–M3（核心）已完成并可从源码构建使用。M4（Tauri 托盘）与 M5（安装
程序、签名、CI）尚未实现，规划见 spec 第 22 节。

## 发布前检查单（M5 草案）

- [ ] `cargo test --workspace` 全绿
- [ ] `cargo clippy --workspace -- -D warnings`
- [ ] Windows 安装程序（含 zsb.exe、默认配置）
- [ ] macOS 签名与公证流程
- [ ] GitHub Actions：fmt / clippy / test / release 构建
- [ ] 数据库迁移升级路径验证（user_version 递增）
