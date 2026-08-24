/**
 * @see https://prettier.io/docs/en/configuration.html
 * @type {import("prettier").Config}
 */
const config = {
    plugins: [
        "prettier-plugin-jinja-template",
        "prettier-plugin-tailwindcss",
        "prettier-plugin-organize-imports",
    ],
    organizeImportsSkipDestructiveCodeActions: true,
    trailingComma: "all",
    useTabs: false,
    tabWidth: 4,
    semi: true,
    singleQuote: false,
    printWidth: 80,
    overrides: [
        {
            files: "app/src/**/*.html",
            options: {
                parser: "jinja-template",
                tabWidth: 2,
            },
        },
    ],
};

export default config;
