import { useEffect } from 'react';
import { create } from 'zustand';

interface ToastState {
  message: string | null;
  detail: string[];
  show: (msg: string, detail?: string[]) => void;
  clear: () => void;
}

export const useToastStore = create<ToastState>((set) => ({
  message: null,
  detail: [],
  show: (msg, detail) => set({ message: msg, detail: detail ?? [] }),
  clear: () => set({ message: null, detail: [] }),
}));

export function ToastHost() {
  const message = useToastStore((s) => s.message);
  const detail = useToastStore((s) => s.detail);
  const clear = useToastStore((s) => s.clear);
  useEffect(() => {
    if (!message) return;
    // Longer duration when there's detail to read.
    const t = setTimeout(clear, detail.length > 0 ? 6500 : 3500);
    return () => clearTimeout(t);
  }, [message, detail, clear]);
  if (!message) return null;
  return (
    <div className="cw-toast" role="status">
      <div className="cw-toast-message">{message}</div>
      {detail.length > 0 && (
        <ul className="cw-toast-detail">
          {detail.map((line, i) => <li key={i}>{line}</li>)}
        </ul>
      )}
    </div>
  );
}
