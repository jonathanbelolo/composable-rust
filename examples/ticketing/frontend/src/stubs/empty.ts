/**
 * Stubs for Node.js modules in browser builds.
 * These modules are used by @composable-svelte/core/dist/ssr/ssg.js during SSR/SSG
 * but should not execute in the browser. We provide no-op implementations.
 */

// fs/promises stubs
export const mkdir = async () => {};
export const writeFile = async () => {};
export const readFile = async () => '';
export const readdir = async () => [];
export const stat = async () => ({});
export const unlink = async () => {};
export const rmdir = async () => {};
export const access = async () => {};
export const copyFile = async () => {};
export const rename = async () => {};

// path stubs
export const join = (...args: string[]) => args.join('/');
export const dirname = (p: string) => p.split('/').slice(0, -1).join('/');
export const basename = (p: string) => p.split('/').pop() || '';
export const extname = (p: string) => {
  const base = p.split('/').pop() || '';
  const idx = base.lastIndexOf('.');
  return idx > 0 ? base.slice(idx) : '';
};
export const resolve = (...args: string[]) => args.join('/');
export const relative = (from: string, to: string) => to;
export const normalize = (p: string) => p;
export const isAbsolute = (p: string) => p.startsWith('/');
export const sep = '/';
export const delimiter = ':';

// url stubs
export const fileURLToPath = (url: string) => url.replace('file://', '');
export const pathToFileURL = (path: string) => new URL(`file://${path}`);
export const URL = globalThis.URL;
export const URLSearchParams = globalThis.URLSearchParams;

// fs stubs (sync versions)
export const existsSync = () => false;
export const mkdirSync = () => {};
export const writeFileSync = () => {};
export const readFileSync = () => '';
export const readdirSync = () => [];
export const statSync = () => ({});

export default {};
