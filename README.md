# Copy & Paste

一款专为 macOS 打造的轻量化开源剪贴板工具

可设置开机静默启动，按自定义快捷键唤出窗口，查找复制过的文字、图片等内容，点选即可粘贴到正在使用的应用。

![Copy & Paste — 专为 macOS 打造的剪贴板工具](docs/images/copy-and-paste-github-poster.png)

## 下载与首次打开

前往本仓库的 [Releases 页面](https://github.com/ChongWang0407/copy-and-paste/releases)，下载 `Copy-and-Paste_1.0.dmg`；打开 DMG，将 `Copy & Paste.app` 拖入“应用程序”文件夹，再从“应用程序”启动。

当前版本尚未经过 Apple 公证（详见 [Apple 官方说明](https://support.apple.com/zh-cn/102445)），若 macOS 阻止打开，请先点击“完成”以关闭弹窗，再按下图操作允许 App 运行。
![下载后五步打开：拖入应用程序、点击完成、在设置中找到应用、再次确认打开、指纹或密码验证](docs/images/00-install-guide.png)

## 功能

- **自动记录与快速粘贴**：保存文字、富文本、代码、网址、图片，以及从访达复制的视频和文件。默认 `⇧Z` 唤出面板，点击记录即可粘贴。
- **搜索与分类**：按内容、来源应用或图片中识别出的文字搜索；按类型筛选。
- **图片文字识别**：使用 macOS 的 Apple Vision 在本机识别图片中的中英文文字，并可将识别结果粘贴为文字。
- **批量粘贴**：勾选多条记录，在预览中调整顺序与连接格式后粘贴。
- **复制提示**：玻璃胶囊、黑色胶囊、黑色刘海三种样式，可调整停留时间或关闭。
- **常用内容与编辑**：保存常用语和常用图；编辑文字记录，或选择以原格式、纯文本、Markdown 粘贴。
- **系统翻译**：macOS 26 及以上可用 Apple 的本机翻译框架处理中英互译；首次使用可能需要下载系统语言包。

## 系统要求与权限

| 项目 | 要求 |
| --- | --- |
| 系统 | macOS 14 或更新版本 |
| 芯片 | 当前安装包仅提供 Apple 芯片版；Intel Mac 尚未验证 |
| 外观 | macOS 26 使用系统液态玻璃；较早版本使用毛玻璃 |
| 翻译 | 需要 macOS 26 或更新版本 |
| 辅助功能权限 | 用于向其他应用发送粘贴快捷键；首次启动时按应用提示授权 |

## 数据与隐私

剪贴板记录默认保存在 `~/Library/Application Support/com.copypaste/`。应用没有账号、云同步或使用情况统计。文字识别和翻译在本机完成；复制网页富文本时，应用可能下载其中的网络图片以保留内容。系统翻译语言包由 macOS 下载和管理。可以在设置中关闭持久化存储，使记录仅保留在当前会话中。

## 从源码构建

需要 macOS、Node.js、Rust、Xcode，以及构建 Swift 桥接代码所需的 macOS 26 SDK。

```bash
npm ci
npm run tauri:dev     # 开发版，与正式版使用不同的数据目录
npm test              # 前端测试
npm run test:rust     # Rust 测试
npm run lint
npm run tauri:build   # 生成 .app 与 .dmg
```

当前配置使用 ad hoc 签名。若希望用户无需手动放行，可改用 Developer ID 签名并提交 Apple 公证。构建产物位于 `src-tauri/target/release/bundle/`。

## 开源许可与致谢

本项目基于 [TieZ](https://github.com/jimuzhe/tiez-clipboard) 修改，沿用 [GPL-3.0](https://github.com/ChongWang0407/copy-and-paste/blob/main/LICENSE)，液态玻璃窗口与 OCR 的实现参考了 [Maccy](https://github.com/p0deje/Maccy)（MIT）。

项目在 TieZ 基础上重写了原生 Liquid Glass 界面，移除了云同步、AI 等功能，增加了批量粘贴、复制弹窗、离线翻译与 OCR 等实用功能。
