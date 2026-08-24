/// <reference types="vitest/config" />
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";

export default defineConfig(({ mode }) => {
    const suffix = mode === "development" ? "" : "-[hash]";

    return {
        base: "/static/",
        plugins: [tailwindcss()],
        publicDir: "public",
        build: {
            outDir: "static",
            emptyOutDir: true,
            manifest: true,
            rolldownOptions: {
                input: "assets/main.ts",
                output: {
                    entryFileNames: `assets/[name]${suffix}.js`,
                    chunkFileNames: `assets/[name]${suffix}.js`,
                    assetFileNames: `assets/[name]${suffix}[extname]`,
                },
            },
        },
        test: {
            include: ["app/assets/**/*.test.ts"],
        },
    };
});
