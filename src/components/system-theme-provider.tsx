import { useEffect } from "react";

const DARK_QUERY = "(prefers-color-scheme: dark)";

type SystemThemeProviderProps = {
  children: React.ReactNode;
};

export function SystemThemeProvider({ children }: SystemThemeProviderProps) {
  useEffect(() => {
    const media = window.matchMedia(DARK_QUERY);

    const applyTheme = () => {
      const root = document.documentElement;
      root.classList.toggle("dark", media.matches);
      root.classList.toggle("light", !media.matches);
    };

    applyTheme();
    media.addEventListener("change", applyTheme);

    return () => {
      media.removeEventListener("change", applyTheme);
    };
  }, []);

  return <>{children}</>;
}
