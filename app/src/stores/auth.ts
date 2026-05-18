import { create } from 'zustand';
import type { User } from '@/domain/types';
import { getToken, setToken } from '@/api/client';

interface AuthState {
  currentUser: User | null;
  setCurrentUser: (user: User | null) => void;
  isAuthenticated: () => boolean;
  reset: () => void;
}

export const useAuthStore = create<AuthState>((set) => ({
  currentUser: null,
  setCurrentUser: (user) => set({ currentUser: user }),
  isAuthenticated: () => Boolean(getToken()),
  reset: () => {
    setToken(null);
    set({ currentUser: null });
  },
}));
