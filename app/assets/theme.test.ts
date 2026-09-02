// @vitest-environment happy-dom
import { beforeEach, expect, test, vi } from "vitest";
import { DEFAULT_THEME, initThemeSelector } from "./theme";

beforeEach(() => {
    document.documentElement.dataset.theme = DEFAULT_THEME;
    document.body.replaceChildren();
});

function themeForm(theme: string): HTMLFormElement {
    const form = document.createElement("form");
    form.innerHTML = `
        <div data-active-theme="${theme}"></div>
        <select data-theme-select name="theme">
            <option value="springfield-light">Springfield light</option>
            <option value="springfield-dark">Springfield dark</option>
        </select>
    `;
    document.body.append(form);
    return form;
}

test("the selector uses the server-rendered theme", () => {
    const form = themeForm("springfield-dark");

    initThemeSelector(form);

    expect(document.documentElement.dataset.theme).toBe("springfield-dark");
    expect(form.querySelector<HTMLSelectElement>("select")!.value).toBe(
        "springfield-dark",
    );
});

test("an invalid server-rendered theme defaults to Springfield light", () => {
    const form = themeForm("unknown");

    initThemeSelector(form);

    expect(document.documentElement.dataset.theme).toBe("springfield-light");
    expect(form.querySelector<HTMLSelectElement>("select")!.value).toBe(
        "springfield-light",
    );
});

test("a change applies immediately and submits the preference", () => {
    const form = themeForm("springfield-light");
    const requestSubmit = vi
        .spyOn(form, "requestSubmit")
        .mockImplementation(() => {});
    initThemeSelector(form);
    const select = form.querySelector<HTMLSelectElement>("select")!;

    select.value = "springfield-dark";
    select.dispatchEvent(new Event("change", { bubbles: true }));

    expect(document.documentElement.dataset.theme).toBe("springfield-dark");
    expect(requestSubmit).toHaveBeenCalledOnce();
});

test("reconciliation restores the server-rendered selection", () => {
    const form = themeForm("springfield-light");
    const instance = initThemeSelector(form);
    document.documentElement.dataset.theme = "springfield-dark";
    form.innerHTML = themeForm("springfield-light").innerHTML;

    instance.reconcile?.({} as never);

    expect(document.documentElement.dataset.theme).toBe("springfield-light");
    expect(form.querySelector<HTMLSelectElement>("select")!.value).toBe(
        "springfield-light",
    );
});
