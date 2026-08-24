// @vitest-environment happy-dom
import { expect, test } from "vitest";
import { initConnectErrors } from "./connect-errors";

function connectRoot(withSummary: boolean): HTMLElement {
    const root = document.createElement("section");
    root.id = "connect-card";
    root.innerHTML = withSummary
        ? `<div id="connect-errors" tabindex="-1"></div><form></form>`
        : `<form></form>`;
    document.body.append(root);
    return root;
}

test("mount focuses an existing error summary", () => {
    const root = connectRoot(true);
    const island = initConnectErrors(root);
    expect(document.activeElement?.id).toBe("connect-errors");
    island.destroy();
    root.remove();
});

test("an applied rejection focuses the error summary", () => {
    const root = connectRoot(true);
    const form = root.querySelector("form")!;
    const island = initConnectErrors(root);
    document.body.focus();
    island.reconcile?.({
        cause: "patch",
        detail: {
            requestKind: "patch",
            form,
            url: "/connect",
            outcome: "applied-patch",
            status: 422,
            targetIds: ["connect-card"],
        },
    });
    expect(document.activeElement?.id).toBe("connect-errors");
    island.destroy();
    root.remove();
});

test("a successful patch does not move focus", () => {
    const root = connectRoot(false);
    const form = root.querySelector("form")!;
    const island = initConnectErrors(root);
    const previous = document.activeElement;
    island.reconcile?.({
        cause: "patch",
        detail: {
            requestKind: "patch",
            form,
            url: "/connect",
            outcome: "applied-patch",
            status: 200,
            targetIds: ["connect-card"],
        },
    });
    expect(document.activeElement).toBe(previous);
    island.destroy();
    root.remove();
});
