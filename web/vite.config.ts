import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig, loadEnv } from 'vite';

type Emitter = { on(event: string, fn: (_res: unknown, _req: unknown, res: { flushHeaders(): void }) => void): void };

const DESIGN_PAGE = '/src/routes/design/+page.svelte';

// The /design page is a dev-only catalog; its fixtures must not ship in the binary, so a build sees an empty page there.
const devOnlyDesignPage = {
  name: 'dev-only-design-page',
  apply: 'build' as const,
  enforce: 'pre' as const,
  load: (id: string) => (id.endsWith(DESIGN_PAGE) ? '' : null),
};

export default defineConfig(({ mode }) => {
  // The same variable the daemon reads for its bind address, so a scratch daemon and the dev server agree.
  const daemon = loadEnv(mode, '.', '').BRR_SERVER__BIND || '127.0.0.1:9898';

  return {
    plugins: [devOnlyDesignPage, sveltekit()],
    server: {
      proxy: {
        '/v1': {
          target: `http://${daemon}`,
          ws: true,
          // http-proxy holds response headers until the first body byte; SSE streams send none for a while.
          configure: (proxy) =>
            (proxy as unknown as Emitter).on('proxyRes', (_res, _req, res) => setTimeout(() => res.flushHeaders())),
        },
        '/openapi.json': `http://${daemon}`,
      },
    },
  };
});
