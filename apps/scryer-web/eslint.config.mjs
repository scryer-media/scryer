import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";

export default tseslint.config(
  { ignores: ["node_modules/**", "dist/**"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    plugins: { "react-hooks": reactHooks },
    rules: {
      // Standard hooks rules
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "warn",

      // React Compiler rules — keep the ones that catch real bugs
      "react-hooks/refs": "error", // no ref.current mutation during render
      "react-hooks/purity": "error", // no Math.random/Date.now in useMemo

      // Allow _-prefixed variables as intentional unused (e.g. destructure-to-omit)
      "@typescript-eslint/no-unused-vars": [
        "error",
        {
          varsIgnorePattern: "^_",
          argsIgnorePattern: "^_",
          ignoreRestSiblings: true,
        },
      ],

      // React Compiler rules — too strict for this codebase
      "react-hooks/preserve-manual-memoization": "off",
      "react-hooks/set-state-in-effect": "off",
      "react-hooks/incompatible-library": "off",
    },
  },
  {
    // Facets a user hides must vanish everywhere at once, so components learn
    // which facets to offer from `useFacets()` (see AGENTS.md, "Facet
    // visibility"). A data-only list, such as a settings key map, disables the
    // rule on its line and says why.
    files: ["components/**/*.{ts,tsx}"],
    ignores: ["**/*.test.{ts,tsx}"],
    rules: {
      "no-restricted-syntax": [
        "error",
        {
          selector:
            "ArrayExpression > Literal[value=/^(MOVIE|SERIES|ANIME|movie|series|anime)$/] ~ Literal[value=/^(MOVIE|SERIES|ANIME|movie|series|anime)$/]",
          message:
            "Do not list facets by hand. Enumerate useFacets().facets or filter with useFacets().values so hidden facets are left out.",
        },
      ],
      "no-restricted-imports": [
        "error",
        {
          paths: [
            {
              name: "@/lib/facets/registry",
              importNames: ["FACET_REGISTRY", "FACET_IDS", "MEDIA_VIEW_IDS"],
              message:
                "Enumerate useFacets().facets so hidden facets are left out. Look up one facet with facetById or facetForView.",
            },
          ],
          patterns: [
            {
              group: ["**/lib/facets/registry", "**/lib/facets/registry.ts"],
              importNames: ["FACET_REGISTRY", "FACET_IDS", "MEDIA_VIEW_IDS"],
              message:
                "Enumerate useFacets().facets so hidden facets are left out. Look up one facet with facetById or facetForView.",
            },
          ],
        },
      ],
    },
  },
);
