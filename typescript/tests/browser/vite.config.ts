import { defineConfig } from 'vite';
import path from 'node:path';

export default defineConfig({
  resolve: {
    alias: {
      '@bearcove/telex-core': path.resolve(__dirname, '../../packages/telex-core/src'),
      '@bearcove/telex-ws': path.resolve(__dirname, '../../packages/telex-ws/src'),
      '@bearcove/telex-generated': path.resolve(__dirname, '../../generated'),
    },
  },
  build: {
    target: 'esnext',
  },
  server: {
    port: 3000,
  },
});
