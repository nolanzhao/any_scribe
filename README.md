# AnyScribe

音视频转录文本工具。基于 [whisper.cpp](https://github.com/ggerganov/whisper.cpp) + [Tauri](https://tauri.app)，支持中英文语音识别，所有数据完全在本地处理。

> 💡 **建议：使用时建议手动选择语言，转录结果更准确。**

## 功能

- 支持本地视频文件 — MP4 / MKV / AVI / MOV 等常见格式
- 支持在线平台 — 抖音 / 小红书 / 小宇宙
- 支持中文、英文及自动语言检测
- 实时转录 — 边转录边显示文本，边写入文件
- 自动保存 SRT 字幕 + TXT 文本
- 转录完成后一键打开文件或所在文件夹
- 支持浅色 / 深色主题切换
- 完全离线转录，无需上传数据



## 安装

### 下载安装包

前往 [Releases](https://github.com/nolanzhao/any_scribe/releases) 下载最新版本的 `.dmg` 文件（仅支持 Apple Silicon Mac）。

> **遇到了"应用已损坏，无法打开"？**
> 这是因为 macOS 的 Gatekeeper 机制拦截了未经过 Apple 开发者签名的第三方开源应用。
> **解决方法：**
> 打开终端 (Terminal)，执行以下命令：
>    ```bash
>    sudo xattr -r -d com.apple.quarantine /Applications/AnyScribe.app
>    ```

### 系统要求

- macOS 14.0+（Apple Silicon M3 / M4）

### 环境依赖 & 模型

**开箱即用，无需手动配置：**
首次启动时应用会自动依次下载所需的环境依赖和模型：

| 组件 | 大小 | 用途 |
|------|------|------|
| FFmpeg | ~30 MB | 音视频格式转换 |
| Whisper Large V3 Turbo | ~1.5 GB | 语音识别模型 |

下载后存储在 `~/Library/Application Support/AnyScribe/`，之后无需重复下载。

## 从源码构建

```bash
# 克隆仓库
git clone https://github.com/nolanzhao/any_scribe.git
cd any_scribe

# 安装前端依赖
npm install

# 开发模式
npx @tauri-apps/cli dev

# 打包
npx @tauri-apps/cli build
```

构建产物位于 `src-tauri/target/release/bundle/dmg/`

## 技术栈

- **后端**: Rust + [Tauri 2](https://tauri.app) + [whisper-rs](https://github.com/tazz4843/whisper-rs)
- **前端**: Vanilla JS + CSS
- **推理**: whisper.cpp（Apple Accelerate 加速）

## License

[MIT](LICENSE)
