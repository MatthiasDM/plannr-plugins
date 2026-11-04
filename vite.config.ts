// One plugin's browser half: PLUGIN=<id> npx vite build → <id>/dist/web.js, a self-contained ES module.
// It imports only types from Plannr (erased here); at run time it reaches the app through window.plannr.
import { defineConfig } from 'vite';

const id = process.env.PLUGIN;
if (!id || !/^[a-z0-9-]+$/.test(id)) throw new Error('set PLUGIN=<id>');

export default defineConfig({
  build: {
    outDir: `${id}/dist`, emptyOutDir: true, target: 'es2022', copyPublicDir: false,
    lib: { entry: `${id}/web/index.ts`, formats: ['es'], fileName: () => 'web.js' },
    rollupOptions: { external: ['three'] },
  },
});
