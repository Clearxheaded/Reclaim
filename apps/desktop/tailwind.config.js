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
        canvas: "#0c0d12",
        surface: "#14161f",
        elevated: "#1b1e2b",
        subtle: "#222638",
        border: {
          subtle: "rgba(255, 255, 255, 0.06)",
          muted: "rgba(255, 255, 255, 0.10)",
          active: "rgba(255, 255, 255, 0.20)",
        },
        keeper: {
          bg: "rgba(56, 189, 248, 0.10)",
          border: "rgba(56, 189, 248, 0.28)",
          text: "#38bdf8",
        },
        safe: {
          bg: "rgba(16, 185, 129, 0.10)",
          border: "rgba(16, 185, 129, 0.28)",
          text: "#34d399",
        },
        warning: {
          bg: "rgba(245, 158, 11, 0.10)",
          border: "rgba(245, 158, 11, 0.28)",
          text: "#fbbf24",
        },
        danger: {
          bg: "rgba(244, 63, 94, 0.10)",
          border: "rgba(244, 63, 94, 0.28)",
          text: "#fb7185",
        },
        alias: {
          bg: "rgba(148, 163, 184, 0.08)",
          border: "rgba(148, 163, 184, 0.20)",
          text: "#94a3b8",
        },
      },
      fontFamily: {
        sans: [
          "-apple-system",
          "BlinkMacSystemFont",
          "SF Pro Text",
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


