// @vitest-environment happy-dom
import { afterEach, expect, test, vi } from "vitest";
import { initWorkspace } from "./workspace";

afterEach(() => {
    document.body.replaceChildren();
    vi.unstubAllGlobals();
});

test.each(["plan", "workflow"])(
    "a %s companion excludes mobile conversation controls and releases them after a command patch",
    (kind) => {
        vi.stubGlobal("matchMedia", () => ({
            matches: true,
            addEventListener() {},
        }));
        const root = document.createElement("div");
        root.innerHTML = `<section id="conversation-detail"><a data-plans-toggle href="/conversations/example?plans=true">Plans</a><a data-workflow-toggle href="/conversations/example/workflow">Run a workflow</a><section id="transcript"></section><section id="conversation-composer-dock"></section><aside id="conversation-work" data-work-active="true"><section id="${kind}-detail"></section><button data-work-close>Close</button></aside></section>`;
        document.body.append(root);
        const controller = new AbortController();
        const island = initWorkspace(root, {
            signal: controller.signal,
        } as Parameters<typeof initWorkspace>[1]);
        const transcript = root.querySelector<HTMLElement>("#transcript")!;
        expect(transcript.inert).toBe(true);
        root.querySelector<HTMLButtonElement>("[data-work-close]")!.click();
        expect(transcript.inert).toBe(false);
        expect(document.activeElement).toBe(
            root.querySelector(
                kind === "plan"
                    ? "[data-plans-toggle]"
                    : "[data-workflow-toggle]",
            ),
        );
        root.querySelector(`#${kind}-detail`)!.remove();
        root.querySelector<HTMLElement>(
            "#conversation-work",
        )!.dataset.workActive = "false";
        island.reconcile?.(
            {} as Parameters<NonNullable<typeof island.reconcile>>[0],
        );
        expect(transcript.inert).toBe(false);
        island.destroy?.();
        controller.abort();
    },
);

test.each(["plan", "workflow"])(
    "explicit %s navigation reopens a closed retained companion, unlike patches",
    (kind) => {
        vi.stubGlobal("matchMedia", () => ({
            matches: true,
            addEventListener() {},
        }));
        const root = document.createElement("div");
        root.innerHTML = `<section id="conversation-detail"><section id="transcript"></section><aside id="conversation-work" data-work-active="true"><section id="plan-detail"></section><button data-work-close>Close</button></aside></section>`;
        document.body.append(root);
        const controller = new AbortController();
        const island = initWorkspace(root, { signal: controller.signal });
        const work = root.querySelector<HTMLElement>("#conversation-work")!;
        root.querySelector<HTMLButtonElement>("[data-work-close]")!.click();
        const form = document.createElement("form");
        island.reconcile?.({
            cause: "live-patch",
            detail: { form, url: "/live", targetIds: ["conversation-detail"] },
        });
        island.reconcile?.({
            cause: "patch",
            detail: {
                form,
                url: "/command",
                requestKind: "patch",
                outcome: "applied-patch",
                status: 200,
                targetIds: ["conversation-detail"],
            },
        });
        island.reconcile?.({
            cause: "location",
            detail: { url: "/plans/one", cause: "command-patch-replacement" },
        });
        island.reconcile?.({
            cause: "location",
            detail: {
                url: "/conversations/one?title=true",
                cause: "get-form-replacement",
            },
        });
        expect(work.hidden).toBe(true);
        work.querySelector("section")!.id = `${kind}-detail`;
        island.reconcile?.({
            cause: "location",
            detail: {
                url:
                    kind === "plan"
                        ? "/plans/two"
                        : "/conversations/one/workflow",
                cause: "link-navigation",
            },
        });
        expect(work.hidden).toBe(false);
        expect(root.querySelector<HTMLElement>("#transcript")!.inert).toBe(
            true,
        );
        island.destroy();
        controller.abort();
    },
);

test("Escape closes navigation before the companion and restores the menu trigger", () => {
    vi.stubGlobal("matchMedia", () => ({
        matches: true,
        addEventListener() {},
    }));
    const root = document.createElement("div");
    root.innerHTML = `<button data-workspace-menu aria-expanded="false">Menu</button><aside id="workspace-index"><a href="/resources">Resources</a></aside><section id="conversation-detail"><section id="transcript"></section><aside id="conversation-work" data-work-active="true"></aside></section>`;
    document.body.append(root);
    const controller = new AbortController();
    const island = initWorkspace(root, {
        signal: controller.signal,
    } as Parameters<typeof initWorkspace>[1]);
    const menu = root.querySelector<HTMLButtonElement>(
        "[data-workspace-menu]",
    )!;
    menu.click();
    root.querySelector<HTMLAnchorElement>("a")!.focus();
    root.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    );
    expect(menu.getAttribute("aria-expanded")).toBe("false");
    expect(document.activeElement).toBe(menu);
    expect(root.querySelector<HTMLElement>("#transcript")!.inert).toBe(true);
    expect(root.querySelector<HTMLElement>("#conversation-work")!.hidden).toBe(
        false,
    );
    island.destroy?.();
    controller.abort();
});

test("diff colours preserve untrusted text without HTML interpretation", () => {
    vi.stubGlobal("matchMedia", () => ({
        matches: false,
        addEventListener() {},
    }));
    const root = document.createElement("div");
    const pre = document.createElement("pre");
    pre.className = "workspace-diff";
    const code = document.createElement("code");
    const text = "-<script>alert(1)</script>\n+<img src=x onerror=alert(1)>\n";
    code.textContent = text;
    pre.append(code);
    root.append(pre);
    document.body.append(root);
    const controller = new AbortController();
    const island = initWorkspace(root, {
        signal: controller.signal,
    } as Parameters<typeof initWorkspace>[1]);
    expect(code.textContent).toBe(text);
    expect(root.querySelector("script, img")).toBeNull();
    island.destroy?.();
    controller.abort();
});
