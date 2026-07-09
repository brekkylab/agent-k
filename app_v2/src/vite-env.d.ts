/// <reference types="vite/client" />

// Augment the built-in ImportMetaEnv with app-specific env vars.
interface ImportMetaEnv {
  readonly VITE_BACKEND_URL?: string;
  readonly VITE_LOCAL_USERNAME?: string;
  readonly VITE_LOCAL_PASSWORD?: string;
}
