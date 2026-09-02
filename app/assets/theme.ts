import type { IslandInstance } from "hypergraft/browser";

export const DEFAULT_THEME = "springfield";

export const THEMES = [
    "springfield",
    "evergreen-terrace",
    "leftorium",
    "stonecutters",
    "sector-7-g",
] as const;

export type Theme = (typeof THEMES)[number];

function isTheme(value: string | null | undefined): value is Theme {
    return THEMES.some((theme) => theme === value);
}

function activeTheme(root: HTMLElement): Theme {
    const marker = root.querySelector<HTMLElement>("[data-active-theme]");
    return isTheme(marker?.dataset.activeTheme)
        ? marker.dataset.activeTheme
        : DEFAULT_THEME;
}

export function initThemeSelector(root: HTMLElement): IslandInstance {
    if (!(root instanceof HTMLFormElement)) {
        return { destroy() {} };
    }

    const page = document.documentElement;
    const applyAuthoritativeTheme = () => {
        const theme = activeTheme(root);
        page.dataset.theme = theme;
        const select = root.querySelector<HTMLSelectElement>(
            "[data-theme-select]",
        );
        if (select !== null) {
            select.value = theme;
        }
    };
    applyAuthoritativeTheme();

    const onChange = (event: Event) => {
        const select = event.target;
        if (
            !(select instanceof HTMLSelectElement) ||
            !select.matches("[data-theme-select]")
        ) {
            return;
        }
        if (!isTheme(select.value)) {
            applyAuthoritativeTheme();
            return;
        }
        page.dataset.theme = select.value;
        root.requestSubmit();
    };

    root.addEventListener("change", onChange);

    return {
        reconcile() {
            applyAuthoritativeTheme();
        },
        destroy() {
            root.removeEventListener("change", onChange);
        },
    };
}
