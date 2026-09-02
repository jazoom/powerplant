import type { IslandInstance } from "hypergraft/browser";

export const DEFAULT_THEME = "springfield-light";
export const THEME_STORAGE_KEY = "powerplant.theme";

export type Theme = "springfield-light" | "springfield-dark";

type ThemeStorage = Pick<Storage, "getItem" | "setItem">;

function isTheme(value: string | null | undefined): value is Theme {
    return value === "springfield-light" || value === "springfield-dark";
}

function browserStorage(): ThemeStorage | null {
    try {
        return window.localStorage;
    } catch {
        return null;
    }
}

export function readStoredTheme(
    storage: ThemeStorage | null = browserStorage(),
): Theme {
    if (storage === null) {
        return DEFAULT_THEME;
    }
    try {
        const value = storage.getItem(THEME_STORAGE_KEY);
        return isTheme(value) ? value : DEFAULT_THEME;
    } catch {
        return DEFAULT_THEME;
    }
}

export function applyStoredTheme(
    root: HTMLElement = document.documentElement,
    storage: ThemeStorage | null = browserStorage(),
): Theme {
    const theme = readStoredTheme(storage);
    root.dataset.theme = theme;
    return theme;
}

function storeTheme(theme: Theme, storage: ThemeStorage | null): void {
    if (storage === null) {
        return;
    }
    try {
        storage.setItem(THEME_STORAGE_KEY, theme);
    } catch {
        // The selected theme remains active when browser storage is unavailable.
    }
}

export function initThemeSelector(root: HTMLElement): IslandInstance {
    const select = root.querySelector<HTMLSelectElement>("[data-theme-select]");
    if (select === null) {
        return { destroy() {} };
    }

    const page = document.documentElement;
    const storage = browserStorage();
    const activeTheme = isTheme(page.dataset.theme)
        ? page.dataset.theme
        : DEFAULT_THEME;
    select.value = activeTheme;

    const onChange = () => {
        if (!isTheme(select.value)) {
            select.value = isTheme(page.dataset.theme)
                ? page.dataset.theme
                : DEFAULT_THEME;
            return;
        }
        page.dataset.theme = select.value;
        storeTheme(select.value, storage);
    };

    select.addEventListener("change", onChange);

    return {
        destroy() {
            select.removeEventListener("change", onChange);
        },
    };
}
