import { createRootRoute, Outlet } from '@tanstack/react-router';
import { ensureBootstrap } from '@/lib/bootstrap';
import { AppShell } from '@/components/layout/AppShell';

export const Route = createRootRoute({
  beforeLoad: () => ensureBootstrap(),
  component: RootLayout,
});

function RootLayout() {
  return (
    <AppShell>
      <Outlet />
    </AppShell>
  );
}
