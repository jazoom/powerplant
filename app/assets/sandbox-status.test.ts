// @vitest-environment happy-dom
import { expect, test, vi } from "vitest";
import { initSandboxStatus } from "./sandbox-status";

function sandboxRoot(active = true): HTMLElement {
    const root = document.createElement("div");
    if (active) {
        root.dataset.sandboxActive = "true";
    }
    root.innerHTML = `<form method="get" action="/sandbox"><button type="submit">Refresh</button></form>`;
    return root;
}

test("an active sandbox island submits the observation form after mount", () => {
    vi.useFakeTimers();
    const root = sandboxRoot();
    const form = root.querySelector("form")!;
    let submitted = false;
    form.requestSubmit = () => {
        submitted = true;
    };
    const island = initSandboxStatus(root);
    expect(submitted).toBe(false);
    vi.advanceTimersByTime(0);
    expect(submitted).toBe(true);
    island.destroy();
    vi.useRealTimers();
});

test("an idle sandbox island does not submit", () => {
    const root = sandboxRoot(false);
    const form = root.querySelector("form")!;
    let submitted = false;
    form.requestSubmit = () => {
        submitted = true;
    };
    initSandboxStatus(root);
    expect(submitted).toBe(false);
});
