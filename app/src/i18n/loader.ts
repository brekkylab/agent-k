import i18n from './';

// Use in TanStack Router `loader` to await i18next namespace chunks before
// the route component mounts. Parent and child loaders run in parallel, so
// only list ns that this route (or its components) directly consumes — ns
// already covered by a parent loader resolves instantly here.
export function loadNs(...ns: string[]): Promise<unknown> {
  return i18n.loadNamespaces(ns);
}
