import { create } from 'zustand';
import i18n from '@/i18n';
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
  setCurrentUser: (user) => {
    if (user && i18n.language !== user.preferredLanguage) {
      void i18n.changeLanguage(user.preferredLanguage);
    }
    set({ currentUser: user });
  },
  isAuthenticated: () => Boolean(getToken()),
  reset: () => {
    setToken(null);
    set({ currentUser: null });
  },
}));
