import i18n from './';

// Use in TanStack Router `loader` to await i18next namespace chunks before
// the route component mounts.
export function loadNs(...ns: string[]): Promise<unknown> {
  return i18n.loadNamespaces(ns);
}
