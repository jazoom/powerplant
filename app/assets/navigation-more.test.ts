// @vitest-environment happy-dom
import { afterEach, expect, test } from "vitest";
import { initNavigationMore } from "./navigation-more";

afterEach(() => {
    document.body.replaceChildren();
});

test("a destination closes the mobile navigation disclosure", () => {
    document.body.innerHTML = `
        <details open>
            <summary>More</summary>
            <a href="/agents"><span>Agents</span></a>
        </details>
    `;
    const root = document.querySelector("details");
    if (!(root instanceof HTMLDetailsElement)) {
        throw new Error("details fixture is missing");
    }
    initNavigationMore(root, { signal: new AbortController().signal });

    root.querySelector("span")?.click();

    expect(root.open).toBe(false);
});
