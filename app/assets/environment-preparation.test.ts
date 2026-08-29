// @vitest-environment happy-dom
import { expect, test, vi } from "vitest";
import { initEnvironmentPreparation } from "./environment-preparation";

function preparationRoot(active = true): HTMLElement {
    const root = document.createElement("div");
    if (active) {
        root.dataset.preparationActive = "true";
    }
    root.innerHTML = `<form method="get" action="/environments/x/configuration"><button type="submit">Refresh</button></form>`;
    return root;
}

test("an active preparation island submits the observation form after mount", () => {
    vi.useFakeTimers();
    const root = preparationRoot();
    const form = root.querySelector("form")!;
    let submitted = false;
    form.requestSubmit = () => {
        submitted = true;
    };
    const island = initEnvironmentPreparation(root);
    expect(submitted).toBe(false);
    vi.advanceTimersByTime(0);
    expect(submitted).toBe(true);
    island.destroy();
    vi.useRealTimers();
});

test("an idle preparation island does not submit", () => {
    const root = preparationRoot(false);
    const form = root.querySelector("form")!;
    let submitted = false;
    form.requestSubmit = () => {
        submitted = true;
    };
    initEnvironmentPreparation(root);
    expect(submitted).toBe(false);
});

test("an applied patch continues while preparation is active", () => {
    vi.useFakeTimers();
    const root = preparationRoot();
    const form = root.querySelector("form")!;
    let submissions = 0;
    form.requestSubmit = () => {
        submissions += 1;
    };
    const island = initEnvironmentPreparation(root);
    vi.advanceTimersByTime(0);
    expect(submissions).toBe(1);
    island.reconcile?.({
        cause: "patch",
        detail: {
            requestKind: "patch",
            form,
            url: "/",
            outcome: "applied-patch",
            status: 200,
            targetIds: ["environment-preparation"],
        },
    });
    expect(submissions).toBe(2);
    island.destroy();
    vi.useRealTimers();
});

test("teardown clears the retry timer", () => {
    vi.useFakeTimers();
    const root = preparationRoot();
    const form = root.querySelector("form")!;
    let submissions = 0;
    form.requestSubmit = () => {
        submissions += 1;
    };
    const island = initEnvironmentPreparation(root);
    island.reconcile?.({
        cause: "patch",
        detail: {
            requestKind: "patch",
            form,
            url: "/",
            outcome: "safe-failure",
        },
    });
    island.destroy();
    vi.advanceTimersByTime(1000);
    expect(submissions).toBe(0);
    vi.useRealTimers();
});
