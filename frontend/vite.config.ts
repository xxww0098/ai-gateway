/// <reference types="vitest" />
import path from "path"
import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { manualChunks } from './src/shared/build/manualChunks'

const BACKEND = 'http://127.0.0.1:8888'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  test: {
    globals: true,
    environment: 'jsdom',
  },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  build: {
    rollupOptions: {
      output: {
        manualChunks,
      },
    },
  },
  server: {
    proxy: {
      // ── CPA Gateway API ──
      '/api': {
        target: BACKEND,
        changeOrigin: true,
      },
      // ── SDK 管理 API ──
      '/v0': {
        target: BACKEND,
        changeOrigin: true,
      },
      // ── SDK 代理路由 ──
      '/v1': {
        target: BACKEND,
        changeOrigin: true,
      },
      // ── SDK Gemini 路由 ──
      '/v1beta': {
        target: BACKEND,
        changeOrigin: true,
      },
      // ── 健康检查 ──
      '/healthz': {
        target: BACKEND,
        changeOrigin: true,
      },
      // ── SDK OAuth 浏览器回调 ──
      // 使用正则匹配所有 /{provider}/callback 路径，
      // 避免逐条列举每个 OAuth 提供商。
      '^/(anthropic|codex|google|gemini|iflow|antigravity|qwen|kimi)/callback': {
        target: BACKEND,
        changeOrigin: true,
      },
    }
  }
})
