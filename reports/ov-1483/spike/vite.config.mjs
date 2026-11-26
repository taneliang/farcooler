import { defineConfig } from 'vite'

export default defineConfig({
  esbuild: { jsx: 'automatic' },
  build: {
    assetsInlineLimit: 0,
    modulePreload: { polyfill: false },
    rollupOptions: {
      output: {
        hashCharacters: 'hex',
        entryFileNames: 'a/[name].[hash:16].js',
        chunkFileNames: 'a/[name].[hash:16].js',
        assetFileNames: 'a/[name].[hash:16][extname]',
      },
    },
  },
})
