# 构建、签名与发布（M5）

## 本地构建

```sh
cd apps/desktop
npm ci
npx tauri build
# Windows:
#   target/release/bundle/nsis/Zotero Bridge_<ver>_x64-setup.exe
#   target/release/bundle/msi/Zotero Bridge_<ver>_x64_en-US.msi
# macOS:
#   target/release/bundle/dmg/*.dmg
```

Windows 本地发布产物：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\release.ps1
```

macOS 本地发布产物：

```sh
bash scripts/release-macos.sh
```

NSIS 安装器按当前用户安装（`installMode: currentUser`），无需管理员
权限；WebView2 采用 `embedBootstrapper` 随包分发。

## CI / 发布流水线

- `.github/workflows/ci.yml`：push/PR 时在 Windows 与 macOS 上执行
  `cargo fmt --check`、`cargo clippy -D warnings`、`cargo test --workspace`，
  并单独构建前端。
- `.github/workflows/release.yml`：推送 `v*` 标签时构建 Windows zip、
  Windows 桌面安装包（NSIS/MSI）和 macOS 通用应用（DMG/app zip），
  并按 `CHANGELOG.md` 中对应版本章节发布正式 GitHub Release。
  Windows zip 由 `scripts/release.ps1` 生成，macOS 产物由
  `scripts/release-macos.sh` 生成，最终只上传 `target/dist/` 下的成品。

## Windows 签名

安装包未签名时 SmartScreen 会提示“未知发布者”。两种方案：

1. **signtool + 代码签名证书（PFX）**：在 release.yml 的 `npx tauri build`
   之后追加步骤：
   ```powershell
   signtool sign /fd sha256 /tr http://timestamp.digicert.com /td sha256 `
     /f "$env:CERT_PFX" /p "$env:CERT_PASSWORD" `
     "target/release/zotero-bridge.exe" "target/release/bundle/nsis/*-setup.exe" "target/release/bundle/msi/*.msi"
   ```
   PFX 以 base64 存入仓库 secret，运行时解码到临时文件。
2. **Azure Trusted Signing**：Tauri 2 支持通过
   `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
   环境变量在打包时签名。

## macOS 签名与公证

在仓库 secrets 配置（release.yml 已引用）：

| Secret | 内容 |
| --- | --- |
| `APPLE_CERTIFICATE` | Developer ID Application 证书（base64 编码的 .p12） |
| `APPLE_CERTIFICATE_PASSWORD` | .p12 密码 |
| `APPLE_SIGNING_IDENTITY` | 如 `Developer ID Application: Name (TEAMID)` |
| `APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID` | 公证用 App 专用密码与团队 ID |

配置齐全后 `npx tauri build` 会自动签名并提交公证（notarytool）。

## 版本迁移机制

- 数据库：`PRAGMA user_version` 递增迁移（`crates/index/src/migrations.rs`）。
  新增迁移文件放入 `migrations/NNNN_name.sql` 并登记到 `MIGRATIONS`
  数组即可；旧版本启动时自动按序升级。
- 配置：`serde(default)` 保证旧版 config.toml 缺少新字段时回落默认值；
  保存采用临时文件 + 原子替换。
- 升级安装：NSIS 覆盖安装保留 `%APPDATA%` / `%LOCALAPPDATA%` 下的配置、
  索引与镜像，升级后首次启动自动执行增量同步补齐。

## 发布检查单

- [x] `cargo test --workspace` 全绿
- [x] `cargo clippy --workspace --all-targets -D warnings` 无警告
- [x] Windows NSIS/MSI 安装包可构建
- [ ] 安装包在干净 Windows 机器上完成安装→首次同步→搜索验收
- [ ] macOS 签名 + 公证通过（`stapler validate`）
- [ ] Windows 签名后 SmartScreen 无警告
- [x] 发布说明与升级指南（docs/release.md、README、CHANGELOG.md）
