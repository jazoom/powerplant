// @vitest-environment happy-dom
import { expect, test, vi } from "vitest";
import { initObserve } from "./observe";

const configurations = [
    {
        name: "job",
        target: "job-observe",
        unrelated: "transcript",
        action: "/",
    },
    {
        name: "environment preparation",
        target: "environment-preparation",
        unrelated: "environment-form",
        action: "/environments/x/configuration",
    },
] as const;

function observeRoot(target: string, active = true, action = "/"): HTMLElement {
    const root = document.createElement("div");
    root.dataset.observeTarget = target;
    if (active) {
        root.dataset.observeActive = "true";
    }
    root.innerHTML = `<form method="get" action="${action}"><button type="submit">Refresh</button></form>`;
    return root;
}

function countSubmissions(form: HTMLFormElement): { count: number } {
    const state = { count: 0 };
    form.requestSubmit = () => {
        state.count += 1;
    };
    return state;
}

for (const config of configurations) {
    test(`${config.name}: an active island submits after mount`, () => {
        vi.useFakeTimers();
        const root = observeRoot(config.target, true, config.action);
        const form = root.querySelector("form")!;
        const submissions = countSubmissions(form);
        const island = initObserve(root);
        expect(submissions.count).toBe(0);
        vi.advanceTimersByTime(0);
        expect(submissions.count).toBe(1);
        island.destroy();
        vi.useRealTimers();
    });

    test(`${config.name}: an idle island does not submit`, () => {
        vi.useFakeTimers();
        const root = observeRoot(config.target, false, config.action);
        const form = root.querySelector("form")!;
        const submissions = countSubmissions(form);
        const island = initObserve(root);
        vi.advanceTimersByTime(0);
        expect(submissions.count).toBe(0);
        island.reconcile?.({
            cause: "patch",
            detail: {
                requestKind: "patch",
                form,
                url: config.action,
                outcome: "applied-patch",
                status: 200,
                targetIds: [config.target],
            },
        });
        expect(submissions.count).toBe(0);
        island.destroy();
        vi.useRealTimers();
    });

    test(`${config.name}: a relevant applied patch continues immediately`, () => {
        vi.useFakeTimers();
        const root = observeRoot(config.target, true, config.action);
        const form = root.querySelector("form")!;
        const submissions = countSubmissions(form);
        const island = initObserve(root);
        vi.advanceTimersByTime(0);
        expect(submissions.count).toBe(1);
        island.reconcile?.({
            cause: "patch",
            detail: {
                requestKind: "patch",
                form,
                url: config.action,
                outcome: "applied-patch",
                status: 200,
                targetIds: [config.target],
            },
        });
        expect(submissions.count).toBe(2);
        island.destroy();
        vi.useRealTimers();
    });

    test(`${config.name}: an insertion patch starts one observation request`, () => {
        vi.useFakeTimers();
        const root = observeRoot(config.target, true, config.action);
        const form = root.querySelector("form")!;
        const submissions = countSubmissions(form);
        const island = initObserve(root);
        island.reconcile?.({
            cause: "patch",
            detail: {
                requestKind: "patch",
                form,
                url: config.action,
                outcome: "applied-patch",
                status: 200,
                targetIds: [config.target],
            },
        });
        expect(submissions.count).toBe(1);
        vi.advanceTimersByTime(0);
        expect(submissions.count).toBe(1);
        island.destroy();
        vi.useRealTimers();
    });

    test(`${config.name}: an unrelated patch does not start an observation request`, () => {
        vi.useFakeTimers();
        const root = observeRoot(config.target, true, config.action);
        const form = root.querySelector("form")!;
        const submissions = countSubmissions(form);
        const island = initObserve(root);
        vi.advanceTimersByTime(0);
        island.reconcile?.({
            cause: "patch",
            detail: {
                requestKind: "patch",
                form,
                url: config.action,
                outcome: "applied-patch",
                status: 200,
                targetIds: [config.unrelated],
            },
        });
        expect(submissions.count).toBe(1);
        island.destroy();
        vi.useRealTimers();
    });

    test(`${config.name}: a safe failure retries from the current form`, () => {
        vi.useFakeTimers();
        const root = observeRoot(config.target, true, config.action);
        const form = root.querySelector("form")!;
        const submissions = countSubmissions(form);
        const island = initObserve(root);
        vi.advanceTimersByTime(0);
        expect(submissions.count).toBe(1);
        island.reconcile?.({
            cause: "patch",
            detail: {
                requestKind: "patch",
                form,
                url: config.action,
                outcome: "safe-failure",
            },
        });
        expect(submissions.count).toBe(1);
        vi.advanceTimersByTime(1000);
        expect(submissions.count).toBe(2);
        island.destroy();
        vi.useRealTimers();
    });

    test(`${config.name}: teardown clears the retry timer`, () => {
        vi.useFakeTimers();
        const root = observeRoot(config.target, true, config.action);
        const form = root.querySelector("form")!;
        const submissions = countSubmissions(form);
        const island = initObserve(root);
        island.reconcile?.({
            cause: "patch",
            detail: {
                requestKind: "patch",
                form,
                url: config.action,
                outcome: "safe-failure",
            },
        });
        island.destroy();
        vi.advanceTimersByTime(1000);
        expect(submissions.count).toBe(0);
        vi.useRealTimers();
    });
}
