import js from "@eslint/js";
import globals from "globals";

export default [
  {
    ignores: ["dist/**", "web/pkg/**", "artifacts/**", "coverage/**"],
  },
  js.configs.recommended,
  {
    files: ["web/**/*.js"],
    languageOptions: {
      globals: {
        ...globals.browser,
        __AUTOMATION__: "readonly",
      },
    },
  },
  {
    files: ["*.js", "tests/**/*.js", "tests/**/*.mjs", "scripts/**/*.mjs"],
    languageOptions: {
      globals: {
        ...globals.node,
        ...globals.browser,
      },
    },
  },
];
