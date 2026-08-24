/// <reference types="vite/client" />

declare module "*.vue" {
  import type { DefineComponent } from "vue";
  const component: DefineComponent<{}, {}, any>;
  export default component;
}

declare module "vue" {
  interface ComponentCustomProperties {
    /** Global translation helper (injected in main.ts). */
    $t: (key: import("./i18n").TranslationKey, params?: Record<string, string | number>) => string;
  }
}

export {};
