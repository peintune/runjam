# SignPath Foundation 代码签名接入指南

给 Windows 安装包（`RunJam_*_x64-setup.exe`）加上 Authenticode 签名，消除
SmartScreen 拦截和"发布者: 未知"。SignPath Foundation 对开源项目**免费**。

前提：项目是 MIT 许可 + GitHub 公开仓库（RunJam 满足）。

---

## 一、申请

1. 打开 <https://signpath.org/apply>
2. 填写项目信息（仓库地址、项目名称、许可类型、项目主页）
3. 等待审核（人工审核，通常数日到数周）
4. 通过后你会拿到一个 SignPath 组织（Organization），组织内已预置好证书

审核重点是"这个仓库确实是活跃的开源项目"，不需要个人身份验证 —— 这正是
SignPath 相对自购证书最省事的地方：**证书由 SignPath Foundation 签发，它以
自己的名义为"该二进制确实构建自你的开源仓库"这件事背书**，所以你不需要去 CA
做身份核验，也不需要 USB token。

## 二、SignPath 侧配置

登录 <https://app.signpath.io>，按顺序完成：

### 1. 项目（Project）

创建 Project，记下 **Project slug**（例如 `runjam`）。

### 2. 签名策略（Signing Policy）

在项目下创建 Signing Policy，记下 **Signing policy slug**（例如 `release-signing`）。

- 证书选择 SignPath Foundation 提供的那个（Open Source Code Signing）
- 是否要求审批：个人项目建议设为自动批准（否则每次构建都要去页面点一下）

### 3. 制品配置（Artifact Configuration）

这是最容易配错的一步。**推荐做法：上传样本自动生成** ——
在 Artifact Configurations 点 `Add` → `Upload an artifact sample`，上传一个本地
构建出的 `RunJam_*_x64-setup.exe`，SignPath 会自动分析并生成配置，然后删掉里面
你不打算签名的第三方组件。

如果要手写 XML，用这份（对应 workflow 里 `upload-artifact` 的 ZIP 包装）：

```xml
<artifact-configuration xmlns="http://signpath.io/artifact-configuration/v1">
  <zip-file>
    <pe-file path="**/RunJam_*_x64-setup.exe">
      <authenticode-sign />
    </pe-file>
  </zip-file>
</artifact-configuration>
```

要点：

- 根元素**必须是 `<zip-file>`**：`actions/upload-artifact` 默认会把文件打成 ZIP
  （SignPath 文档明确要求根元素与之匹配）。
- `**/` 前缀用来兼容 ZIP 内是否带目录层级，避免路径假设导致匹配失败。
- 记下 slug（例如 `runjam-setup`）。若把它设为该项目的默认制品配置，则
  `SIGNPATH_ARTIFACT_CONFIGURATION_SLUG` 可以不配置。

### 4. CI 用户与 API Token

创建一个专用于 CI 的用户（或"CI 用户"类型），在目标 Project + Signing Policy
上授予 **submitter** 权限，然后为该用户生成 API token。

### 5. （可选）安装 SignPath GitHub App

安装 <https://github.com/apps/signpath> 并授权本仓库，可用上更高级的源码与构建
策略校验（分支保护、CodeQL 扫描结果等）。基础签名流程不装也能跑。

## 三、GitHub 侧配置

在本仓库 **Settings → Secrets and variables → Actions** 添加：

| 位置 | 名称 | 值 |
|---|---|---|
| Secrets | `SIGNPATH_API_TOKEN` | 上一步生成的 API token |
| Variables | `SIGNPATH_ORGANIZATION_ID` | 组织 ID（UUID，在 SignPath 组织设置页） |
| Variables | `SIGNPATH_PROJECT_SLUG` | 例如 `runjam` |
| Variables | `SIGNPATH_SIGNING_POLICY_SLUG` | 例如 `release-signing` |
| Variables | `SIGNPATH_ARTIFACT_CONFIGURATION_SLUG` | 例如 `runjam-setup`（若为默认配置可留空） |

用 Variables 而非 Secrets，是因为后四者不是敏感信息，且便于在日志中排查。

`SIGNPATH_API_TOKEN` 为空时，workflow 会**跳过全部签名步骤**并照常发布未签名包，
所以可以安全地先合入、后配置。

## 四、构建流程说明

`build.yml` 中 `build-windows` job 的顺序是：

```
tauri build                → 生成 exe + .sig（此 .sig 基于未签名 exe，后面作废）
  ↓
stage + upload-artifact    → 未签名 exe 作为临时 artifact 上传（名字刻意不含 runjam 前缀）
  ↓
SignPath 签名              → 云端 HSM 完成 Authenticode 签名，下载回 signed-installer/
  ↓
覆盖原 exe + 重签 .sig     → 关键步骤
  ↓
upload-artifact            → runjam-windows-x64（进 Release / OSS）
```

**为什么必须重签 `.sig`**：Tauri updater 用 minisign 校验下载到的 exe 字节。
Authenticode 签名会改变文件字节，不重签的话用户自动更新会直接校验失败。
这一步已内置在 workflow 里，无需手动干预。

## 五、验证

1. 打一个 tag 触发构建，看 `build-windows` job 日志里是否出现
   `✅ 已替换为 SignPath 签名版` 和 `✅ updater 签名已重新生成`。
2. 下载 Release 里的 exe，右键 → 属性 → **数字签名** 页签，应能看到签名。
3. 用 `signtool verify /pa RunJam_*_x64-setup.exe` 复核。

## 六、预期效果与注意事项

- **签名 ≠ 立刻没有警告。** 2024-03 起微软取消了 EV 证书的 SmartScreen 特权，
  OV/EV 现在一律靠下载量积累信誉。新签名包初期仍可能出现"仍要运行"提示，
  随着下载量上升（通常几百到几千次、数周）会自然消失。这是正常的，不是配置失败。
- **不要为此去买 EV 证书。** 它已经没有 SmartScreen 上的优势，只剩内核驱动签名
  和更强的法律身份验证两个用途，RunJam 都用不到。
- **OSS 项目硬性要求**：提交签名请求的 workflow，其所有 job 必须跑在
  GitHub-hosted runner 上。`build.yml` 全部使用 `windows-latest` / `macos-*` /
  `ubuntu-latest`，满足要求；若将来改成 self-hosted runner，SignPath 会拒绝签名。
- 证书有效期自 2026-03-01 起被 CA/B 论坛限制为最长 458 天，但 SignPath 托管证书
  的续期由它自动处理，你无需关心。
