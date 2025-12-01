import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { resolve } from 'path';

export default defineConfig(({ isSsrBuild }) => {
  if (isSsrBuild) {
    // Server build configuration
    return {
      plugins: [svelte()],
      build: {
        ssr: true,
        outDir: 'dist/server',
        rollupOptions: {
          input: {
            index: 'src/server/index.ts'
          },
          output: {
            format: 'esm',
            entryFileNames: '[name].js'
          }
        },
        target: 'node18',
        minify: false
      },
      resolve: {
        alias: {
          '$lib': resolve(__dirname, 'src/shared')
        }
      },
      ssr: {
        noExternal: ['@composable-svelte/core']
      }
    };
  }

  // Client build configuration
  return {
    plugins: [svelte()],
    build: {
      outDir: 'dist/client',
      rollupOptions: {
        input: 'src/client/index.ts',
        output: {
          format: 'esm',
          entryFileNames: '[name].js',
          assetFileNames: '[name].[ext]'
        }
      },
      target: 'es2022',
      minify: true
    },
    resolve: {
      alias: {
        '$lib': resolve(__dirname, 'src/shared'),
        // Stub out Node.js modules for browser (used by ssg.js)
        'fs/promises': resolve(__dirname, 'src/stubs/empty.ts'),
        'node:fs/promises': resolve(__dirname, 'src/stubs/empty.ts'),
        'fs': resolve(__dirname, 'src/stubs/empty.ts'),
        'node:fs': resolve(__dirname, 'src/stubs/empty.ts'),
        'path': resolve(__dirname, 'src/stubs/empty.ts'),
        'node:path': resolve(__dirname, 'src/stubs/empty.ts'),
        'url': resolve(__dirname, 'src/stubs/empty.ts'),
        'node:url': resolve(__dirname, 'src/stubs/empty.ts')
      }
    },
  };
});
