/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        canvas: "var(--bg-canvas)",
        surface: "var(--bg-surface)",
        elevated: "var(--bg-elevated)",
        subtle: "var(--bg-subtle)",
        border: {
          subtle: "var(--border-subtle)",
          muted: "var(--border-muted)",
          active: "var(--border-active)",
        },
        keeper: {
          bg: "var(--color-keeper-bg)",
          border: "var(--color-keeper-border)",
          text: "var(--color-keeper-text)",
        },
        safe: {
          bg: "var(--color-safe-bg)",
          border: "var(--color-safe-border)",
          text: "var(--color-safe-text)",
        },
        warning: {
          bg: "var(--color-warning-bg)",
          border: "var(--color-warning-border)",
          text: "var(--color-warning-text)",
        },
        danger: {
          bg: "var(--color-danger-bg)",
          border: "var(--color-danger-border)",
          text: "var(--color-danger-text)",
        },
        alias: {
          bg: "var(--color-alias-bg)",
          border: "var(--color-alias-border)",
          text: "var(--color-alias-text)",
        },
      },
      fontFamily: {
        sans: [
          "-apple-system",
          "BlinkMacSystemFont",
          "Segoe UI",
          "Inter",
          "Roboto",
          "sans-serif",
        ],
        mono: [
          "JetBrains Mono",
          "SF Mono",
          "Menlo",
          "Consolas",
          "monospace",
        ],
      },
    },
  },
  plugins: [],
}

