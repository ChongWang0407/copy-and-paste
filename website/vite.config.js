import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// base: './' 让构建产物用相对路径，可直接部署到 GitHub Pages 子路径或任意静态托管
export default defineConfig({
  base: './',
  plugins: [react()],
})
