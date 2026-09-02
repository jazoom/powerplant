// @vitest-environment happy-dom
import { beforeEach, expect, test } from "vitest";
import {
    DEFAULT_THEME,
    THEME_STORAGE_KEY,
    applyStoredTheme,
    initThemeSelector,
    readStoredTheme,
} from "./theme";

beforeEach(() => {
    window.localStorage.clear();
    document.documentElement.dataset.theme = DEFAULT_THEME;
    document.body.replaceChildren();
});

test("an unknown stored theme falls back to Springfield light", () => {
    window.localStorage.setItem(THEME_STORAGE_KEY, "unknown");

    expect(readStoredTheme()).toBe("springfield-light");
    expect(applyStoredTheme()).toBe("springfield-light");
    expect(document.documentElement.dataset.theme).toBe("springfield-light");
});

test("a stored dark theme applies to the document", () => {
    window.localStorage.setItem(THEME_STORAGE_KEY, "springfield-dark");

    expect(applyStoredTheme()).toBe("springfield-dark");
    expect(document.documentElement.dataset.theme).toBe("springfield-dark");
});

test("the selector applies and stores a valid theme", () => {
    document.documentElement.dataset.theme = "springfield-dark";
    const root = document.createElement("div");
    root.innerHTML = `
        <select data-theme-select>
            <option value="springfield-light">Springfield light</option>
            <option value="springfield-dark">Springfield dark</option>
        </select>
    `;
    document.body.append(root);

    const instance = initThemeSelector(root);
    const select = root.querySelector<HTMLSelectElement>("select")!;
    expect(select.value).toBe("springfield-dark");

    select.value = "springfield-light";
    select.dispatchEvent(new Event("change"));
    expect(document.documentElement.dataset.theme).toBe("springfield-light");
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe(
        "springfield-light",
    );

    instance.destroy();
});
