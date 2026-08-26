// @vitest-environment happy-dom
import { expect, test, vi } from "vitest";
import { initJobObserve } from "./job-observe";

function jobRoot(active = true): HTMLElement {
    const root = document.createElement("div");
    if (active) {
        root.dataset.jobActive = "true";
    }
    root.innerHTML = `<form method="get" action="/"><button type="submit">Refresh</button></form>`;
    return root;
}

test("an active job island submits the observation form after mount", () => {
    vi.useFakeTimers();
    const root = jobRoot();
    const form = root.querySelector("form")!;
    let submitted = false;
    form.requestSubmit = () => {
        submitted = true;
    };
    const island = initJobObserve(root);
    expect(submitted).toBe(false);
    vi.advanceTimersByTime(0);
    expect(submitted).toBe(true);
    island.destroy();
    vi.useRealTimers();
});

test("an idle job island does not submit", () => {
    const root = jobRoot(false);
    const form = root.querySelector("form")!;
    let submitted = false;
    form.requestSubmit = () => {
        submitted = true;
    };
    initJobObserve(root);
    expect(submitted).toBe(false);
});

test("an insertion patch starts one observation request", () => {
    vi.useFakeTimers();
    const root = jobRoot();
    const form = root.querySelector("form")!;
    let submissions = 0;
    form.requestSubmit = () => {
        submissions += 1;
    };
    const island = initJobObserve(root);
    island.reconcile?.({
        cause: "patch",
        detail: {
            requestKind: "patch",
            form,
            url: "/",
            outcome: "applied-patch",
            status: 200,
            targetIds: ["job-observe"],
        },
    });
    expect(submissions).toBe(1);
    vi.advanceTimersByTime(0);
    expect(submissions).toBe(1);
    island.destroy();
    vi.useRealTimers();
});

test("an applied patch continues while the job is active", () => {
    vi.useFakeTimers();
    const root = jobRoot();
    const form = root.querySelector("form")!;
    let submissions = 0;
    form.requestSubmit = () => {
        submissions += 1;
    };
    const island = initJobObserve(root);
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
            targetIds: ["job-observe"],
        },
    });
    expect(submissions).toBe(2);
    island.destroy();
    vi.useRealTimers();
});

test("an unrelated patch does not start an observation request", () => {
    vi.useFakeTimers();
    const root = jobRoot();
    const form = root.querySelector("form")!;
    let submissions = 0;
    form.requestSubmit = () => {
        submissions += 1;
    };
    const island = initJobObserve(root);
    vi.advanceTimersByTime(0);
    island.reconcile?.({
        cause: "patch",
        detail: {
            requestKind: "patch",
            form,
            url: "/",
            outcome: "applied-patch",
            status: 200,
            targetIds: ["transcript"],
        },
    });
    expect(submissions).toBe(1);
    island.destroy();
    vi.useRealTimers();
});

test("a safe failure retries from the current form", () => {
    vi.useFakeTimers();
    const root = jobRoot();
    const form = root.querySelector("form")!;
    let submissions = 0;
    form.requestSubmit = () => {
        submissions += 1;
    };
    const island = initJobObserve(root);
    vi.advanceTimersByTime(0);
    expect(submissions).toBe(1);
    island.reconcile?.({
        cause: "patch",
        detail: {
            requestKind: "patch",
            form,
            url: "/",
            outcome: "safe-failure",
        },
    });
    expect(submissions).toBe(1);
    vi.advanceTimersByTime(1000);
    expect(submissions).toBe(2);
    island.destroy();
    vi.useRealTimers();
});
