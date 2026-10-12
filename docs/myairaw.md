# MyAIRAW 中文版

基于 ddxmu/rawmakase 的 4d89aaa5acd8b67b01f8aeee72a35f3e830a89be，保留原项目 MIT 及第三方许可证。

- 简体中文界面与嵌入的 Noto Sans SC 字体，默认显示中文。
- MyAIRAW 应用名称、独立 macOS bundle ID 及橘红到黑色的 RAW 图标。
- macOS 数据目录为 `~/Library/Application Support/MyAIRAW`。
- 图库与 RAW 编辑格式、参数 ID、控制协议和影像处理算法保持原格式。
- 更新来源为 https://github.com/ddxmu/MyAIRAW/releases，默认不自动检查。
- 0.2.3 新增 JPEG、PNG、TIFF 编辑、裁剪、调色和导出；编辑存入图库，禁止导出覆盖原图。
- 0.2.4 新增 AI 助手：可配置 OpenAI 兼容 API、拉取并选择模型，按提示词生成可撤销的显影参数调整。
- 普通图片按嵌入 ICC 转换为 sRGB，保留 EXIF 方向及 16 位输入精度；PNG 透明部分在 RGB 编辑器中合成白色，源文件不变。
- 更新器优先下载与当前版本匹配的文件级增量 ZIP，不重复下载未变化的应用文件和动态库。校验 Ed25519 发布签名、包 SHA256、每个文件哈希和应用签名；失败回退到签名完整包。重启安装仍使用原更新器的备份、启动确认与回滚流程。
- 0.2.2 未接入该仓库，需手动安装一次 0.2.3；之后可通过偏好设置里的更新升级到 0.2.4 及后续版本。
- 本地发布私钥只保存在被 Git 忽略的 `.release-private/`，绝不发布到仓库。
- `assets/zh-CN.json` 记录文案翻译；`src/app/chinese.rs` 只翻译模型返回的显示名称。
- 本地 DMG 使用 ad-hoc 签名，未使用 Apple Developer ID 公证。
- 此本机构建为 Apple Silicon arm64，内置原生组件的最低系统版本为 macOS 27.0；不声明兼容 Intel 或更早系统。

macOS打包采用 `packaging/macos/bundle.py` 将动态库封装至应用内，并使用 `packaging/macos/dmg.rb` 生成磁盘映像。

## MyAIRAW 本地发布

此分支不自动运行原版的跨平台公证发布流程。先运行仓库规定的 fmt、clippy、完整工作区测试和架构检查。
编译时使用 `--no-default-features`，不启用遥测。打包参数为最低 macOS 27.0，验证 app 签名、DMG 和实际打开/编辑/导出。

发布完整包必须命名 `myairaw-vVERSION-macos-arm64.dmg`。增量包命名 `myairaw-vVERSION-macos-arm64-from-OLD.delta.zip`：

```
python3 packaging/macos/myairaw-delta.py OLD.app NEW.app OUTPUT.delta.zip
python3 packaging/myairaw-sign.py .release-private/update-private.pem RELEASE_DIR ASSET.dmg ASSET.delta.zip
```

将完整包、可用增量包、`checksums.txt`、`checksums.txt.sig` 上传同一 `vVERSION` Release。用上一次真正发布的 app 作基准，不重签基准。
同一版本不得换包；增量只发布下一版本。签名私钥须安全保留，丢失后旧安装无法信任新的发布签名。
