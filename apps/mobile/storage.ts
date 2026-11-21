// Keep native credentials usable when WebView storage is unavailable.
export const storage = {
  getItem(key: string): string | null {
    try { return localStorage.getItem(key); } catch { return null; }
  },
  setItem(key: string, value: string) {
    try { localStorage.setItem(key, value); } catch { /* session-only preference */ }
  },
  removeItem(key: string) {
    try { localStorage.removeItem(key); } catch { /* storage may be unavailable */ }
  },
};
