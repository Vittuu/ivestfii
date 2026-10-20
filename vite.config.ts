import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: '127.0.0.1',
    // Cargo locks native artifacts on Windows. They are never frontend inputs.
    watch: { ignored: ['**/target/**', '**/crates/**', '**/src-tauri/**', '**/.local/**', '**/data/**', '**/build/**', '**/dist/**', '**/web-dist/**'] },
    // A local frontend server must not serve the legacy portfolio or databases.
    fs: { deny: ['.env', '.env.*', '*.{crt,pem}', '**/.git/**', '**/data/**', '**/.local/**', '**/*.sqlite*', '**/*.db*'] },
  },
  build: { outDir: 'web-dist', target: 'es2022' },
});
