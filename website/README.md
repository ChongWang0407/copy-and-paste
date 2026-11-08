# Copy & Paste 官网

```bash
npm install
npm run dev      # 本地预览 http://localhost:5173
npm run build    # 产物在 dist/，纯静态文件
```

## 以后要替换的地方（都在 `src/config.js`）

- `GITHUB_URL`：GitHub 仓库地址（现在是占位）
- `DOWNLOAD_URL`：Download 按钮地址，默认是仓库的 Releases/latest
- `VIDEO_SRC`：把宣传视频 MP4 放进 `public/`，填 `'./文件名.mp4'`
- `VIDEO_POSTER`：视频封面图（可选）

## 调试

- `?intro`：强制重播开场（默认同一个浏览器会话只播一次）
- `?intro&introAt=2.4`：开场定格在第 2.4 秒（仅开发模式）
